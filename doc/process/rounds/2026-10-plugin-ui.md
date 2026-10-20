# 本轮任务：插件交互改版（第二步，自下而上）

分支：`rebuild/plugin-ui`（自 `rebuild/core-ui` 8c1a53e）。流程按[社区工作流](../community-workflow.md) §2、§4（自下而上）。每个作者开工前读完：本文、社区工作流 §1、§2、§4、§8，[界面整改文档](../../product/ui-alignment.md) §2、§6、§8、§9 和与自己插件相关的小节（§11 列出各插件的整改项），[API 参考](../../plugins/reference.md)，[核心界面设计](../../architecture/core-ui-design.md) §2–§12。

## 1. 目标

各插件按整改文档 §11 改版交互与外观，对齐参考播放器。核心第一步已完成，所需公共能力原则上都已具备：绑定表（`bindings`、`focus`、`repeat`）、自由菜单（`path` 可省略、`order`、`group`、`radio`）、主题与控件库（`@vestra/theme.slint`、`@vestra/widgets.slint` 的 Button、Slider、ToolTipArea）、屏幕提示 `plugin.osd.show`、播放序列 `player.next/previous`、面板状态与分离 `plugin.panels.*`、面板右键菜单 `panel.showMenu`。

## 2. 规则

- 每个作者只写自己插件目录。不改核心、不改兄弟插件。
- 按整改文档的目标值做；文档里标「未知」的项用文档给的暂定值。
- 用现有接口做不到时，先自己想插件内的做法；仍做不到，就在报告的「API 请求」一节按社区工作流 §4.1 写请求（附最小复现与希望的写法），**跳过这一项继续做其它项**。协调者转给 API 维护者，维护者从严把关，可能给出现有接口的写法而不改核心。
- 菜单位置与快捷键严格按 §3 的分配表，避免插件之间冲突；需要表外的项，在报告中说明并取表内未用的号段。
- 不用参考播放器的图片素材；图标用 Slint `Path` 自绘。
- 颜色、字号、控件尺寸用 `Theme`，不写死颜色（主题未覆盖的特殊色可在报告中说明）。
- 测试：原有测试覆盖的行为若因改版改变，更新测试；新交互补测试。
- 完成条件：`npm run plugin -- check|build|test <名字>` 全部通过；报告列出完成的整改项编号、未完成项与原因、API 请求。

## 3. 菜单与快捷键分配表（整改文档 §6.2、§8.1）

`group` 与顶层 `order` 骨架：`open` 100–199、`domain` 200–699、`screen` 700–799（宿主全屏 700）、`windows` 800–899、`exit` 900–999（宿主退出 990）。子菜单的位置取它下面最小的 `order`。

| 菜单路径 | 插件 | order | group | 绑定 |
| --- | --- | --- | --- | --- |
| 打开文件… | playback | 100 | open | Ctrl+O、F3 |
| 打开 ▶ YouTube 视频… | youtube | 111 | open | |
| 打开 ▶ Bilibili 视频… | bilibili | 112 | open | |
| 打开 ▶ 打开字幕… | subtitles | 113 | open | Alt+O |
| 关闭 | playback | 120 | open | F4 |
| 播放 ▶ 播放/暂停 | playback | 201 | domain | Space |
| 播放 ▶ 上一个 / 下一个（`player.previous/next`） | playback | 202 / 203 | domain | PageUp、Mouse:X1 / PageDown、Mouse:X2 |
| 播放 ▶ 跳转 ▶ 后退5秒 / 前进5秒 | playback | 210 / 211 | domain | ArrowLeft / ArrowRight（`repeat`） |
| 播放 ▶ 跳转 ▶ 后退/前进 30 秒、60 秒、300 秒 | playback | 212–217 | domain | Ctrl / Shift / Ctrl+Alt + ←/→ |
| 播放 ▶ 跳转到… | playback | 218 | domain | G |
| 播放 ▶ 速度 ▶ 正常 / 减速 0.1 / 加速 0.1 | playback | 220–222 | domain | Z / X / C |
| 播放 ▶ 速度 ▶ 0.5 / 1.5 / 2 倍（`radio`）、自定义… | playback | 223–226 | domain | |
| 播放 ▶ 播放列表 ▶ 添加文件… / 添加链接… / 循环 ▶（radio）/ 随机 | playlist | 230–249 | domain | |
| 字幕 ▶ 显示字幕（勾选） | subtitles | 301 | domain | Alt+H |
| 字幕 ▶ 选择字幕轨 ▶ / 同步 超前 / 滞后 / 复位 / 刷新 / 字幕设置… | subtitles | 302–319 | domain | `>` / `<` / `/` |
| 视频 ▶ 画面调节… / 复位 / 亮度 ±1 / 对比度 / 饱和度 / 高斯模糊 | video | 401–419 | domain | Q、W/E、R/T、Y/U，高斯 Ctrl+B |
| 视频 ▶ 截图 ▶ 保存当前画面 / 当前画面另存为… / 连拍… / 导出片段… / 导出任务… / 截图文件夹… | capture | 420–439 | domain | 保存 Ctrl+E，连拍 Ctrl+G |
| 声音 ▶ 音量 + / 音量 − / 静音（勾选） | playback | 501–503 | domain | ArrowUp、Mouse:WheelUp / ArrowDown、Mouse:WheelDown（`repeat`）/ M |
| 声音 ▶ 音频工作台… / 均衡器（勾选）/ 预设 ▶（radio）/ 复位 | audio | 510–529 | domain | 均衡器 Shift+E |
| 模拟器 ▶ … | nes | 601–619 | domain | |
| 全屏 | 宿主 | 700 | screen | Enter、F11、双击 |
| 播放列表（勾选） | playlist | 800 | windows | F6 |
| 播放信息（勾选） | info | 810 | windows | Tab |
| 插件工坊… | workshop | 820 | windows | |
| 退出 | 宿主 | 990 | exit | Cmd+Q |

删除（整改文档已定）：「视图 → 显示播放栏」（UI-MENU-03）。「文件」「视图」不再作为顶层子菜单。

播放列表内部（`focus: "panel"`，可省略 `path` 只进列表右键菜单）：Enter / P 播放、Delete 移除所选、Ctrl+↑/↓/Home/End 移动、Ctrl+A 全选。

## 4. 分派

| 批次 | 作者 | 插件 | 整改项（整改文档 §11） |
| --- | --- | --- | --- |
| 1 | A | playback | UI-BAR-01–11、UI-MENU-01/03/05、UI-DLG-01、§3 中归 playback 的菜单与按键、UI-OSD-02 |
| 1 | B | playlist | UI-PL-01/02/04–20、UI-MENU-01、UI-OSD-02 |
| 1 | C | video、audio | UI-TOOL-01/03–07/09、UI-MENU-01、UI-OSD-02 |
| 1 | D | subtitles、capture | UI-TOOL-01/03–05/08/09、UI-MENU-01/04、UI-DLG-03–05、UI-OSD-02 |
| 2 | E | youtube、bilibili、nes、workshop、info | UI-TOOL-01/05、UI-DLG-02/06、UI-MENU-01、UI-INFO 外观 |

## 5. 验收（协调者）

全部插件完成后：静态检查、全部插件测试、菜单快照（与 §3 对照）、全量 GUI 测试（社区工作流 §6.2 加整改文档各项「验收」列），然后提交推送。

## 6. 记录
