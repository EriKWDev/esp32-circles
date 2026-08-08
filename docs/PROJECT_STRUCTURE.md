# Project structure

[简体中文](PROJECT_STRUCTURE_ZH.md)

This repository is a mixed board-support repository:

1. independent ESP-IDF examples;
2. independent Arduino sketches with pinned local libraries;
3. reusable board components mixed with example-specific policy wrappers;
4. a feature-rich preserved source firmware tree;
5. recovery binaries with different provenance from source-built artifacts.

It is not an ESP32-P4 plus hosted-wireless repository. References to P4 and
hosted transports under `firmware/xiaozhi/` belong to the upstream multi-board
firmware source and do not describe this product.

## Ownership boundaries

| Path | Maintained as | Notes |
| --- | --- | --- |
| `examples/esp-idf/<name>` | Independent project | Must configure and build without another example |
| `examples/arduino/examples/<name>` | Independent sketch | CI injects only the pinned libraries it needs |
| `examples/arduino/libraries` | Arduino dependency snapshot | LVGL major versions remain isolated |
| `components/xpowers` | Shared local component | Common source only; no board-specific power policy |
| `examples/esp-idf/*/components/pmicpower` | Example policy wrapper | Startup delays and peripheral policy remain local |
| `firmware/xiaozhi` | Preserved upstream source | Not modified or validated by repository CI |
| `firmware/factory_firmware` | Immutable recovery input | Not generated or validated by source CI |
| `.github/policy` | First-party policy | Routing and Markdown ownership declarations |
| `.github/scripts/repository_audit` | First-party audit tooling | Inventory, Markdown, privacy, and CI routing checks |
| `release-artifacts` | Generated output | Never committed |

## Dependency policy

Use the ESP Component Registry for maintained display, touch, sensor, codec, and
bus components when an authoritative compatible upstream component exists. Keep
local code when it is board-specific or when API, license, target, ownership, or
hardware-equivalence evidence is incomplete.

The XPowers source was duplicated across all ESP-IDF examples. The identical
core is now in `components/xpowers`; each example keeps its own `power_bsp.cpp`
because those wrappers express example-specific behavior. Bundled Arduino
libraries and the XiaoZhi snapshot remain provenance-preserved upstream content,
not first-party code to normalize opportunistically.

## Validation boundary

The repository itself does not contain the product schematic. Existing pin
definitions were preserved, not re-derived, and the current examples configure
16 MB flash without establishing PSRAM support. See
[Hardware validation](HARDWARE_VALIDATION.md) for the official schematic link
and the evidence required before changing hardware mappings.
