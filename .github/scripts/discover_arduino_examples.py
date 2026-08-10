#!/usr/bin/env python3
"""Select first-party Arduino sketches for CI."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path, PurePosixPath

from ci_routing_adapter import RoutingSelectionError, routing_report


REPOSITORY = Path(__file__).resolve().parents[2]
EXAMPLES_ROOT = PurePosixPath("examples/arduino")


def examples() -> list[dict[str, str]]:
    root = REPOSITORY / EXAMPLES_ROOT
    result: list[dict[str, str]] = []
    for directory in sorted(root.iterdir()):
        if directory.is_dir() and any(directory.glob("*.ino")):
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


def select(selection: str, base: str | None, head: str) -> list[dict[str, str]]:
    available = examples()
    if selection == "all":
        return available
    if selection != "changed":
        wanted = PurePosixPath(selection).name
        chosen = [entry for entry in available if entry["name"] == wanted or entry["example"] == selection]
        if not chosen:
            raise SystemExit(f"Unknown Arduino example: {selection}")
        return chosen

    base = base or event_base()
    if not base:
        raise SystemExit(
            "Changed Arduino selection requires a complete base revision; "
            "use --base, a pull request event, or --selection all."
        )
    try:
        sketch_files = routing_report(base, head)["arduino"]["selected"]
    except RoutingSelectionError as exc:
        raise SystemExit(f"Arduino routing failed: {exc}") from exc
    selected_roots = {PurePosixPath(path).parent.as_posix() for path in sketch_files}
    return [entry for entry in available if entry["example"] in selected_roots]


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
