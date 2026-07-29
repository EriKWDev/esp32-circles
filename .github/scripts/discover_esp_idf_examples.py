#!/usr/bin/env python3
"""Select ESP-IDF examples for CI from an explicit name or a Git diff."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path, PurePosixPath


REPOSITORY = Path(__file__).resolve().parents[2]
EXAMPLES_ROOT = PurePosixPath("examples/esp-idf")
GLOBAL_PREFIXES = (
    ".github/scripts/",
    ".github/workflows/esp-idf-examples.yml",
    "components/",
    "config/",
)


def examples() -> list[dict[str, str]]:
    root = REPOSITORY / EXAMPLES_ROOT
    result: list[dict[str, str]] = []
    for directory in sorted(root.iterdir()):
        if not directory.is_dir():
            continue
        if (directory / "CMakeLists.txt").is_file() and (directory / "main").is_dir():
            result.append({"name": directory.name, "example": directory.relative_to(REPOSITORY).as_posix()})
    return result


def event_base() -> str | None:
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if not event_path or not Path(event_path).is_file():
        return None
    event = json.loads(Path(event_path).read_text(encoding="utf-8"))
    if "pull_request" in event:
        return event["pull_request"]["base"]["sha"]
    before = event.get("before")
    if before and set(before) != {"0"}:
        return before
    return None


def changed_paths(base: str | None, head: str) -> list[str] | None:
    base = base or event_base()
    if not base:
        return None
    output = subprocess.check_output(
        ["git", "diff", "--name-only", f"{base}...{head}"],
        cwd=REPOSITORY,
        text=True,
    )
    return [PurePosixPath(line.strip()).as_posix() for line in output.splitlines() if line.strip()]


def select(selection: str, base: str | None, head: str) -> list[dict[str, str]]:
    available = examples()
    if selection == "all":
        return available
    if selection != "changed":
        wanted = PurePosixPath(selection).name
        chosen = [entry for entry in available if entry["name"] == wanted or entry["example"] == selection]
        if not chosen:
            raise SystemExit(f"Unknown ESP-IDF example: {selection}")
        return chosen

    paths = changed_paths(base, head)
    if paths is None:
        return available
    if any(path.startswith(GLOBAL_PREFIXES) for path in paths):
        return available
    selected_names = {
        PurePosixPath(path).parts[2]
        for path in paths
        if len(PurePosixPath(path).parts) > 2
        and PurePosixPath(path).parts[:2] == EXAMPLES_ROOT.parts
    }
    return [entry for entry in available if entry["name"] in selected_names]


def emit(selected: list[dict[str, str]]) -> None:
    matrix = json.dumps({"include": selected}, separators=(",", ":"))
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"matrix={matrix}\n")
            stream.write(f"has_examples={'true' if selected else 'false'}\n")
    print(json.dumps(selected, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--selection", default="changed", help="changed, all, a directory name, or a repo-relative path")
    parser.add_argument("--base", help="base revision for changed selection")
    parser.add_argument("--head", default="HEAD", help="head revision for changed selection")
    args = parser.parse_args()
    emit(select(args.selection, args.base, args.head))


if __name__ == "__main__":
    main()
