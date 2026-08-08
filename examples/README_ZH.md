# 示例

[English](README.md)

仓库为 ESP-IDF 和 Arduino 提供并行学习路径：

- [`esp-idf/`](esp-idf/README_ZH.md) 包含 9 个相互独立的工程，由 CI 中固定的
  ESP-IDF 精确维护版本验证。
- [`arduino/examples/`](arduino/README_ZH.md) 包含 9 个相互独立的草图，使用仓库
  固定的 Arduino-ESP32 Core 验证。

数字前缀按从单项外设检查到显示、音频和 UI 集成的顺序排列示例，并不表示构建
依赖。嵌入库自带的示例属于上游内容，不会被发现为产品示例。

每个产品示例均面向 ESP32-C6，并使用仓库记录的 16 MB Flash 布局。编译成功
不能用于推断支持 PSRAM，也不能证明实体外设已通过验证。
