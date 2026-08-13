from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[3]
AUDIT_ROOT = REPOSITORY / ".github" / "scripts" / "repository_audit"
POLICY_ROOT = REPOSITORY / ".github" / "policy"
sys.path.insert(0, str(AUDIT_ROOT))

from audit_ci_routing import (  # noqa: E402
    RoutingError,
    load_routing_config,
    route_changes,
)
from audit_markdown import Change, load_config  # noqa: E402


class RoutingFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.routing = load_routing_config(POLICY_ROOT / "ci-routing.json")
        self.ownership = load_config(POLICY_ROOT / "markdown-audit.json")
        self._write("README.md", "# Fixture\n")
        for name in ("alpha", "beta"):
            self._write(
                f"examples/esp-idf/{name}/CMakeLists.txt",
                "cmake_minimum_required(VERSION 3.16)\n"
                "include($ENV{IDF_PATH}/tools/cmake/project.cmake)\n"
                f"project({name})\n",
            )
            self._write(
                f"examples/esp-idf/{name}/main/CMakeLists.txt",
                'idf_component_register(SRCS "main.c")\n',
            )
            self._write(f"examples/esp-idf/{name}/main/main.c", "void app_main(void) {}\n")
            self._write(
                f"examples/esp-idf/{name}/sdkconfig.defaults",
                'CONFIG_IDF_TARGET="esp32c6"\n',
            )
        for name in ("Blink", "Sensor"):
            self._write(f"examples/arduino/{name}/{name}.ino", "void setup() {}\nvoid loop() {}\n")
        self._write("examples/arduino/libraries/Upstream/README.md", "# Upstream\n")
        self._write("components/xpowers/source.cpp", "int shared;\n")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def route(self, *changes: Change) -> dict:
        return route_changes(
            self.root,
            list(changes),
            self.routing,
            self.ownership,
            max_files=1_000,
            max_text_files=1_000,
        )

    def test_documentation_never_selects_examples(self) -> None:
        report = self.route(
            Change("M", "README.md"),
            Change("M", "examples/esp-idf/alpha/README.md"),
            Change("M", "examples/arduino/Blink/README.md"),
            Change("M", "examples/arduino/libraries/Upstream/README.md"),
        )
        self.assertTrue(report["scope"]["docs_only"])
        self.assertEqual(report["esp_idf"]["mode"], "none")
        self.assertEqual(report["arduino"]["mode"], "none")

    def test_direct_sources_select_only_the_affected_entries(self) -> None:
        report = self.route(
            Change("M", "examples/esp-idf/alpha/main/main.c"),
            Change("M", "examples/arduino/Blink/Blink.ino"),
        )
        self.assertEqual(report["esp_idf"]["mode"], "selected")
        self.assertEqual(report["esp_idf"]["selected"], ["examples/esp-idf/alpha"])
        self.assertEqual(
            report["arduino"]["selected"],
            ["examples/arduino/Blink/Blink.ino"],
        )

    def test_shared_and_workflow_inputs_expand_the_matrix(self) -> None:
        idf = self.route(Change("M", "components/xpowers/source.cpp"))
        self.assertEqual(idf["esp_idf"]["mode"], "all")
        self.assertEqual(idf["arduino"]["mode"], "none")
        global_report = self.route(Change("M", ".github/workflows/esp-idf-examples.yml"))
        self.assertEqual(global_report["esp_idf"]["mode"], "all")
        self.assertEqual(global_report["arduino"]["mode"], "all")

    def test_firmware_kinds_stay_outside_example_ci(self) -> None:
        report = self.route(
            Change("M", "firmware/README.md"),
            Change("M", "firmware/demo/main.c"),
            Change("M", "firmware/factory/image.bin"),
            Change("M", "firmware/factory/resources.zip"),
        )
        self.assertTrue(report["scope"]["firmware_touched"])
        self.assertTrue(report["scope"]["release_review_required"])
        self.assertFalse(report["scope"]["example_build_required"])
        self.assertEqual(report["esp_idf"]["mode"], "none")
        self.assertEqual(report["arduino"]["mode"], "none")
        self.assertEqual(
            {item["kind"] for item in report["routes"]},
            {"documentation", "firmware_source_or_config", "firmware_delivery_artifact"},
        )

    def test_canonical_firmware_surfaces_are_classified_without_example_builds(self) -> None:
        markdown = self.route(Change("M", "firmware/xiaozhi/README.md"))
        source = self.route(Change("M", "firmware/xiaozhi/sdkconfig.defaults"))
        binary = self.route(Change("M", "firmware/factory_firmware/01_Fac-v1.0.0.bin"))
        for report in (markdown, source, binary):
            self.assertTrue(report["scope"]["firmware_touched"])
            self.assertFalse(report["scope"]["example_build_required"])
            self.assertEqual(report["esp_idf"]["mode"], "none")
            self.assertEqual(report["arduino"]["mode"], "none")
            self.assertEqual(report["unknown_paths"], [])
        self.assertTrue(markdown["scope"]["docs_only"])
        self.assertFalse(markdown["scope"]["release_review_required"])
        self.assertEqual(markdown["routes"][0]["kind"], "documentation")
        self.assertFalse(source["scope"]["docs_only"])
        self.assertFalse(source["scope"]["release_review_required"])
        self.assertEqual(source["routes"][0]["kind"], "firmware_source_or_config")
        self.assertFalse(binary["scope"]["docs_only"])
        self.assertTrue(binary["scope"]["release_review_required"])
        self.assertEqual(binary["routes"][0]["kind"], "firmware_delivery_artifact")

    def test_canonical_firmware_rename_and_legacy_deletion_remain_firmware(self) -> None:
        renamed = self.route(
            Change(
                "R",
                "firmware/xiaozhi/main/main.c",
                "02_Example/XiaoZhi-v2.2.5/main/main.c",
            )
        )
        deleted = self.route(Change("D", "03_Firmware/retired.bin"))
        for report in (renamed, deleted):
            self.assertTrue(report["scope"]["firmware_touched"])
            self.assertFalse(report["scope"]["example_build_required"])
            self.assertEqual(report["esp_idf"]["mode"], "none")
            self.assertEqual(report["arduino"]["mode"], "none")
            self.assertEqual(report["unknown_paths"], [])
        self.assertFalse(renamed["scope"]["release_review_required"])
        self.assertEqual(renamed["scope"]["impact_paths"], 2)
        self.assertTrue(deleted["scope"]["release_review_required"])

    def test_policy_inputs_stay_in_the_visible_lightweight_gate_only(self) -> None:
        report = self.route(
            Change("M", ".github/policy/ci-routing.json"),
            Change("M", ".github/policy/firmware-integrity.json"),
            Change("M", ".github/scripts/repository_audit/check_firmware_integrity.py"),
            Change("M", ".github/scripts/tests/test_firmware_integrity.py"),
        )
        self.assertFalse(report["scope"]["example_build_required"])
        self.assertEqual(report["unknown_paths"], [])
        self.assertEqual(report["esp_idf"]["mode"], "none")
        self.assertEqual(report["arduino"]["mode"], "none")

    def test_deleted_or_renamed_example_routes_the_remaining_framework(self) -> None:
        deleted = self.route(Change("D", "examples/esp-idf/removed/main/main.c"))
        self.assertEqual(deleted["esp_idf"]["mode"], "all")
        renamed = self.route(
            Change(
                "R",
                "docs/retired-example.md",
                "examples/arduino/Removed/Removed.ino",
            )
        )
        self.assertEqual(renamed["arduino"]["mode"], "all")
        self.assertEqual(renamed["scope"]["impact_paths"], 2)

    def test_empty_scope_fails_closed(self) -> None:
        with self.assertRaises(RoutingError):
            self.route()

    def test_unknown_path_is_visible_and_conservatively_selects_all(self) -> None:
        report = self.route(Change("M", ".github/custom/tool.py"))
        self.assertEqual(report["unknown_paths"], [".github/custom/tool.py"])
        self.assertEqual(report["esp_idf"]["mode"], "all")
        self.assertEqual(report["arduino"]["mode"], "all")

    @unittest.skipUnless(shutil.which("git"), "git is required for the exact CLI contract")
    def test_exact_policy_cli_invocation_uses_a_complete_base_diff(self) -> None:
        def git(*args: str) -> str:
            result = subprocess.run(
                ["git", *args],
                cwd=self.root,
                check=True,
                capture_output=True,
                text=True,
            )
            return result.stdout.strip()

        git("init", "-q")
        git("config", "user.email", "fixture@example.invalid")
        git("config", "user.name", "Fixture")
        git("add", ".")
        git("commit", "-qm", "base")
        base = git("rev-parse", "HEAD")
        self._write("firmware/xiaozhi/sdkconfig.defaults", 'CONFIG_IDF_TARGET="esp32c6"\n')
        git("add", ".")
        git("commit", "-qm", "change")
        command = [
            sys.executable,
            str(AUDIT_ROOT / "audit_ci_routing.py"),
            str(self.root),
            "--base",
            base,
            "--routing-config",
            str(POLICY_ROOT / "ci-routing.json"),
            "--ownership-config",
            str(POLICY_ROOT / "markdown-audit.json"),
            "--strict-unknown",
            "--format",
            "json",
        ]
        result = subprocess.run(command, check=False, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["scope"]["firmware_touched"])
        self.assertFalse(report["scope"]["release_review_required"])
        self.assertFalse(report["scope"]["example_build_required"])
        self.assertEqual(report["unknown_paths"], [])
        self.assertEqual(report["esp_idf"]["selected"], [])
        self.assertEqual(report["arduino"]["mode"], "none")


