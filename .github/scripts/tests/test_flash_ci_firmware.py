from __future__ import annotations

import re
import shutil
import subprocess
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[3]
SCRIPT = REPOSITORY / "scripts" / "Flash-CI-Firmware.ps1"
COMMAND = REPOSITORY / "Flash-CI-Firmware.cmd"
NAMES = [
    "01_AXP2101_Test", "02_I2C_QMI8658", "03_I2C_PCF85063", "04_SD_Card",
    "05_WIFI_STA", "06_WIFI_AP", "07_Audio_Test", "08_LVGL_V8_Test", "09_LVGL_V9_Test",
]


class FlashCiFirmwareTest(unittest.TestCase):
    def test_static_safety_contract(self) -> None:
        command = COMMAND.read_text(encoding="utf-8")
        script = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("-STA", command)
        self.assertIn("exit /b %ERRORLEVEL%", command)
        for required in (
            "state-v1.json", "VID_303A&PID_1001", "write_flash",
            "Hash of data verified", "schema_version", "archive_path", "sha256", "16MB",
            "Assert-CleanWorktree", "Resolve-CurrentBranch", "Assert-ReadyPullRequest",
            "Resolve-ArtifactRuns", "Expand-SafeZip", "Mark PASS and flash next",
        ):
            self.assertIn(required, script)
        self.assertNotIn("erase_flash", script)
        self.assertNotIn("erase_region", script)

    def test_guided_list_and_self_test_without_hardware(self) -> None:
        shell = shutil.which("pwsh") or shutil.which("powershell")
        if shell is None:
            self.skipTest("pwsh or powershell is unavailable")
        self_test = subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(SCRIPT), "-SelfTest"],
            check=False, capture_output=True, text=True,
        )
        self.assertEqual(self_test.returncode, 0, self_test.stderr)
        self.assertIn("SELF_TEST_OK startIndex=1 transitions=26 completed=27", self_test.stdout)
        listed = subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(SCRIPT), "-ListOnly"],
            check=False, capture_output=True, text=True,
        )
        self.assertEqual(listed.returncode, 0, listed.stderr)
        entries = [line for line in listed.stdout.splitlines() if re.match(r"^\d+: workflow=", line)]
        self.assertEqual(len(entries), 27)
        expected = [
            f"{name}-esp-idf-{version}"
            for name in NAMES for version in ("v5.5.5", "v6.0.2")
        ] + [f"{name}-arduino-3.3.11" for name in NAMES]
        self.assertEqual([line.split(" artifact=", 1)[1].split(" source=", 1)[0] for line in entries], expected)


if __name__ == "__main__":
    unittest.main()
