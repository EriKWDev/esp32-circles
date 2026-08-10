# ESP-IDF 示例

[English](README.md)

每个子目录都是面向 ESP32-C6 的独立 ESP-IDF 工程。工程对适合的上游驱动使用
托管组件，并通过仓库共享组件 `components/xpowers` 复用通用 PMU 代码。

01–06 示例现已将 PMIC、传感器、SD 卡或 Wi-Fi 状态发布到设备屏幕，同时保留原有
串口日志。对于 SD 卡和 Wi-Fi 示例，屏幕初始化是可选的；即使显示初始化失败，核心
演示流程仍会继续运行。

请从单个工程目录中构建：

```sh
idf.py set-target esp32c6
idf.py build
idf.py flash monitor
```

CI 要求直接子目录同时包含 `CMakeLists.txt` 和 `main/` 才会发现为工程。符合该
结构的新示例会自动进入矩阵。工作流使用 ESP-IDF v5.5.5 和 v6.0.2 验证所有已发现
工程，并为每个成功构建打包精确的二进制和烧录元数据。

当前全部工程都通过 `sdkconfig.defaults` 选择 ESP32-C6 和 16 MB Flash，没有工程
启用 PSRAM。编译不能证明现有 GPIO 分配或连接外设与特定 PCB 修订版一致。
