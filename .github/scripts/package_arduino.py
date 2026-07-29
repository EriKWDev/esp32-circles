#!/usr/bin/env python3
"""Package Arduino CLI outputs as a self-describing ESP32-C6 flash bundle."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path


def find_one(directory: Path, pattern: str) -> Path | None:
    matches = sorted(directory.glob(pattern))
    return matches[0] if matches else None


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sketch", required=True)
    parser.add_argument("--build-dir", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--core-version", required=True)
    parser.add_argument("--output-dir", default="release-artifacts")
    args = parser.parse_args()

    repository = Path.cwd()
    build = (repository / args.build_dir).resolve()
    output = (repository / args.output_dir).resolve()
    files = [
        ("0x0", find_one(build, "*.bootloader.bin")),
        ("0x8000", find_one(build, "*.partitions.bin")),
        ("0xe000", find_one(build, "boot_app0.bin")),
        ("0x10000", find_one(build, "*.ino.bin")),
    ]
    if any(source is None for _, source in files):
        raise SystemExit(f"Incomplete Arduino output set in {build}")

    bundle_name = f"{args.name}-arduino-{args.core_version}"
    bundle = output / bundle_name
    binary_dir = bundle / "bin"
    binary_dir.mkdir(parents=True, exist_ok=True)
    packaged: list[dict[str, str]] = []
    for address, source in files:
        assert source is not None
        destination = binary_dir / source.name
        shutil.copy2(source, destination)
        packaged.append({"address": address, "file": destination.relative_to(bundle).as_posix()})

    manifest = {
        "name": args.name,
        "framework": "arduino-esp32",
        "framework_version": args.core_version,
        "target": "esp32c6",
        "sketch": Path(args.sketch).as_posix(),
        "git_sha": os.environ.get("GITHUB_SHA", "unknown"),
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "files": packaged,
    }
    (bundle / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    pairs = " ".join(f"{item['address']} {item['file']}" for item in packaged)
    flash_script = bundle / "flash.sh"
    flash_script.write_text(
        f'#!/usr/bin/env sh\nset -eu\ncd "$(dirname "$0")"\nesptool.py --chip esp32c6 --baud 921600 write_flash {pairs}\n',
        encoding="utf-8",
    )
    flash_script.chmod(0o755)
    (bundle / "flash.bat").write_text(
        "@echo off\r\nsetlocal\r\ncd /d \"%~dp0\"\r\nesptool.py --chip esp32c6 --baud 921600 write_flash " + pairs.replace("/", "\\") + "\r\n",
        encoding="utf-8",
    )

    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{bundle_name}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
        for file in sorted(bundle.rglob("*")):
            if file.is_file():
                zipped.write(file, Path(bundle_name) / file.relative_to(bundle))
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with Path(github_output).open("a", encoding="utf-8") as stream:
            stream.write(f"artifact_path={archive.as_posix()}\n")
    print(archive)


if __name__ == "__main__":
    main()
