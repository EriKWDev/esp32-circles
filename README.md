<div align="center">
<h1>ESP32-C6-Touch-AMOLED-2.16</h1>
<strong>Examples, reusable components, firmware references, and recovery resources for the Waveshare ESP32-C6 touch AMOLED board</strong>

<p><a href="README_ZH.md">简体中文</a></p>
<p>
  <a href="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/esp-idf-examples.yml"><img alt="ESP-IDF examples" src="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/esp-idf-examples.yml/badge.svg"></a>
  <a href="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/arduino-examples.yml"><img alt="Arduino examples" src="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/arduino-examples.yml/badge.svg"></a>
  <a href="LICENSE.txt"><img alt="Apache-2.0 license" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
</p>
<img src="https://www.waveshare.com/img/devkit/ESP32-C6-Touch-AMOLED-2.16/ESP32-C6-Touch-AMOLED-2.16-details-1.jpg" alt="ESP32-C6-Touch-AMOLED-2.16 product" width="760">
<p>
  <a href="https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16">🌐 Product</a> ·
  <a href="docs/PROJECT_STRUCTURE.md">📚 Documentation</a> ·
  <a href="firmware/README.md">📦 Firmware</a> ·
  <a href="examples/esp-idf/README.md">🧩 ESP-IDF</a> ·
  <a href="examples/arduino/README.md">🔧 Arduino</a>
</p>
</div>

---

## ✨ Overview

This repository contains nine first-party ESP-IDF projects, nine first-party
Arduino sketches, reusable board components, a preserved XiaoZhi source snapshot,
and prebuilt recovery images for the Waveshare ESP32-C6-Touch-AMOLED-2.16.

The examples target ESP32-C6 with 16 MB flash. The current project configurations
do not claim PSRAM support. Compilation in GitHub Actions proves software build
compatibility only; see [hardware validation](docs/HARDWARE_VALIDATION.md) before
making claims about a physical board.

## 🗂️ Repository structure

| Path | Purpose |
| --- | --- |
| `examples/esp-idf/` | Independent first-party ESP-IDF projects |
| `examples/arduino/examples/` | Independent first-party Arduino sketches |
| `examples/arduino/libraries/` | Pinned libraries used by Arduino examples |
| `components/` | Reusable local ESP-IDF components |
| `firmware/xiaozhi/` | Preserved upstream XiaoZhi source snapshot |
| `firmware/factory_firmware/` | Immutable prebuilt recovery images |
| `docs/` | CI, firmware, migration, structure, and validation notes |

Read [Project structure](docs/PROJECT_STRUCTURE.md) for ownership boundaries and
[Repository migration](docs/MIGRATION.md) for the old-to-new path map.

## 🧪 Examples

### ESP-IDF

Each directory under `examples/esp-idf/` is an independent project:

```sh
cd examples/esp-idf/01_AXP2101_Test
idf.py set-target esp32c6
idf.py build
idf.py flash monitor
```

GitHub Actions validates every project against the exact maintained ESP-IDF 5.5
and 6.0 versions pinned in the workflow. For scope and artifact details, see
[Continuous integration](docs/CI.md).

### Arduino

Open the `.ino` file inside an example directory. Select ESP32-C6, 16 MB flash,
QIO mode, and the 3 MB application/9 MB FAT partition scheme shown below. The
workflow pins the exact Arduino-ESP32 core used for validation.

![Arduino IDE Tools configuration](docs/assets/arduino-tools-configuration.png)

LVGL 8 and LVGL 9 deliberately use separate library roots. Use LVGL 8 only with
`08_LVGL_V8_Test`; `07_Audio_Test` and `09_LVGL_V9_Test` use LVGL 9.

## 📦 Firmware

The repository keeps the XiaoZhi source snapshot and the factory/recovery images
as provenance-preserved inputs. They are outside the default example-build and
packaging workflows. Read [Firmware](firmware/README.md) before flashing or
changing either tree.

Successful example jobs publish reproducible ZIP bundles containing binaries,
flash offsets, a manifest, and platform-specific flash helpers. Those generated
bundles are CI outputs and are never committed.

## 🤝 Support and contributing

Start with the [official product documentation](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)
and [Support](SUPPORT.md). Report reproducible repository defects through GitHub
Issues, without credentials or private device and network data.

Before proposing a change, read [Contributing](CONTRIBUTING.md),
[Security policy](SECURITY.md), and [Third-party software](THIRD_PARTY.md).
Hardware-affecting changes require a board revision, source evidence, and an
on-device result in addition to green CI.
