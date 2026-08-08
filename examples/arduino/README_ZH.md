# Arduino 示例

[English](README.md)

第一方草图位于 `examples/`，固定版本的库快照位于 `libraries/`。发现脚本只检查
`examples/` 下包含 `.ino` 文件的直接子目录，因此嵌入库自带的示例永远不会进入
产品矩阵。

CI 构建脚本为全部草图提供 XPowersLib，只为 `08_LVGL_V8_Test` 提供 LVGL 8，
只为 `07_Audio_Test` 和 `09_LVGL_V9_Test` 提供 LVGL 9，防止不兼容的主版本同时
出现在编译器搜索路径中。

固定的开发板配置为 ESP32-C6、启动时启用 CDC、16 MB Flash、80 MHz QIO、
160 MHz CPU，以及 3 MB 应用程序/9 MB FAT 分区方案。工作流当前验证
Arduino-ESP32 3.3.11，但不配置或证明支持 PSRAM。

Arduino IDE 设置见[仓库首页](../../README_ZH.md)和
[官方产品文档](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)。