class RepositoryDiscoveryTest(unittest.TestCase):
    def test_common_bsp_source_selects_all_nine_arduino_sketches(self) -> None:
        report = route_changes(
            REPOSITORY,
            [Change("M", "examples/arduino/libraries/C6_AMOLED_BSP/src/C6AmoledBsp.cpp")],
            load_routing_config(POLICY_ROOT / "ci-routing.json"),
            load_config(POLICY_ROOT / "markdown-audit.json"),
            max_files=1_000,
            max_text_files=1_000,
        )
        self.assertEqual(report["arduino"]["mode"], "all")
        self.assertEqual(len(report["arduino"]["selected"]), 9)

    def test_guided_flasher_inputs_select_all_examples(self) -> None:
        for path in ("Flash-CI-Firmware.cmd", "scripts/Flash-CI-Firmware.ps1"):
            with self.subTest(path=path):
                report = route_changes(
                    REPOSITORY,
                    [Change("M", path)],
                    load_routing_config(POLICY_ROOT / "ci-routing.json"),
                    load_config(POLICY_ROOT / "markdown-audit.json"),
                    max_files=1_000,
                    max_text_files=1_000,
                )
                self.assertEqual(report["esp_idf"]["mode"], "all")
                self.assertEqual(len(report["esp_idf"]["selected"]), 9)
                self.assertEqual(report["arduino"]["mode"], "all")
                self.assertEqual(len(report["arduino"]["selected"]), 9)

    def test_product_discoverers_find_only_the_nine_first_party_entries(self) -> None:
        scripts = REPOSITORY / ".github" / "scripts"
        for script in ("discover_esp_idf_examples.py", "discover_arduino_examples.py"):
            result = subprocess.run(
                [sys.executable, str(scripts / script), "--selection", "all"],
                cwd=REPOSITORY,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            entries = json.loads(result.stdout)
            self.assertEqual(len(entries), 9)
            self.assertEqual([item["name"] for item in entries], [
                "01_AXP2101_Test",
                "02_I2C_QMI8658",
                "03_I2C_PCF85063",
                "04_SD_Card",
                "05_WIFI_STA",
                "06_WIFI_AP",
                "07_Audio_Test",
                "08_LVGL_V8_Test",
                "09_LVGL_V9_Test",
            ])


if __name__ == "__main__":
    unittest.main()
