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

from audit_markdown import (  # noqa: E402
    Change,
    audit,
    docs_only_findings,
    homepage_findings,
    link_findings,
    load_config,
    sensitive_findings,
)


def homepage(language: str) -> str:
    chinese = language == "zh"
    counterpart = "README.md" if chinese else "README_ZH.md"
    switch = "English" if chinese else "简体中文"
    subtitle = "ESP32-C6 产品仓库" if chinese else "ESP32-C6 product repository"
    labels = (
        ("🌐 产品", "📚 文档", "📦 固件", "🧩 ESP-IDF", "🔧 Arduino")
        if chinese
        else ("🌐 Product", "📚 Documentation", "📦 Firmware", "🧩 ESP-IDF", "🔧 Arduino")
    )
    headings = (
        ("## ✨ 概述", "## 🗂️ 仓库结构", "## 🧪 示例", "## 📦 固件", "## 🤝 支持与贡献")
        if chinese
        else (
            "## ✨ Overview",
            "## 🗂️ Repository structure",
            "## 🧪 Examples",
            "## 📦 Firmware",
            "## 🤝 Support and contributing",
        )
    )
    quick = " · ".join(
        [
            f'<a href="https://example.invalid/product">{labels[0]}</a>',
            f'<a href="https://example.invalid/docs">{labels[1]}</a>',
            f'<a href="https://example.invalid/firmware">{labels[2]}</a>',
            f'<a href="https://example.invalid/idf">{labels[3]}</a>',
            f'<a href="https://example.invalid/arduino">{labels[4]}</a>',
        ]
    )
    return (
        '<div align="center">\n'
        '<h1>Example Product</h1>\n'
        f'<strong>{subtitle}</strong>\n\n'
        f'<p><a href="{counterpart}">{switch}</a></p>\n'
        '<p><a href="https://example.invalid/actions"><img alt="Build" src="build.svg"></a> '
        '<a href="LICENSE.txt"><img alt="License" src="license.svg"></a></p>\n'
        '<img src="https://example.invalid/hero.png" alt="Product hero">\n'
        f'<p>{quick}</p>\n'
        '</div>\n\n---\n\n'
        "\n\n".join(headings)
        + "\n"
    )


class MarkdownPolicyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.config = load_config(POLICY_ROOT / "markdown-audit.json")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def codes(self, report: dict) -> set[str]:
        return {item["code"] for item in report["findings"]}

    def test_first_party_pair_and_reciprocal_links_pass(self) -> None:
        self.write("docs/GUIDE.md", "# Guide\n\n[简体中文](GUIDE_ZH.md)\n")
        self.write("docs/GUIDE_ZH.md", "# 指南\n\n[English](GUIDE.md)\n")
        report = audit(
            self.root,
            [Change("A", "docs/GUIDE.md"), Change("A", "docs/GUIDE_ZH.md")],
            self.config,
            all_mode=False,
            expect_docs_only=True,
        )
        self.assertEqual(report["summary"], {"errors": 0, "warnings": 0})

    def test_missing_pair_is_an_error_for_changed_markdown(self) -> None:
        self.write("docs/GUIDE.md", "# Guide\n")
        report = audit(
            self.root,
            [Change("A", "docs/GUIDE.md")],
            self.config,
            all_mode=False,
            expect_docs_only=False,
        )
        self.assertIn("BILINGUAL_PAIR_MISSING", self.codes(report))

    def test_wrong_language_internal_link_is_rejected(self) -> None:
        self.write(
            "docs/GUIDE.md",
            "# Guide\n\n[简体中文](GUIDE_ZH.md)\n\n[Other](OTHER.md)\n",
        )
        self.write(
            "docs/GUIDE_ZH.md",
            "# 指南\n\n[English](GUIDE.md)\n\n[其他](OTHER.md)\n",
        )
        self.write("docs/OTHER.md", "# Other\n\n[简体中文](OTHER_ZH.md)\n")
        self.write("docs/OTHER_ZH.md", "# 其他\n\n[English](OTHER.md)\n")
        report = audit(
            self.root,
            [Change("M", "docs/GUIDE_ZH.md")],
            self.config,
            all_mode=False,
            expect_docs_only=False,
        )
        self.assertIn("WRONG_LANGUAGE_INTERNAL_LINK", self.codes(report))

    def test_local_link_file_fragment_and_escape_checks(self) -> None:
        self.write("docs/GUIDE.md", "# Guide\n")
        self.write("docs/TARGET.md", "# Existing heading\n")
        text = (
            "[missing](MISSING.md)\n"
            "[fragment](TARGET.md#missing-heading)\n"
            "[escape](../../outside.md)\n"
        )
        codes = {item.code for item in link_findings(self.root, "docs/GUIDE.md", text, self.config)}
        self.assertIn("RELATIVE_LINK_MISSING", codes)
        self.assertIn("RELATIVE_LINK_FRAGMENT_MISSING", codes)
        self.assertIn("RELATIVE_LINK_ESCAPES_REPO", codes)

    def test_sensitive_public_text_shapes_are_detected(self) -> None:
        text = (
            "C:\\Users\\Alice\\work COM36 11:22:33:44:55:66 "
            "ghp_abcdefghijklmnopqrstuvwxyz123456\n"
        )
        codes = {item.code for item in sensitive_findings("docs/GUIDE.md", text, self.config)}
        self.assertTrue(
            {
                "LOCAL_ABSOLUTE_PATH",
                "ACTUAL_SERIAL_PORT",
                "MAC_ADDRESS",
                "CREDENTIAL_OR_TOKEN",
            }.issubset(codes)
        )

    def test_single_product_homepage_contract_is_symmetric(self) -> None:
        self.write("README.md", homepage("en"))
        self.write("README_ZH.md", homepage("zh"))
        self.write("LICENSE.txt", "fixture\n")
        settings = self.config["homepage_pairs"][0]
        findings = homepage_findings(self.root, "README.md", "README_ZH.md", settings)
        self.assertEqual(findings, [])
        self.write("README_ZH.md", homepage("zh").replace("## 🧪 示例", "## 示例"))
        codes = {
            item.code
            for item in homepage_findings(self.root, "README.md", "README_ZH.md", settings)
        }
        self.assertIn("HOMEPAGE_H2_EMOJI_MISSING", codes)
        self.assertIn("HOMEPAGE_H2_ASYMMETRY", codes)

    def test_docs_only_scope_rejects_source_config_binary_and_archive(self) -> None:
        findings = docs_only_findings(
            [
                Change("M", "main.c"),
                Change("M", "sdkconfig.defaults"),
                Change("M", "firmware/image.bin"),
                Change("M", "firmware/resources.zip"),
                Change("M", "notes.txt"),
                Change("M", "docs/allowed.png"),
            ],
            self.config,
        )
        codes = {item.code for item in findings}
        self.assertEqual(
            codes,
            {
                "DOCS_ONLY_SOURCE_CHANGE",
                "DOCS_ONLY_CONFIG_CHANGE",
                "DOCS_ONLY_FIRMWARE_BINARY",
                "DOCS_ONLY_RELEASE_PACKAGE",
                "DOCS_ONLY_NON_MARKDOWN_CHANGE",
            },
        )

    @unittest.skipUnless(shutil.which("git"), "git is required for the exact CLI contract")
    def test_exact_markdown_cli_invocation_passes_a_clean_base_diff(self) -> None:
        def git(*args: str) -> str:
            result = subprocess.run(
                ["git", *args],
                cwd=self.root,
                check=True,
                capture_output=True,
                text=True,
            )
            return result.stdout.strip()

        self.write("README.md", homepage("en"))
        self.write("README_ZH.md", homepage("zh"))
        self.write("LICENSE.txt", "fixture\n")
        self.write("docs/GUIDE.md", "# Guide\n\n[简体中文](GUIDE_ZH.md)\n")
        self.write("docs/GUIDE_ZH.md", "# 指南\n\n[English](GUIDE.md)\n")
        git("init", "-q")
        git("config", "user.email", "fixture@example.invalid")
        git("config", "user.name", "Fixture")
        git("add", ".")
        git("commit", "-qm", "base")
        base = git("rev-parse", "HEAD")
        self.write("docs/GUIDE.md", "# Guide\n\n[简体中文](GUIDE_ZH.md)\n\nUpdated.\n")
        self.write("docs/GUIDE_ZH.md", "# 指南\n\n[English](GUIDE.md)\n\n已更新。\n")
        git("add", ".")
        git("commit", "-qm", "docs")
        result = subprocess.run(
            [
                sys.executable,
                str(AUDIT_ROOT / "audit_markdown.py"),
                str(self.root),
                "--base",
                base,
                "--config",
                str(POLICY_ROOT / "markdown-audit.json"),
                "--strict",
                "--format",
                "json",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["summary"], {"errors": 0, "warnings": 0})


if __name__ == "__main__":
    unittest.main()
