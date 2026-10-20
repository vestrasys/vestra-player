# PotPlayer 260819 静态盘点数据

日期：2026-10-17。阅读主文档：[PotPlayer 功能清单](../../potplayer-inventory.md)。本目录不含 Windows 可执行程序或完整语言包。

| 文件 | 用途 |
| --- | --- |
| metadata.json | 安装包 URL、版本、文件哈希、计数和覆盖边界 |
| resource-map.json | REA 主 DLL 资源检查结果摘录，保留证据 ID、MENU/VERSION 位置和 payload 哈希 |
| menus.json | 1,043 个静态菜单节点，包括分隔符；实际 CmdID、原模板文字、双语标签、快捷键、路径和语言文件行号 |
| menu-tree.md | 全部非分隔节点的中英文可读菜单树 |
| commands.json | 按实际 CmdID 收集的菜单路径，用于发现多入口复用 |
| dialogs.json | 169 组对话框的本地化文本记录，按 section/control 定位 |
| strings.json | 字符串表记录，包含静态菜单之外的操作名称和状态文本 |
| features.json | 人工归纳的 106 项功能，引用 M/D/S 证据，候选功能组不是架构决定 |
| extract_inventory.py | 校验固定样本身份后生成原始索引及菜单树；不执行目标程序 |

`template_text` 是原 DLL 菜单模板文字，可能与中英文翻译不同；`en`、`zh` 来自同发行包语言文件，不能将某一翻译当作运行时行为证据。`line_en`、`line_zh` 是相应语言文件中的 1-based 行号。

菜单 `key` 表示模板结构路径，不是命令 ID；popup 的 `command_id` 为 null。当前有一个无对应本地化记录的非分隔占位节点，另有四条 DVD 子菜单的翻译记录为空，均保留原状。CmdID 的意义只绑定到此版本，不能假定其他版本不变。

长于 240 字符的说明文字不复制全文，而保留长度与 UTF-8 文本哈希；哈希不是原文件字节片段的哈希。源文件和 PE payload 的原始字节哈希另行记录。

这些数据是静态资源证据。尚未在 Windows 运行核对动态菜单、硬件/账号条件、服务可用性或每项功能实现。完整复现命令见主文档。
