# Project structure

This repository is classified as a mixed board-support repository:

1. independent ESP-IDF examples;
2. independent Arduino sketches with pinned local libraries;
3. reusable board components mixed with example-specific policy wrappers;
4. a feature-rich source firmware tree;
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
| `firmware/xiaozhi` | Preserved source firmware | Not modified or validated by repository CI |
| `firmware/factory_firmware` | Recovery input | Not generated or validated by source CI |
| `release-artifacts` | Generated output | Never committed |

## Dependency policy

Use the ESP Component Registry for maintained display, touch, sensor, codec, and
bus components when an appropriate upstream component exists. Keep local code
when it is board-specific or when no authoritative managed component exists.

The XPowers source was duplicated across all ESP-IDF examples. The identical
core is now in `components/xpowers`; each example keeps its own `power_bsp.cpp`
because those wrappers express example-specific behavior.

## Validation boundary

The repository did not contain the product schematic at the time of this
restructure. Existing pin definitions were therefore preserved, not
re-derived. See [Hardware validation](HARDWARE_VALIDATION.md).
