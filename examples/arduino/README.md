# Arduino examples

[简体中文](README_ZH.md)

The nine first-party sketches are direct children of this Arduino root; pinned
libraries are in `libraries/`. The discovery script inspects only direct children
that contain an `.ino` file, so bundled library examples never enter the product
matrix.

The CI build script supplies XPowersLib to all sketches. It supplies LVGL 8 only
to `08_LVGL_V8_Test` and LVGL 9 only to `07_Audio_Test` and
`09_LVGL_V9_Test`, preventing incompatible major versions from being visible to
the compiler at the same time.

`libraries/C6_AMOLED_BSP` is a first-party board library that centralizes C6
pins, I2C, AXP2101, SH8601, touch, and LVGL 8/9 integration. CI stages only the
fixed dependencies needed by each sketch.

The pinned board configuration is ESP32-C6 with CDC on boot, 16 MB flash, QIO at
80 MHz, 160 MHz CPU, and the 3 MB application/9 MB FAT partition scheme. The
workflow currently validates Arduino-ESP32 3.3.11. It does not configure or prove
PSRAM support.

For Arduino IDE setup, see the [repository homepage](../../README.md) and the
[official product documentation](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16).
