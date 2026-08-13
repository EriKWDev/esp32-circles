#!/usr/bin/env python3
"""Verify checked-in immutable firmware artifacts without modifying them."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Sequence


SHA256 = re.compile(r"^[0-9a-f]{64}$")


class IntegrityError(RuntimeError):
    """The manifest or immutable artifact set is not safe to trust."""


@dataclass(frozen=True)
class Artifact:
    path: str
    size: int
    sha256: str


@dataclass(frozen=True)
class Manifest:
    root: str
    artifacts: tuple[Artifact, ...]


def repository_path(value: object, label: str) -> str:
    if not isinstance(value, str) or not value or value != value.strip() or "\\" in value:
        raise IntegrityError(f"{label} must be a non-empty repository-relative POSIX path")
    pure = PurePosixPath(value)
    if pure.is_absolute() or ".." in pure.parts or value.startswith("/"):
        raise IntegrityError(f"{label} must not escape the repository")
    return pure.as_posix()


def path_is_within(path: str, root: str) -> bool:
    return path == root or path.startswith(root.rstrip("/") + "/")


def load_manifest(path: Path) -> Manifest:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise IntegrityError("cannot read a valid firmware integrity manifest") from exc
    if not isinstance(data, dict) or set(data) != {"schema_version", "root", "artifacts"}:
        raise IntegrityError("firmware integrity manifest has an invalid schema")
    if type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise IntegrityError("firmware integrity manifest has an unsupported schema version")
    root = repository_path(data["root"], "manifest root")
    entries = data["artifacts"]
    if not isinstance(entries, list) or not entries:
        raise IntegrityError("firmware integrity manifest must contain artifacts")

    artifacts: list[Artifact] = []
    seen: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "size", "sha256"}:
            raise IntegrityError("firmware integrity manifest has a malformed artifact entry")
        artifact_path = repository_path(entry["path"], "artifact path")
        if not path_is_within(artifact_path, root) or PurePosixPath(artifact_path).suffix.lower() != ".bin":
            raise IntegrityError("manifest artifacts must be .bin files beneath the configured root")
        if artifact_path in seen:
            raise IntegrityError("firmware integrity manifest contains duplicate artifact paths")
        if isinstance(entry["size"], bool) or not isinstance(entry["size"], int) or entry["size"] < 0:
            raise IntegrityError("artifact size must be a non-negative integer")
        if not isinstance(entry["sha256"], str) or not SHA256.fullmatch(entry["sha256"]):
            raise IntegrityError("artifact SHA-256 must be a lowercase 64-character hexadecimal digest")
        seen.add(artifact_path)
        artifacts.append(Artifact(artifact_path, entry["size"], entry["sha256"]))
    return Manifest(root, tuple(artifacts))


def safe_directory(repository: Path, relative: str) -> Path:
    current = repository
    for part in PurePosixPath(relative).parts:
        current /= part
        if current.is_symlink():
            raise IntegrityError("configured firmware root must not contain symlinks")
        if not current.exists() or not current.is_dir():
            raise IntegrityError("configured firmware root is missing or is not a directory")
    return current


def safe_artifact(repository: Path, artifact: Artifact) -> Path:
    current = repository
    for part in PurePosixPath(artifact.path).parts:
        current /= part
        if current.is_symlink():
            raise IntegrityError(f"symlink is not permitted for immutable artifact: {artifact.path}")
    if not current.exists():
        raise IntegrityError(f"immutable artifact is missing: {artifact.path}")
    if not current.is_file():
        raise IntegrityError(f"immutable artifact is not a regular file: {artifact.path}")
    return current


def configured_bins(root: Path, repository: Path) -> set[str]:
    found: set[str] = set()
    for directory, dirnames, filenames in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        for name in dirnames:
            if (directory_path / name).is_symlink():
                raise IntegrityError("symlinks are not permitted beneath the configured firmware root")
        for name in filenames:
            candidate = directory_path / name
            relative = candidate.relative_to(repository).as_posix()
            if candidate.is_symlink():
                raise IntegrityError(f"symlink is not permitted for immutable artifact: {relative}")
            if candidate.suffix.lower() == ".bin":
                found.add(relative)
    return found


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_integrity(repository: Path, manifest_path: Path) -> int:
    if not repository.is_dir():
        raise IntegrityError("repository root is not a directory")
    repository = repository.resolve()
    manifest = load_manifest(manifest_path)
    configured_root = safe_directory(repository, manifest.root)
    expected = {artifact.path for artifact in manifest.artifacts}
    actual = configured_bins(configured_root, repository)
    missing = sorted(expected - actual)
    unexpected = sorted(actual - expected)
    if missing:
        raise IntegrityError("manifest artifact is missing from the configured root: " + missing[0])
    if unexpected:
        raise IntegrityError("unexpected .bin file beneath the configured root: " + unexpected[0])

    for artifact in manifest.artifacts:
        candidate = safe_artifact(repository, artifact)
        if candidate.stat().st_size != artifact.size:
            raise IntegrityError(f"immutable artifact size mismatch: {artifact.path}")
        if sha256(candidate) != artifact.sha256:
            raise IntegrityError(f"immutable artifact SHA-256 mismatch: {artifact.path}")
    return len(manifest.artifacts)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Verify immutable firmware artifact sizes and SHA-256 digests.")
    parser.add_argument("repo", type=Path, help="repository checkout to inspect")
    parser.add_argument("--manifest", type=Path, required=True, help="repository integrity manifest")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        checked = verify_integrity(args.repo, args.manifest)
    except IntegrityError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except Exception as exc:  # pragma: no cover - defensive CLI boundary
        print(f"internal error: {type(exc).__name__}", file=sys.stderr)
        return 2
    print(f"verified immutable firmware artifacts: {checked}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
