# Repository migration

The layout changed without changing the purpose or ordering of the examples.

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
component instead of compiling private copies of the same source.

Build scripts and documentation should use the current paths. Release archives
are generated under `release-artifacts/` by CI and are not source files.
