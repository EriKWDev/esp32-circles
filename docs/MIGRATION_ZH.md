# 仓库迁移

[English](MIGRATION.md)

目录结构已经调整，但示例用途和数字顺序没有改变。

| 原路径 | 当前路径 |
| --- | --- |
| `01_Arduino_Libraries/` | `examples/arduino/libraries/` |
| `02_Example/Arduino-v3.3.3/` | `examples/arduino/examples/` |
| `02_Example/ESP-IDF-v5.5.3/` | `examples/esp-idf/` |
| `02_Example/XiaoZhi-v2.2.5/` | `firmware/xiaozhi/` |
| `03_Firmware/` | `firmware/factory_firmware/` |
| `Tools Configuration.png` | `docs/assets/arduino-tools-configuration.png` |

ESP-IDF 工程现在通过 `EXTRA_COMPONENT_DIRS` 加入仓库级 `components/` 目录。
各工程的 PMIC 包装组件依赖共享 `xpowers`，不再编译同一核心的私有副本；示例专用
`power_bsp.cpp` 策略仍保留在本地。

构建脚本和文档必须使用当前路径。CI 路由保留旧路径映射，使大规模迁移差异可以
正确分类，不会把旧产品文件视为未知输入。删除或重命名示例路径仍会触发对应框架
验证。

生成的发布压缩包应位于 `release-artifacts/`，不属于源文件。保留的 XiaoZhi 目录和
出厂二进制继续保持各自来源边界，并处于默认示例 CI 范围之外。
