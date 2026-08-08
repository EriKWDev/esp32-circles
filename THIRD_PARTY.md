# Third-party software

[简体中文](THIRD_PARTY_ZH.md)

This repository includes third-party source snapshots and components. Their
licenses remain authoritative within their respective directories.

| Software | Location | Version or source | License location |
| --- | --- | --- | --- |
| XPowersLib | `examples/arduino/libraries/XPowersLib` | 0.3.3, Lewis He | `examples/arduino/libraries/XPowersLib/LICENSE` |
| XPowersLib core | `components/xpowers` | Derived from the same 0.3.3 snapshot | `components/xpowers/LICENSE` |
| LVGL | `examples/arduino/libraries/lvgl8/lvgl` | 8.4.0 | `examples/arduino/libraries/lvgl8/lvgl/LICENCE.txt` |
| LVGL | `examples/arduino/libraries/lvgl9/lvgl` | 9.3.0 | `examples/arduino/libraries/lvgl9/lvgl/LICENCE.txt` |
| XiaoZhi | `firmware/xiaozhi` | Repository snapshot labeled 2.2.5 | `firmware/xiaozhi/LICENSE` |
| ESP Codec Device snapshot | Audio examples | Bundled source | License files beside the bundled source |

Managed ESP-IDF dependencies are resolved from each project's
`idf_component.yml`; consult the generated dependency metadata and upstream
component records for their exact versions and licenses. Do not infer that a
similarly named registry component is a drop-in replacement without API,
license, target, and hardware validation evidence.
