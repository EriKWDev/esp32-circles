#!/usr/bin/env python3
"""Package an ESP-IDF build as a self-describing flash bundle."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path


def write_output(path: Path) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"artifact_path={path.as_posix()}\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--build-dir", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--framework-version", required=True)
    parser.add_argument("--output-dir", default="release-artifacts")
    args = parser.parse_args()

    repository = Path.cwd()
    project = (repository / args.project).resolve()
    build = (repository / args.build_dir).resolve()
    output = (repository / args.output_dir).resolve()
    flasher_args = build / "flasher_args.json"
    if not flasher_args.is_file():
        raise SystemExit(f"Missing ESP-IDF flash metadata: {flasher_args}")

    flash_metadata = json.loads(flasher_args.read_text(encoding="utf-8"))
    flash_files = flash_metadata.get("flash_files", {})
    if not flash_files:
        raise SystemExit("flasher_args.json does not contain flash_files")

    bundle_name = f"{args.name}-esp-idf-{args.framework_version}"
    bundle = output / bundle_name
    binary_dir = bundle / "bin"
    binary_dir.mkdir(parents=True, exist_ok=True)

    packaged: list[dict[str, str]] = []
    for address, relative in sorted(flash_files.items(), key=lambda item: int(item[0], 0)):
        source = build / relative
        if not source.is_file():
            raise SystemExit(f"Missing build output referenced by flasher_args.json: {source}")
        destination = binary_dir / source.name
        shutil.copy2(source, destination)
        packaged.append({"address": address, "file": destination.relative_to(bundle).as_posix()})

    shutil.copy2(flasher_args, bundle / "flasher_args.json")
    manifest = {
        "name": args.name,
        "framework": "esp-idf",
        "framework_version": args.framework_version,
        "target": "esp32c6",
        "project": project.relative_to(repository).as_posix(),
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
    write_output(archive)
    print(archive)


if __name__ == "__main__":
    main()
