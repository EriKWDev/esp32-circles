#!/usr/bin/env python3
"""Package Arduino CLI outputs as a self-describing ESP32-C6 flash bundle."""

from __future__ import annotations

import argparse
import hashlib
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


def find_unique(directory: Path, pattern: str) -> Path:
    matches = sorted(path.resolve() for path in directory.glob(pattern) if path.is_file())
    if len(matches) != 1:
        raise SystemExit(
            f"Expected exactly one {pattern} in {directory}, found {len(matches)}"
        )
    source = matches[0]
    try:
        source.relative_to(directory)
    except ValueError as exc:
        raise SystemExit(f"Arduino output escapes the build directory: {source}") from exc
    return source


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
    parser.add_argument("--sketch", required=True)
    parser.add_argument("--build-dir", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--core-version", required=True)
    parser.add_argument("--output-dir", default="release-artifacts")
    args = parser.parse_args()

    repository = Path.cwd()
    sketch = resolve_repository_path(repository, args.sketch, "sketch")
    if not sketch.is_dir():
        raise SystemExit(f"Sketch directory does not exist: {sketch}")
    raw_build = Path(args.build_dir)
    build = (raw_build if raw_build.is_absolute() else repository / raw_build).resolve()
    if not build.is_dir():
        raise SystemExit(f"Build directory does not exist: {build}")
    raw_output = Path(args.output_dir)
    output = (raw_output if raw_output.is_absolute() else repository / raw_output).resolve()
    files = [
        ("0x0", find_unique(build, "*.bootloader.bin")),
        ("0x8000", find_unique(build, "*.partitions.bin")),
        ("0xe000", find_unique(build, "boot_app0.bin")),
        ("0x10000", find_unique(build, "*.ino.bin")),
    ]

    name = safe_component(args.name, "example name")
    core_version = safe_component(args.core_version, "core version")
    bundle_name = f"{name}-arduino-{core_version}"
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{bundle_name}.zip"
    with tempfile.TemporaryDirectory(prefix=f".{bundle_name}-", dir=output) as temporary:
        bundle = Path(temporary) / bundle_name
        binary_dir = bundle / "bin"
        binary_dir.mkdir(parents=True)
        destination_names: set[str] = set()
        packaged: list[dict[str, object]] = []
        for address, source in files:
            safe_component(source.name, "Arduino output filename")
            if source.name in destination_names:
                raise SystemExit(f"Multiple Arduino outputs share the same bundle name: {source.name}")
            destination_names.add(source.name)
            destination = binary_dir / source.name
            shutil.copy2(source, destination)
            bundled_path = destination.relative_to(bundle).as_posix()
            packaged.append(
                {
                    "address": address,
                    "file": bundled_path,
                    "archive_path": bundled_path,
                    "offset": address,
                    "size": destination.stat().st_size,
                    "sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
                }
            )

        manifest = {
            "schema_version": 1,
            "board": "ESP32-C6-Touch-AMOLED-2.16",
            "chip": "esp32c6",
            "name": name,
            "framework": "arduino-esp32",
            "framework_version": core_version,
            "target": "esp32c6",
            "sketch": sketch.relative_to(repository).as_posix(),
            "source_project": sketch.relative_to(repository).as_posix(),
            "git_sha": os.environ.get("PACKAGE_GIT_SHA") or os.environ.get("GITHUB_SHA", "unknown"),
            "generated_utc": datetime.now(timezone.utc).isoformat(),
            "flash": {"baud": 921600},
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
