# Repository migration

[简体中文](MIGRATION_ZH.md)

The layout changed without changing the purpose or numeric ordering of the
examples.

| Previous path | Current path |
| --- | --- |
| `01_Arduino_Libraries/` | `examples/arduino/libraries/` |
| `02_Example/Arduino-v3.3.3/` | `examples/arduino/` (nine direct sketch directories) |
| `02_Example/ESP-IDF-v5.5.3/` | `examples/esp-idf/` |
| `02_Example/XiaoZhi-v2.2.5/` | `firmware/xiaozhi/` |
| `03_Firmware/` | `firmware/factory_firmware/` |
| `Tools Configuration.png` | `docs/assets/arduino-tools-configuration.png` |

The ESP-IDF projects now consume the shared
`waveshare/esp32_c6_touch_amoled_2_16` BSP instead of compiling private I2C,
PMU, display, touch, storage, and audio board wrappers. During validation, each
project manifest pins the exact commit from BSP PR #185. Once that PR is merged,
the BSP is published, and both CI matrices pass, the Git dependency will be
replaced with the formal registry release. Product application logic, managed
sensor dependencies, and the shared status UI remain local.

Build scripts and documentation must use current paths. CI routing retains the
legacy path map so the large migration diff can be classified without treating
old product files as unknown inputs. Deleted or renamed example paths still
trigger validation of the affected framework.

The product-maintained `libraries/C6_AMOLED_BSP` centralizes C6 pins, I2C,
AXP2101, SH8601, touch, and LVGL 8/9 integration. CI continues to stage only
the fixed dependencies required by each sketch.

Generated release archives belong under `release-artifacts/` and are not source
files. The preserved XiaoZhi tree and factory binaries keep their separate
provenance and remain outside default example CI.
