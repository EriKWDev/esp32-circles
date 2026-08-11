from __future__ import annotations

import json
import hashlib
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[3]
SCRIPTS = REPOSITORY / ".github" / "scripts"


class PackagingTest(unittest.TestCase):
    def setUp(self) -> None:
        self.repo_tmp = tempfile.TemporaryDirectory()
        self.build_tmp = tempfile.TemporaryDirectory()
        self.output_tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.repo_tmp.name)
        self.build = Path(self.build_tmp.name)
        self.output = Path(self.output_tmp.name)
        (self.repo / "example").mkdir()

    def tearDown(self) -> None:
        self.output_tmp.cleanup()
        self.build_tmp.cleanup()
        self.repo_tmp.cleanup()

    def run_script(self, script: str, *arguments: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPTS / script), *arguments],
            cwd=self.repo,
            env=env,
            check=False,
            capture_output=True,
            text=True,
        )

    def write_binary(self, relative: str, payload: bytes = b"fixture") -> Path:
        path = self.build / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        return path

    def write_flasher_args(self, flash_files: dict[str, str], **extra: object) -> None:
        metadata: dict[str, object] = {"flash_files": flash_files, **extra}
        (self.build / "flasher_args.json").write_text(
            json.dumps(metadata), encoding="utf-8"
        )

    def load_packager(self, filename: str, module_name: str):
        spec = importlib.util.spec_from_file_location(module_name, SCRIPTS / filename)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_esp_idf_bundle_rewrites_all_metadata_paths(self) -> None:
        self.write_binary("bootloader/bootloader.bin")
        self.write_binary("app.bin")
        self.write_flasher_args(
            {"0x0": "bootloader/bootloader.bin", "0x10000": "app.bin"},
            bootloader={"file": "bootloader/bootloader.bin"},
        )
        github_output = self.output / "github-output.txt"
        env = dict(os.environ, GITHUB_SHA="deadbeef", PACKAGE_GIT_SHA="package-sha", GITHUB_OUTPUT=str(github_output))
        result = self.run_script(
            "package_esp_idf.py",
            "--project",
            "example",
            "--build-dir",
            str(self.build),
            "--name",
            "Example",
            "--framework-version",
            "v6.0.2",
            "--output-dir",
            str(self.output),
            env=env,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output / "Example-esp-idf-v6.0.2.zip"
        self.assertTrue(archive.is_file())
        prefix = "Example-esp-idf-v6.0.2"
        with zipfile.ZipFile(archive) as bundle:
            names = set(bundle.namelist())
            self.assertTrue(all(name.startswith(f"{prefix}/") for name in names))
            self.assertFalse(any(".." in Path(name).parts for name in names))
            self.assertIn(f"{prefix}/bin/bootloader.bin", names)
            self.assertIn(f"{prefix}/bin/app.bin", names)
            metadata = json.loads(bundle.read(f"{prefix}/flasher_args.json"))
            self.assertEqual(
                metadata["flash_files"],
                {"0x0": "bin/bootloader.bin", "0x10000": "bin/app.bin"},
            )
            self.assertEqual(metadata["bootloader"]["file"], "bin/bootloader.bin")
            manifest = json.loads(bundle.read(f"{prefix}/manifest.json"))
            self.assertEqual(manifest["git_sha"], "package-sha")
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(manifest["board"], "ESP32-C6-Touch-AMOLED-2.16")
            self.assertEqual(manifest["chip"], "esp32c6")
            self.assertEqual(manifest["source_project"], "example")
            self.assertEqual(manifest["flash"], {"baud": 921600})
            for entry in manifest["files"]:
                payload = bundle.read(f"{prefix}/{entry['archive_path']}")
                self.assertEqual(entry["file"], entry["archive_path"])
                self.assertEqual(entry["address"], entry["offset"])
                self.assertEqual(entry["size"], len(payload))
                self.assertEqual(entry["sha256"], hashlib.sha256(payload).hexdigest())
            shell_info = bundle.getinfo(f"{prefix}/flash.sh")
            self.assertTrue((shell_info.external_attr >> 16) & 0o111)
            for member in bundle.infolist():
                expected = 0o100755 if member.filename.endswith("/flash.sh") else 0o100644
                self.assertEqual(member.external_attr >> 16, expected)
            shell = bundle.read(f"{prefix}/flash.sh").decode("utf-8")
            self.assertIn("0x0 bin/bootloader.bin 0x10000 bin/app.bin", shell)
        self.assertIn(archive.as_posix(), github_output.read_text(encoding="utf-8"))

    def test_esp_idf_rejects_build_path_escape(self) -> None:
        outside = self.build.parent / "outside.bin"
        outside.write_bytes(b"outside")
        self.write_flasher_args({"0x0": "../outside.bin"})
        result = self.run_script(
            "package_esp_idf.py",
            "--project",
            "example",
            "--build-dir",
            str(self.build),
            "--name",
            "Example",
            "--framework-version",
            "v6.0.2",
            "--output-dir",
            str(self.output),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("escapes the build directory", result.stderr + result.stdout)
        outside.unlink(missing_ok=True)

    def test_esp_idf_rejects_absolute_reference_and_invalid_addresses(self) -> None:
        binary = self.write_binary("app.bin")
        common = [
            "--project", "example",
            "--build-dir", str(self.build),
            "--name", "Example",
            "--framework-version", "v6.0.2",
            "--output-dir", str(self.output),
        ]
        self.write_flasher_args({"0x0": str(binary.resolve())})
        absolute = self.run_script("package_esp_idf.py", *common)
        self.assertNotEqual(absolute.returncode, 0)
        self.assertIn("absolute build path", absolute.stderr + absolute.stdout)

        for address, message in (
            ("invalid", "Invalid flash address"),
            ("0x100000000", "outside the 32-bit range"),
        ):
            with self.subTest(address=address):
                self.write_flasher_args({address: "app.bin"})
                result = self.run_script("package_esp_idf.py", *common)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr + result.stdout)

    def test_packagers_reject_project_or_sketch_outside_repository(self) -> None:
        self.write_binary("app.bin")
        self.write_flasher_args({"0x0": "app.bin"})
        idf = self.run_script(
            "package_esp_idf.py",
            "--project", str(self.build),
            "--build-dir", str(self.build),
            "--name", "Example",
            "--framework-version", "v6.0.2",
            "--output-dir", str(self.output),
        )
        self.assertNotEqual(idf.returncode, 0)
        self.assertIn("must stay inside the repository", idf.stderr + idf.stdout)

        self.create_arduino_outputs()
        arduino = self.run_script(
            "package_arduino.py",
            "--sketch", str(self.build),
            "--build-dir", str(self.build),
            "--name", "Example",
            "--core-version", "3.3.11",
            "--output-dir", str(self.output),
        )
        self.assertNotEqual(arduino.returncode, 0)
        self.assertIn("must stay inside the repository", arduino.stderr + arduino.stdout)

    def test_packagers_reject_multiline_github_output_values(self) -> None:
        github_output = self.output / "github-output.txt"
        old_output = os.environ.get("GITHUB_OUTPUT")
        os.environ["GITHUB_OUTPUT"] = str(github_output)
        try:
            for script, module_name in (
                ("package_esp_idf.py", "package_esp_idf_test"),
                ("package_arduino.py", "package_arduino_test"),
            ):
                with self.subTest(script=script):
                    module = self.load_packager(script, module_name)
                    with self.assertRaisesRegex(SystemExit, "cannot contain a line break"):
                        module.write_output(Path("bad\nartifact.zip"))
        finally:
            if old_output is None:
                os.environ.pop("GITHUB_OUTPUT", None)
            else:
                os.environ["GITHUB_OUTPUT"] = old_output
        self.assertFalse(github_output.exists())

    def test_arduino_rejects_symlinked_output_escape_when_supported(self) -> None:
        self.create_arduino_outputs()
        linked = self.build / "Example.ino.bin"
        linked.unlink()
        outside = self.output / "outside.bin"
        outside.write_bytes(b"outside")
        try:
            linked.symlink_to(outside)
        except OSError as exc:
            self.skipTest(f"symlink creation is unavailable: {exc}")
        result = self.run_script("package_arduino.py", *self.arduino_arguments())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("escapes the build directory", result.stderr + result.stdout)

    def test_esp_idf_rejects_duplicate_normalized_address_and_basename(self) -> None:
        self.write_binary("a/first.bin")
        self.write_binary("b/second.bin")
        self.write_flasher_args({"0": "a/first.bin", "0x0": "b/second.bin"})
        common = [
            "--project", "example",
            "--build-dir", str(self.build),
            "--name", "Example",
            "--framework-version", "v6.0.2",
            "--output-dir", str(self.output),
        ]
        duplicate_address = self.run_script("package_esp_idf.py", *common)
        self.assertNotEqual(duplicate_address.returncode, 0)
        self.assertIn("Duplicate normalized flash address", duplicate_address.stderr + duplicate_address.stdout)

        self.write_binary("a/same.bin")
        self.write_binary("b/same.bin")
        self.write_flasher_args({"0x0": "a/same.bin", "0x10000": "b/same.bin"})
        duplicate_name = self.run_script("package_esp_idf.py", *common)
        self.assertNotEqual(duplicate_name.returncode, 0)
        self.assertIn("share the same bundle name", duplicate_name.stderr + duplicate_name.stdout)

    def create_arduino_outputs(self) -> None:
        for name in (
            "Example.ino.bootloader.bin",
            "Example.ino.partitions.bin",
            "boot_app0.bin",
            "Example.ino.bin",
        ):
            self.write_binary(name)

    def arduino_arguments(self) -> list[str]:
        return [
            "--sketch", "example",
            "--build-dir", str(self.build),
            "--name", "Example",
            "--core-version", "3.3.11",
            "--output-dir", str(self.output),
        ]

    def test_arduino_bundle_supports_external_build_and_output_directories(self) -> None:
        self.create_arduino_outputs()
        result = self.run_script("package_arduino.py", *self.arduino_arguments())
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output / "Example-arduino-3.3.11.zip"
        prefix = "Example-arduino-3.3.11"
        with zipfile.ZipFile(archive) as bundle:
            names = bundle.namelist()
            self.assertTrue(all(name.startswith(f"{prefix}/") for name in names))
            self.assertFalse(any(".." in Path(name).parts for name in names))
            manifest = json.loads(bundle.read(f"{prefix}/manifest.json"))
            self.assertEqual(manifest["sketch"], "example")
            self.assertEqual(manifest["git_sha"], "unknown")
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(manifest["board"], "ESP32-C6-Touch-AMOLED-2.16")
            self.assertEqual(manifest["chip"], "esp32c6")
            self.assertEqual(manifest["source_project"], "example")
            self.assertEqual(manifest["flash"], {"baud": 921600})
            self.assertEqual(len(manifest["files"]), 4)
            for entry in manifest["files"]:
                payload = bundle.read(f"{prefix}/{entry['archive_path']}")
                self.assertEqual(entry["file"], entry["archive_path"])
                self.assertEqual(entry["address"], entry["offset"])
                self.assertEqual(entry["size"], len(payload))
                self.assertEqual(entry["sha256"], hashlib.sha256(payload).hexdigest())
            batch = bundle.read(f"{prefix}/flash.bat").decode("utf-8")
            self.assertIn('0x10000 "bin\\Example.ino.bin"', batch)
            shell = bundle.read(f"{prefix}/flash.sh").decode("utf-8")
            self.assertIn("0x10000 bin/Example.ino.bin", shell)
            for member in bundle.infolist():
                expected = 0o100755 if member.filename.endswith("/flash.sh") else 0o100644
                self.assertEqual(member.external_attr >> 16, expected)

    def test_arduino_rejects_multiple_pattern_matches(self) -> None:
        self.create_arduino_outputs()
        self.write_binary("Other.ino.bin")
        result = self.run_script("package_arduino.py", *self.arduino_arguments())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Expected exactly one *.ino.bin", result.stderr + result.stdout)

    def test_arduino_rejects_shell_sensitive_output_names(self) -> None:
        self.create_arduino_outputs()
        (self.build / "Example.ino.bin").unlink()
        self.write_binary("Bad%Name.ino.bin")
        result = self.run_script("package_arduino.py", *self.arduino_arguments())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid Arduino output filename", result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
