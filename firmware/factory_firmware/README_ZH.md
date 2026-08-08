# 出厂与恢复固件

[English](README.md)

本目录包含从原仓库保留的预编译镜像：

| 文件 | 作用 | SHA-256 |
| --- | --- | --- |
| `01_Fac-v1.0.0.bin` | 出厂恢复镜像 | `1DD2BE304BEC964831EEC8484C67C97D7B56934343DEFC4EBCAB63ED84D32C33` |
| `02_xiaozhi-v2.2.5.bin` | 预编译 XiaoZhi 镜像 | `7FDDD220C86AD793F360EF88FEC8454C2765CF9C596C76FBD0753D01889AF169` |

这些文件不在源码构建 CI 范围内。不得在未于发布说明中记录来源、版本、预期烧录
流程、校验和及硬件验证结果时重命名、重新生成或替换它们。

烧录说明请遵循[官方产品文档](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)。
即使 `firmware/xiaozhi/` 构建成功，也不能证明这些历史二进制与其一致或可由其复现。
