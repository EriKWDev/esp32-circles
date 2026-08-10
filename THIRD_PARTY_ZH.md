# 第三方软件

[English](THIRD_PARTY.md)

本仓库包含第三方源码快照和组件。各自目录中的许可证仍为适用条款的权威来源。

`examples/arduino/libraries/C6_AMOLED_BSP` 是第一方开发板包装，并非第三方软件。
它集中维护直接位于 Arduino 根目录的草图所使用的 C6 引脚、I2C、AXP2101、SH8601、
触控和 LVGL 8/9 集成。

| 软件 | 位置 | 版本或来源 | 许可证位置 |
| --- | --- | --- | --- |
| XPowersLib | `examples/arduino/libraries/XPowersLib` | 0.3.3，Lewis He | `examples/arduino/libraries/XPowersLib/LICENSE` |
| LVGL | `examples/arduino/libraries/lvgl8/lvgl` | 8.4.0 | `examples/arduino/libraries/lvgl8/lvgl/LICENCE.txt` |
| LVGL | `examples/arduino/libraries/lvgl9/lvgl` | 9.3.0 | `examples/arduino/libraries/lvgl9/lvgl/LICENCE.txt` |
| XiaoZhi | `firmware/xiaozhi` | 标记为 2.2.5 的仓库快照 | `firmware/xiaozhi/LICENSE` |
| ESP Codec Device 快照 | 音频示例 | 嵌入源码 | 嵌入源码旁的许可证文件 |

托管的 ESP-IDF 依赖由各工程的 `idf_component.yml` 解析；精确版本和许可证请以
生成的依赖元数据及上游组件记录为准。没有 API、许可证、目标芯片和硬件验证
证据时，不得仅凭组件名称相似就推断注册表组件可以直接替换本地实现。

BSP 评审期间，`waveshare/esp32_c6_touch_amoled_2_16` 从微雪组件仓库的单一精确
PR 提交获取。在依赖切换到正式注册表版本前，其上游组件清单和许可证为权威依据。
