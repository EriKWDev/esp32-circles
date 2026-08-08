# Factory and recovery firmware

[简体中文](README_ZH.md)

This directory contains prebuilt images retained from the original repository:

| File | Role | SHA-256 |
| --- | --- | --- |
| `01_Fac-v1.0.0.bin` | Factory recovery image | `1DD2BE304BEC964831EEC8484C67C97D7B56934343DEFC4EBCAB63ED84D32C33` |
| `02_xiaozhi-v2.2.5.bin` | Prebuilt XiaoZhi image | `7FDDD220C86AD793F360EF88FEC8454C2765CF9C596C76FBD0753D01889AF169` |

These files are excluded from source-build CI. Do not rename, regenerate, or
replace them without recording their source, version, expected flash process,
checksum, and hardware validation result in release notes.

Follow the
[official product documentation](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)
for flashing instructions. A successful build of `firmware/xiaozhi/` would not
certify that these historical binaries match or can be reproduced from it.
