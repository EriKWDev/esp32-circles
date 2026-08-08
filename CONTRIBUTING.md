# Contributing

[简体中文](CONTRIBUTING_ZH.md)

Thank you for improving ESP32-C6-Touch-AMOLED-2.16.

## Before opening a pull request

1. Keep each ESP-IDF project and Arduino sketch independently buildable.
2. Prefer maintained ESP Component Registry dependencies for reusable upstream
   drivers; keep board policy and product-specific glue local.
3. Do not mix LVGL 8 and LVGL 9 in one Arduino library search path.
4. Update both English and Simplified Chinese first-party documentation when
   moving paths, changing build inputs, or altering flash layout.
5. Do not commit build directories, generated lock files, credentials, release
   archives, or host-local paths.

Example compilation and artifact creation are validated by GitHub Actions. The
preserved firmware trees are outside this CI scope; never report compilation as
hardware validation.

## Hardware changes

Changes to pin mappings, display or touch timing, audio routing, PMU behavior,
storage, or partition layout need:

- the exact board revision;
- the affected example or firmware;
- an authoritative source for the hardware mapping;
- an on-device test result;
- logs or measurements sufficient to reproduce the result.

Use the [hardware validation checklist](docs/HARDWARE_VALIDATION.md) and clearly
mark any surface that was not tested.

## Pull request scope

Use a focused title and describe compatibility impact. Separate broad layout
work from functional firmware changes where practical. If CI repair needs
several small commits, maintainers may squash them after all required checks
pass.

The repository policy workflow checks bilingual pairs, local links, public-text
privacy, docs-only scope, and CI routing. The example workflows are the build
source of truth; local compilation is not a substitute for their result.
