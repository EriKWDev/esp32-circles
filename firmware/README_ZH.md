# 固件

[English](README.md)

| 路径 | 用途 |
| --- | --- |
| `xiaozhi/` | 标记为 2.2.5 的保留上游 XiaoZhi 源码快照 |
| `factory_firmware/` | 产品仓库随附的不可变预编译恢复镜像 |

两个路径来源不同，并按原样保留。仓库默认 CI 不会修改、编译、打包或验证它们。
其中的文件不能与生成的示例产物互换。

校验和和来源规则见[固件产物](../docs/FIRMWARE_ZH.md)，不可变清单见
[出厂与恢复固件](factory_firmware/README_ZH.md)，恢复刷写说明见
[官方产品文档](https://docs.waveshare.com/ESP32-C6-Touch-AMOLED-2.16)。
