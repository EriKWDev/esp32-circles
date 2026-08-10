# ESP-IDF examples

[简体中文](README_ZH.md)

Each child directory is a self-contained ESP-IDF project targeting ESP32-C6.
Board-level I2C, PMU, display, touch, storage, and audio support comes from the
`waveshare/esp32_c6_touch_amoled_2_16` BSP. During BSP review, every project
pins the same exact commit from
[Waveshare-ESP32-components PR #185](https://github.com/waveshareteam/Waveshare-ESP32-components/pull/185).
The dependency will move to a released registry version only after that BSP and
this product matrix have both passed CI.

Examples 01–06 explicitly load the product-level
`common/components/status_ui`. Its LVGL v9 dependency is isolated from examples
07–09, which retain their own LVGL major. The status page is application UI;
hardware initialization remains owned by the BSP.

Examples 01–06 now publish their PMIC, sensor, SD-card, or Wi-Fi state to the
on-device status screen while retaining their existing serial logs. Screen
initialization is optional for the SD-card and Wi-Fi examples, so a display
initialization failure does not stop their core demonstration flow.

Build one project from its own directory:

```sh
idf.py set-target esp32c6
idf.py build
idf.py flash monitor
```

CI discovers a direct child by requiring both `CMakeLists.txt` and a `main/`
directory. A new example following that shape enters the matrix automatically.
The workflow validates every discovered project with ESP-IDF v5.5.5 and v6.0.2.
It packages each successful build with its exact binaries and flash metadata.

All current projects select ESP32-C6 and 16 MB flash through
`sdkconfig.defaults`. No project enables PSRAM. Compilation is not evidence that
the existing GPIO assignments or connected peripherals match a particular PCB
revision.
