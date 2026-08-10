# 项目结构

[English](PROJECT_STRUCTURE.md)

本仓库属于混合型开发板支持仓库：

1. 相互独立的 ESP-IDF 示例；
2. 使用固定本地库的独立 Arduino 草图；
3. 可复用开发板组件与示例专用策略包装组件；
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
| `components/xpowers` | 共享本地组件 | 仅含通用源码，不含开发板专用电源策略 |
| `examples/esp-idf/*/components/pmicpower` | 示例策略包装 | 启动延时和外设策略保留在本地 |
| `firmware/xiaozhi` | 保留的上游源码 | 不由仓库 CI 修改或验证 |
| `firmware/factory_firmware` | 不可变恢复输入 | 不由源码 CI 生成或验证 |
| `.github/policy` | 第一方策略 | 路由和 Markdown 所有权声明 |
| `.github/scripts/repository_audit` | 第一方审计工具 | 清单、Markdown、隐私和 CI 路由检查 |
| `release-artifacts` | 生成输出 | 永远不提交 |

## 依赖策略

当存在权威且兼容的上游组件时，显示、触控、传感器、编解码器和总线组件应优先
使用 ESP Component Registry。若 API、许可证、目标芯片、所有权或硬件等效性证据
不完整，则保留本地代码。

此前每个 ESP-IDF 示例都复制了 XPowers 源码。相同核心现已放入
`components/xpowers`；各示例继续保留自己的 `power_bsp.cpp`，因为这些包装表达
示例专用行为。除产品维护的 `C6_AMOLED_BSP` 包装外，嵌入的 Arduino 库和 XiaoZhi
快照继续作为保留来源的上游内容，不应被顺带当作第一方代码进行整理。

## 验证边界

仓库本身不包含产品原理图。现有引脚定义被原样保留，并非重新推导；当前示例配置
16 MB Flash，但不能据此证明支持 PSRAM。官方原理图链接及修改硬件映射前需要的
证据见[硬件验证](HARDWARE_VALIDATION_ZH.md)。
