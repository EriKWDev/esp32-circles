# XPowers 共享组件

[English](README.md)

本目录包含由 ESP-IDF 示例共享的 XPowers 核心。各工程专用的 AXP2101 初始化继续
保留在自己的 `components/pmicpower` 包装组件中，因为不同示例的电源轨启用方式和
任务行为不同。

这些源码由此前每个 ESP-IDF 示例中的相同副本合并而来。仓库也分发匹配的 Arduino
库，并且尚未验证有权威托管替代组件能满足本仓库的 API、许可证、目标芯片和硬件
要求，因此该组件继续保留在本地。

上游项目：<https://github.com/lewisxhe/XPowersLib>

许可证：MIT；请查看源码中的许可证声明及[第三方软件](../../THIRD_PARTY_ZH.md)。
