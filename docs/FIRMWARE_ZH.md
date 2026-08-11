# 固件产物

[English](FIRMWARE.md)

源码快照、不可变恢复输入和生成的示例产物有意相互分离：

- `firmware/xiaozhi/` 是标记为 2.2.5 的保留上游源码快照。
- `firmware/factory_firmware/` 包含从原仓库保留的预编译恢复镜像。
- `release-artifacts/` 由示例 CI 生成，永远不会提交。
- 两个保留固件路径均不会被默认仓库工作流修改、构建、打包或验证。

## 不可变恢复镜像清单

| 文件 | 大小 | SHA-256 |
| --- | ---: | --- |
| `01_Fac-v1.0.0.bin` | 15,859,712 字节 | `1DD2BE304BEC964831EEC8484C67C97D7B56934343DEFC4EBCAB63ED84D32C33` |
| `02_xiaozhi-v2.2.5.bin` | 11,075,284 字节 | `7FDDD220C86AD793F360EF88FEC8454C2765CF9C596C76FBD0753D01889AF169` |

这些哈希只记录仓库输入，不是数字签名，也不能证明真实性或硬件适用性。不得在未
记录来源、版本、预期烧录流程、新校验和及硬件验证结果时重命名、重新生成或替换镜像。

恢复镜像只能按[官方产品文档](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)
中的偏移和流程使用。即使当前 XiaoZhi 源码构建成功，也不能证明历史二进制可由其
复现。CI 产物只覆盖示例工程，不覆盖保留固件路径。

## 引导式 CI 产物烧录

`Flash-CI-Firmware.cmd` 会启动 Windows 图形工具，用于依次测试 27 个示例包。它要求
当前分支非 detached、工作区干净，且该分支恰有一个已打开的非草稿 PR；只接受 SHA 与
本地 `HEAD` 完全一致的成功工作流产物。请先安装 Git、已认证的 GitHub CLI（`gh auth login`）、
带 `esptool` 的 Python 以及 USB 串口驱动。`Flash-CI-Firmware.cmd -ListOnly` 可查看计划
顺序，`Flash-CI-Firmware.cmd -SelfTest` 可在不需要设备、认证或图形界面的情况下检查本地
约定。若无法自动识别唯一的 ESP32-C6 USB VID/PID 设备，请使用 `-Port COMx`。

图形工具会在执行 `python -m esptool ... write_flash` 前校验每个压缩包清单、包内文件的
哈希和大小以及烧录范围；它绝不会擦除 flash。写入校验成功后，请先测试开发板，再选择
**Mark PASS and flash next**。进度按最终 SHA 保存到仓库外。示例 CI 产物仅用于诊断测试，
不能替代工厂恢复镜像或工厂烧录流程。
