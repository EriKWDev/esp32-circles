# 仓库迁移

[English](MIGRATION.md)

目录结构已经调整，但示例用途和数字顺序没有改变。

| 原路径 | 当前路径 |
| --- | --- |
| `01_Arduino_Libraries/` | `examples/arduino/libraries/` |
| `02_Example/Arduino-v3.3.3/` | `examples/arduino/`（9 个直接草图目录） |
| `02_Example/ESP-IDF-v5.5.3/` | `examples/esp-idf/` |
| `02_Example/XiaoZhi-v2.2.5/` | `firmware/xiaozhi/` |
| `03_Firmware/` | `firmware/factory_firmware/` |
| `Tools Configuration.png` | `docs/assets/arduino-tools-configuration.png` |

ESP-IDF 工程现在使用共享的 `waveshare/esp32_c6_touch_amoled_2_16` BSP，不再编译
各自的 I2C、PMU、显示、触控、存储和音频开发板包装。验证期间，每个工程清单都固定
到 BSP PR #185 的精确提交。只有该 PR 合并、BSP 正式发布且两个 CI 矩阵都通过后，
Git 依赖才会替换为正式注册表版本。产品应用逻辑、托管传感器依赖和共享状态 UI
继续保留在本仓库。

构建脚本和文档必须使用当前路径。CI 路由保留旧路径映射，使大规模迁移差异可以
正确分类，不会把旧产品文件视为未知输入。删除或重命名示例路径仍会触发对应框架
验证。

产品维护的 `libraries/C6_AMOLED_BSP` 集中维护 C6 引脚、I2C、AXP2101、SH8601、
触控和 LVGL 8/9 集成。CI 继续只为每个草图暂存其所需的固定依赖。

生成的发布压缩包应位于 `release-artifacts/`，不属于源文件。保留的 XiaoZhi 目录和
出厂二进制继续保持各自来源边界，并处于默认示例 CI 范围之外。
