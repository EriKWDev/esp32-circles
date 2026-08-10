<div align="center">
<h1>ESP32-C6-Touch-AMOLED-2.16</h1>
<strong>适用于微雪 ESP32-C6 触控 AMOLED 开发板的示例、可复用组件、固件参考与恢复资源</strong>

<p><a href="README.md">English</a></p>
<p>
  <a href="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/esp-idf-examples.yml"><img alt="ESP-IDF 示例" src="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/esp-idf-examples.yml/badge.svg"></a>
  <a href="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/arduino-examples.yml"><img alt="Arduino 示例" src="https://github.com/waveshareteam/ESP32-C6-Touch-AMOLED-2.16/actions/workflows/arduino-examples.yml/badge.svg"></a>
  <a href="LICENSE.txt"><img alt="Apache-2.0 许可证" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
</p>
<img src="https://www.waveshare.com/img/devkit/ESP32-C6-Touch-AMOLED-2.16/ESP32-C6-Touch-AMOLED-2.16-details-1.jpg" alt="ESP32-C6-Touch-AMOLED-2.16 产品图" width="760">
<p>
  <a href="https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16">🌐 产品</a> ·
  <a href="docs/PROJECT_STRUCTURE_ZH.md">📚 文档</a> ·
  <a href="firmware/README_ZH.md">📦 固件</a> ·
  <a href="examples/esp-idf/README_ZH.md">🧩 ESP-IDF</a> ·
  <a href="examples/arduino/README_ZH.md">🔧 Arduino</a>
</p>
</div>

---

## ✨ 概述

本仓库包含微雪 ESP32-C6-Touch-AMOLED-2.16 的 9 个第一方 ESP-IDF 工程、
9 个第一方 Arduino 草图、托管开发板集成、保留的 XiaoZhi 源码快照和
预编译恢复镜像。

示例面向带 16 MB Flash 的 ESP32-C6。当前工程配置不声明支持 PSRAM。
GitHub Actions 编译只能证明软件构建兼容性；如需对实体开发板作出结论，
请先阅读[硬件验证边界](docs/HARDWARE_VALIDATION_ZH.md)。

## 🗂️ 仓库结构

| 路径 | 用途 |
| --- | --- |
| `examples/esp-idf/` | 相互独立的第一方 ESP-IDF 工程 |
| `examples/arduino/<name>/` | 直接位于 Arduino 根目录下的 9 个相互独立的第一方草图 |
| `examples/arduino/libraries/` | 固定依赖库和第一方 `C6_AMOLED_BSP` 开发板共享库 |
| `examples/esp-idf/common/components/` | 由部分 ESP-IDF 示例共享的产品级组件 |
| `firmware/xiaozhi/` | 保留的上游 XiaoZhi 源码快照 |
| `firmware/factory_firmware/` | 不可变的预编译恢复镜像 |
| `docs/` | CI、固件、迁移、结构和验证说明 |

请通过[项目结构](docs/PROJECT_STRUCTURE_ZH.md)了解所有权边界，通过
[仓库迁移](docs/MIGRATION_ZH.md)查看新旧路径映射。

## 🧪 示例

### ESP-IDF

`examples/esp-idf/` 下的每个目录都是独立工程：

```sh
cd examples/esp-idf/01_AXP2101_Test
idf.py set-target esp32c6
idf.py build
idf.py flash monitor
```

GitHub Actions 会使用工作流中精确固定的 ESP-IDF 5.5 和 6.0 版本验证全部
工程。CI 范围和产物格式见[持续集成](docs/CI_ZH.md)。

全部 9 个工程都使用共享的微雪 ESP32-C6-Touch-AMOLED-2.16 BSP。BSP 评审期间，
工程清单固定其 PR 精确提交；只有两个仓库都通过 CI 后，依赖才会切换到正式注册表版本。

### Arduino

打开示例目录内的 `.ino` 文件，并选择 ESP32-C6、16 MB Flash、QIO 模式和
3 MB 应用程序/9 MB FAT 分区方案，如下图所示。工作流会固定用于验证的
Arduino-ESP32 Core 精确版本。

![Arduino IDE 工具配置](docs/assets/arduino-tools-configuration.png)

LVGL 8 与 LVGL 9 有意使用不同的库目录。仅 `08_LVGL_V8_Test` 使用 LVGL 8；
`07_Audio_Test` 和 `09_LVGL_V9_Test` 使用 LVGL 9。
`libraries/C6_AMOLED_BSP` 集中维护 C6 引脚、I2C、AXP2101、SH8601、触控和
LVGL 8/9 的开发板集成。CI 仍只为每个草图暂存其固定依赖。

## 📦 固件

仓库把 XiaoZhi 源码快照与出厂/恢复镜像作为保留来源信息的输入保存，
它们不在默认示例构建和打包工作流范围内。刷写或修改任一目录前，请先阅读
[固件说明](firmware/README_ZH.md)。

成功的示例任务会发布可复现 ZIP 包，其中包含二进制文件、烧录偏移、清单和
不同平台的烧录辅助脚本。这些生成包只作为 CI 产物提供，不会提交到仓库。

## 🤝 支持与贡献

请先查阅[官方产品文档](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)
和[支持说明](SUPPORT_ZH.md)。可复现的仓库缺陷可通过 GitHub Issues 报告，
但不得提交凭据、私有设备数据或网络信息。

提交修改前，请阅读[贡献指南](CONTRIBUTING_ZH.md)、
[安全策略](SECURITY_ZH.md)和[第三方软件](THIRD_PARTY_ZH.md)。影响硬件的修改
除了通过 CI，还必须提供开发板修订版、来源证据和实体设备验证结果。
