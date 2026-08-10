# 项目结构

[English](PROJECT_STRUCTURE.md)

本仓库属于混合型开发板支持仓库：

1. 相互独立的 ESP-IDF 示例；
2. 使用固定本地库的独立 Arduino 草图；
3. 共享托管 BSP 与产品级应用适配组件；
4. 功能丰富且保留来源的固件源码目录；
5. 与源码构建产物来源不同的恢复二进制。

它不是 ESP32-P4 加 Hosted 无线仓库。`firmware/xiaozhi/` 下的 P4 和 Hosted 传输
引用属于上游多开发板固件源码，不描述本产品。

## 所有权边界

| 路径 | 维护方式 | 说明 |
| --- | --- | --- |
| `examples/esp-idf/<name>` | 独立工程 | 必须能在不依赖其他示例的情况下配置和构建 |
| `examples/arduino/<name>` | 独立草图 | 9 个第一方草图直接位于 Arduino 根目录；CI 只暂存该草图所需的固定依赖 |
| `examples/arduino/libraries/C6_AMOLED_BSP` | 第一方开发板包装 | 集中维护 C6 引脚、I2C、AXP2101、SH8601、触控和 LVGL 8/9 集成 |
| `examples/arduino/libraries` | Arduino 依赖快照 | 上游库继续随仓库保存，LVGL 主版本保持隔离 |
| `examples/esp-idf/common/components/status_ui` | 产品应用组件 | 使用托管 BSP 的显示生命周期渲染示例状态 |
| ESP-IDF 工程清单 | 托管开发板依赖 | 验证期间 9 个工程固定到同一个已评审的 `esp32_c6_touch_amoled_2_16` BSP 提交 |
| `firmware/xiaozhi` | 保留的上游源码 | 不由仓库 CI 修改或验证 |
| `firmware/factory_firmware` | 不可变恢复输入 | 不由源码 CI 生成或验证 |
| `.github/policy` | 第一方策略 | 路由和 Markdown 所有权声明 |
| `.github/scripts/repository_audit` | 第一方审计工具 | 清单、Markdown、隐私和 CI 路由检查 |
| `release-artifacts` | 生成输出 | 永远不提交 |

## 依赖策略

当存在权威且兼容的上游组件时，显示、触控、传感器、编解码器和总线组件应优先
使用 ESP Component Registry。若 API、许可证、目标芯片、所有权或硬件等效性证据
不完整，则保留本地代码。

ESP-IDF 示例通过微雪 BSP 仓库提供的板级 API 使用 I2C、PMU、显示、触控、存储和
音频功能。评审期间，每个独立工程都固定到 BSP PR #185 的同一精确提交。只有 BSP
完成合并和发布、且两个仓库的 CI 都通过后，才会把提交固定改为正式注册表版本。
传感器专用应用逻辑和共享状态页继续保留在本产品仓库。

除产品维护的 `C6_AMOLED_BSP` Arduino 包装外，嵌入的 Arduino 库和 XiaoZhi 快照
继续作为保留来源的上游内容，不应被顺带当作第一方代码进行整理。

## 验证边界

仓库本身不包含产品原理图。现有引脚定义被原样保留，并非重新推导；当前示例配置
16 MB Flash，但不能据此证明支持 PSRAM。官方原理图链接及修改硬件映射前需要的
证据见[硬件验证](HARDWARE_VALIDATION_ZH.md)。
