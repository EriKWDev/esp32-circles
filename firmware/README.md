# Firmware

[简体中文](README_ZH.md)

| Path | Purpose |
| --- | --- |
| `xiaozhi/` | Preserved upstream XiaoZhi source snapshot labeled 2.2.5 |
| `factory_firmware/` | Immutable prebuilt recovery images supplied with the product repository |

The two paths have different provenance and are retained as supplied. They are
not modified, compiled, packaged, or validated by this repository's default CI.
Files under them are not interchangeable with generated example artifacts.

See [Firmware artifacts](../docs/FIRMWARE.md) for checksums and provenance rules,
[Factory and recovery firmware](factory_firmware/README.md) for the immutable
inventory, and the
[official product documentation](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)
for recovery flashing instructions.
