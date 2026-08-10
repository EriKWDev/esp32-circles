# Changelog

[简体中文](CHANGELOG_ZH.md)

## Unreleased

- Normalize ESP-IDF, Arduino, firmware, component, documentation, and CI roots.
- Add exact-version ESP-IDF and Arduino CI matrices with reproducible flash
  bundles.
- Refresh GitHub-maintained checkout and artifact actions to their current
  Node.js 24 major versions.
- Add rename-aware, fail-closed change routing and a stable workflow result job.
- Add repository policy checks for bilingual Markdown, local links, public-text
  privacy, docs-only scope, and CI routing.
- Harden flash packaging against path escapes, ambiguous binaries, duplicate
  offsets or names, stale ESP-IDF metadata paths, and unsafe shell filenames.
- Add first-party English and Simplified Chinese documentation with a symmetric
  product homepage.
- Migrate all ESP-IDF examples from duplicated board wrappers to the shared
  ESP32-C6-Touch-AMOLED-2.16 BSP, pinned to its review commit until release.

## 1.0.0 - 2026-03-30

- Initial release.
- Add examples ordered from basic peripheral checks to richer UI applications.
