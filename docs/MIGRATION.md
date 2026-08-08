# Repository migration

[简体中文](MIGRATION_ZH.md)

The layout changed without changing the purpose or numeric ordering of the
examples.

| Previous path | Current path |
| --- | --- |
| `01_Arduino_Libraries/` | `examples/arduino/libraries/` |
| `02_Example/Arduino-v3.3.3/` | `examples/arduino/examples/` |
| `02_Example/ESP-IDF-v5.5.3/` | `examples/esp-idf/` |
| `02_Example/XiaoZhi-v2.2.5/` | `firmware/xiaozhi/` |
| `03_Firmware/` | `firmware/factory_firmware/` |
| `Tools Configuration.png` | `docs/assets/arduino-tools-configuration.png` |

ESP-IDF projects now add the repository-level `components/` directory through
`EXTRA_COMPONENT_DIRS`. Their PMIC wrappers require the shared `xpowers`
component instead of compiling private copies of the same core, while
example-specific `power_bsp.cpp` policy remains local.

Build scripts and documentation must use current paths. CI routing retains the
legacy path map so the large migration diff can be classified without treating
old product files as unknown inputs. Deleted or renamed example paths still
trigger validation of the affected framework.

Generated release archives belong under `release-artifacts/` and are not source
files. The preserved XiaoZhi tree and factory binaries keep their separate
provenance and remain outside default example CI.
