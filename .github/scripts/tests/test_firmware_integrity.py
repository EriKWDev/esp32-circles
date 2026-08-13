from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[3]
AUDIT_ROOT = REPOSITORY / ".github" / "scripts" / "repository_audit"
CHECKER = AUDIT_ROOT / "check_firmware_integrity.py"
sys.path.insert(0, str(AUDIT_ROOT))

from check_firmware_integrity import IntegrityError, verify_integrity  # noqa: E402


class FirmwareIntegrityFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.first = "firmware/factory_firmware/01_Fac-v1.0.0.bin"
        self.second = "firmware/factory_firmware/02_xiaozhi-v2.2.5.bin"
        self.write_bytes(self.first, b"factory firmware fixture\n")
        self.write_bytes(self.second, b"xiaozhi firmware fixture\n")
        self.manifest = self.root / ".github/policy/firmware-integrity.json"
        self.write_manifest()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_bytes(self, relative: str, content: bytes) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    def write_manifest(self) -> None:
        artifacts = []
        for relative in (self.first, self.second):
            content = (self.root / relative).read_bytes()
            artifacts.append(
                {
                    "path": relative,
                    "size": len(content),
                    "sha256": hashlib.sha256(content).hexdigest(),
                }
            )
        self.manifest.parent.mkdir(parents=True, exist_ok=True)
        self.manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "root": "firmware/factory_firmware",
                    "artifacts": artifacts,
                }
            ),
            encoding="utf-8",
        )

    def manifest_with_schema_version(self, schema_version: object) -> str:
        manifest = json.loads(self.manifest.read_text(encoding="utf-8"))
        manifest["schema_version"] = schema_version
        return json.dumps(manifest)

    def test_synthetic_fixture_and_exact_cli_invocation_pass(self) -> None:
        self.assertEqual(verify_integrity(self.root, self.manifest), 2)
        result = subprocess.run(
            [sys.executable, str(CHECKER), str(self.root), "--manifest", str(self.manifest)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "verified immutable firmware artifacts: 2")

    def test_same_size_tampered_artifact_hash_mismatch_fails(self) -> None:
        replacement = b"factory firmware altered\n"
        self.assertEqual(len(replacement), (self.root / self.first).stat().st_size)
        self.write_bytes(self.first, replacement)
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)

    def test_artifact_size_mismatch_fails(self) -> None:
        self.write_bytes(self.first, b"short")
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)

    def test_missing_artifact_fails(self) -> None:
        (self.root / self.second).unlink()
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)

    def test_unexpected_bin_fails(self) -> None:
        self.write_bytes("firmware/factory_firmware/unexpected.bin", b"extra")
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)

    def test_malformed_duplicate_and_escaping_manifest_fail(self) -> None:
        for manifest in (
            "not json",
            json.dumps({"schema_version": 1, "root": "firmware/factory_firmware", "artifacts": []}),
            self.manifest_with_schema_version(True),
            self.manifest_with_schema_version(1.0),
            json.dumps(
                {
                    "schema_version": 1,
                    "root": "firmware/factory_firmware",
                    "artifacts": [
                        {"path": self.first, "size": 1, "sha256": "0" * 64},
                        {"path": self.first, "size": 1, "sha256": "0" * 64},
                    ],
                }
            ),
            json.dumps(
                {
                    "schema_version": 1,
                    "root": "firmware/factory_firmware",
                    "artifacts": [{"path": "../escape.bin", "size": 1, "sha256": "0" * 64}],
                }
            ),
        ):
            with self.subTest(manifest=manifest[:20]):
                self.manifest.write_text(manifest, encoding="utf-8")
                with self.assertRaises(IntegrityError):
                    verify_integrity(self.root, self.manifest)

    def test_symlink_and_non_file_artifacts_fail(self) -> None:
        target = self.root / self.first
        target.unlink()
        target.symlink_to(self.root / self.second)
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)
        target.unlink()
        target.mkdir()
        with self.assertRaises(IntegrityError):
            verify_integrity(self.root, self.manifest)


if __name__ == "__main__":
    unittest.main()
