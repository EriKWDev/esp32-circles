#!/usr/bin/env python3
"""Package an ESP-IDF build as a self-describing flash bundle."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import shlex
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path


SAFE_COMPONENT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def safe_component(value: str, label: str) -> str:
    if not SAFE_COMPONENT.fullmatch(value):
        raise SystemExit(f"Invalid {label}; use letters, numbers, dots, dashes, and underscores")
    return value


def resolve_repository_path(repository: Path, value: str, label: str) -> Path:
    raw = Path(value)
    candidate = (raw if raw.is_absolute() else repository / raw).resolve()
    try:
        candidate.relative_to(repository)
    except ValueError as exc:
        raise SystemExit(f"{label} must stay inside the repository: {value}") from exc
    return candidate


def resolve_build_reference(build: Path, value: str) -> Path:
    raw = Path(value)
    if raw.is_absolute():
        raise SystemExit(f"flasher_args.json contains an absolute build path: {value}")
    candidate = (build / raw).resolve()
    try:
        candidate.relative_to(build)
    except ValueError as exc:
        raise SystemExit(f"flasher_args.json escapes the build directory: {value}") from exc
    if not candidate.is_file():
        raise SystemExit(f"Missing build output referenced by flasher_args.json: {candidate}")
    return candidate


def parse_address(value: object) -> tuple[int, str]:
    if not isinstance(value, str):
        raise SystemExit("flasher_args.json flash addresses must be strings")
    try:
        parsed = int(value, 0)
    except ValueError as exc:
        raise SystemExit(f"Invalid flash address in flasher_args.json: {value}") from exc
    if parsed < 0 or parsed > 0xFFFFFFFF:
        raise SystemExit(f"Flash address is outside the 32-bit range: {value}")
    return parsed, value


def rewrite_metadata_paths(value: object, replacements: dict[str, str]) -> object:
    """Replace build-relative file references throughout ESP-IDF metadata."""
    if isinstance(value, dict):
        return {key: rewrite_metadata_paths(item, replacements) for key, item in value.items()}
    if isinstance(value, list):
        return [rewrite_metadata_paths(item, replacements) for item in value]
    if isinstance(value, str):
        return replacements.get(value, value)
    return value


def add_portable_zip_file(zipped: zipfile.ZipFile, source: Path, archive_path: Path) -> None:
    info = zipfile.ZipInfo.from_file(source, archive_path.as_posix())
    info.create_system = 3
    mode = 0o100755 if source.name == "flash.sh" else 0o100644
    info.external_attr = mode << 16
    zipped.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)


def write_output(path: Path) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        value = path.as_posix()
        if "\n" in value or "\r" in value:
            raise SystemExit("Artifact path cannot contain a line break")
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"artifact_path={value}\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--build-dir", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--framework-version", required=True)
    parser.add_argument("--output-dir", default="release-artifacts")
    args = parser.parse_args()

    repository = Path.cwd()
    project = resolve_repository_path(repository, args.project, "project")
    if not project.is_dir():
        raise SystemExit(f"Project directory does not exist: {project}")
    raw_build = Path(args.build_dir)
    build = (raw_build if raw_build.is_absolute() else repository / raw_build).resolve()
    if not build.is_dir():
        raise SystemExit(f"Build directory does not exist: {build}")
    raw_output = Path(args.output_dir)
    output = (raw_output if raw_output.is_absolute() else repository / raw_output).resolve()
    flasher_args = build / "flasher_args.json"
    if not flasher_args.is_file():
        raise SystemExit(f"Missing ESP-IDF flash metadata: {flasher_args}")

    flash_metadata = json.loads(flasher_args.read_text(encoding="utf-8"))
    flash_files = flash_metadata.get("flash_files", {})
    if not isinstance(flash_files, dict) or not flash_files:
        raise SystemExit("flasher_args.json does not contain flash_files")

    name = safe_component(args.name, "example name")
    framework_version = safe_component(args.framework_version, "framework version")
    bundle_name = f"{name}-esp-idf-{framework_version}"
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{bundle_name}.zip"
    with tempfile.TemporaryDirectory(prefix=f".{bundle_name}-", dir=output) as temporary:
        bundle = Path(temporary) / bundle_name
        binary_dir = bundle / "bin"
        binary_dir.mkdir(parents=True)

        resolved: list[tuple[int, str, Path]] = []
        destination_names: set[str] = set()
        normalized_addresses: set[int] = set()
        for address, relative in flash_files.items():
            parsed_address, normalized_address = parse_address(address)
            if parsed_address in normalized_addresses:
                raise SystemExit(f"Duplicate normalized flash address: {normalized_address}")
            normalized_addresses.add(parsed_address)
            if not isinstance(relative, str):
                raise SystemExit("flasher_args.json build paths must be strings")
            source = resolve_build_reference(build, relative)
            safe_component(source.name, "ESP-IDF output filename")
            if source.name in destination_names:
                raise SystemExit(f"Multiple ESP-IDF outputs share the same bundle name: {source.name}")
            destination_names.add(source.name)
            resolved.append((parsed_address, normalized_address, source))

        packaged: list[dict[str, str]] = []
        metadata_replacements: dict[str, str] = {}
        for _, address, source in sorted(resolved, key=lambda item: item[0]):
            destination = binary_dir / source.name
            shutil.copy2(source, destination)
            bundled_path = destination.relative_to(bundle).as_posix()
            packaged.append({"address": address, "file": bundled_path})
            original_path = flash_files[address]
            metadata_replacements[original_path] = bundled_path

        portable_flash_metadata = rewrite_metadata_paths(flash_metadata, metadata_replacements)
        assert isinstance(portable_flash_metadata, dict)
        portable_flash_metadata["flash_files"] = {
            item["address"]: item["file"] for item in packaged
        }
        (bundle / "flasher_args.json").write_text(
            json.dumps(portable_flash_metadata, indent=2) + "\n", encoding="utf-8"
        )
        manifest = {
            "name": name,
            "framework": "esp-idf",
            "framework_version": framework_version,
            "target": "esp32c6",
            "project": project.relative_to(repository).as_posix(),
            "git_sha": os.environ.get("GITHUB_SHA", "unknown"),
            "generated_utc": datetime.now(timezone.utc).isoformat(),
            "files": packaged,
        }
        (bundle / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )

        posix_pairs = " ".join(
            f"{item['address']} {shlex.quote(item['file'])}" for item in packaged
        )
        windows_pairs = " ".join(
            f"{item['address']} \"{item['file'].replace('/', chr(92))}\"" for item in packaged
        )
        flash_script = bundle / "flash.sh"
        flash_script.write_text(
            "#!/usr/bin/env sh\nset -eu\ncd \"$(dirname \"$0\")\"\n"
            f"esptool.py --chip esp32c6 --baud 921600 write_flash {posix_pairs}\n",
            encoding="utf-8",
        )
        flash_script.chmod(0o755)
        (bundle / "flash.bat").write_text(
            "@echo off\r\nsetlocal\r\ncd /d \"%~dp0\"\r\n"
            "esptool.py --chip esp32c6 --baud 921600 write_flash "
            + windows_pairs
            + "\r\n",
            encoding="utf-8",
        )

        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
            for file in sorted(bundle.rglob("*")):
                if file.is_file():
                    add_portable_zip_file(
                        zipped, file, Path(bundle_name) / file.relative_to(bundle)
                    )
    write_output(archive)
    print(archive)


if __name__ == "__main__":
    main()
