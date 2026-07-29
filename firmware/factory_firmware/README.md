# Factory and recovery firmware

This directory contains prebuilt images retained from the original repository:

| File | Role |
| --- | --- |
| `01_Fac-v1.0.0.bin` | Factory recovery image |
| `02_xiaozhi-v2.2.5.bin` | Prebuilt XiaoZhi image |

These files are excluded from source-build CI. Do not rename, regenerate, or
replace them without recording their source, version, expected flash offset,
checksum, and hardware validation result in the release notes.

Follow the product wiki for flashing instructions. A successful CI build of
`firmware/xiaozhi/` does not certify that these historical binaries match it.
