# ESP-IDF examples

[简体中文](README_ZH.md)

Each child directory is a self-contained ESP-IDF project targeting ESP32-C6.
Projects use managed components for suitable upstream drivers and the shared
repository component in `components/xpowers` for common PMU code.

Examples 01–06 explicitly load `common/components/status_ui`. Its LVGL v9
dependency is isolated from examples 07–09, which retain their own LVGL major.

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
