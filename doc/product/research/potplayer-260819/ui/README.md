# PotPlayer 260819 界面与交互逆向（2026-10-20）

[界面整改文档](../../../ui-alignment.md)的证据来源。对象与上级目录相同（PotPlayer 260819 x64，安装包 SHA-256 `4b9833ad…f9cfe`），只解包、静态分析，没有运行任何 PotPlayer 程序。

| 文件 | 内容 |
| --- | --- |
| A-skin-layout.md | 默认皮肤 `Skins/Default.dsf` 的布局与样式：窗口、标题栏、底部控制栏、播放列表、右键菜单样式、颜色表、控制面板 |
| B-menus-dialogs.md | 右键菜单结构、勾选/单选状态、加速键表、我们 46 条命令到 PotPlayer 的对应、相关对话框的控件坐标（DLU 与像素） |
| C-behavior-defaults.md | 出厂默认值（窗口尺寸、播放列表、音量、快捷键、进度条、OSD）、播放列表交互、自动隐藏、播放信息叠加层 |
| config-defaults.txt | 从 DLL 的保存函数与默认值函数配对得到的约 1200 个配置键出厂值（列：键名、宽度、偏移、保存处地址、默认值） |
| command-symbols.json | DLL 注册的 1264 个命令符号名与 CmdID |

文中的 `$P/...` 路径指分析时的临时工作目录（未入库）。复现：按[功能清单](../../../potplayer-inventory.md) §5 下载并校验安装包，`7zz x` 解包，再 `7zz x Skins/Default.dsf` 得到皮肤文件。可信度标记：确认 / 推断 / 未知；未知项需要 Windows 实机确认。
