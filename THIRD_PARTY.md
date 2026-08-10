# Third-party software

[简体中文](THIRD_PARTY_ZH.md)

This repository includes third-party source snapshots and components. Their
licenses remain authoritative within their respective directories.

`examples/arduino/libraries/C6_AMOLED_BSP` is a first-party board wrapper, not
third-party software. It centralizes the C6 pins, I2C, AXP2101, SH8601, touch,
and LVGL 8/9 integration used by the direct Arduino-root sketches.

| Software | Location | Version or source | License location |
| --- | --- | --- | --- |
| XPowersLib | `examples/arduino/libraries/XPowersLib` | 0.3.3, Lewis He | `examples/arduino/libraries/XPowersLib/LICENSE` |
| LVGL | `examples/arduino/libraries/lvgl8/lvgl` | 8.4.0 | `examples/arduino/libraries/lvgl8/lvgl/LICENCE.txt` |
| LVGL | `examples/arduino/libraries/lvgl9/lvgl` | 9.3.0 | `examples/arduino/libraries/lvgl9/lvgl/LICENCE.txt` |
| XiaoZhi | `firmware/xiaozhi` | Repository snapshot labeled 2.2.5 | `firmware/xiaozhi/LICENSE` |
| ESP Codec Device snapshot | Audio examples | Bundled source | License files beside the bundled source |

Managed ESP-IDF dependencies are resolved from each project's
`idf_component.yml`; consult the generated dependency metadata and upstream
component records for their exact versions and licenses. Do not infer that a
similarly named registry component is a drop-in replacement without API,
license, target, and hardware validation evidence.

During BSP review, `waveshare/esp32_c6_touch_amoled_2_16` is fetched from the
Waveshare components repository at one exact PR commit. Its upstream component
manifest and license are authoritative until the dependency moves to the formal
registry release.
