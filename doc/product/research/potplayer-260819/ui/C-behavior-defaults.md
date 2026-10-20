# C：交互行为与默认值（PotPlayer 260819 x64，静态分析）

分析者：agent C。范围是行为，不含像素布局（布局见 A/B）。全程只读文件，没有执行任何 PotPlayer 可执行文件。

## 0. 方法与证据约定

- `$P` = `/private/tmp/claude-501/-Users-ziyang-Develop-vestra-player/7b0dd90c-cb6e-48be-9332-6cfbc6f60a9d/scratchpad/potplayer`
- `EN:<行>` 指 `$P/x/Language/English.ini` 的行号；`M:<key>` 指 `doc/product/research/potplayer-260819/menus.json` 的节点 key（带 CmdID 和快捷键）；`skin:<文件>:<行>` 指 `$P/skin-default/` 下的文件。
- **默认值的来源（关键方法）**：`PotPlayer64.dll` 的 .data 段 0x181a21050 起有一张配置键名指针表（UTF-16 键名，如 `PlayListWindowVisible`）。保存函数 `0x180593be0`（长 0x45d16）对每个键做“当前值 ≠ 默认对象的值才写入”。默认对象由 `SetDefaults` 函数 `0x18057d820` 填充（参数 bl≠0 时整体重置）。我按“保存函数里的键名 → 字段偏移 → SetDefaults 中对该偏移的立即数写入”逐一配对，得出约 1200 个键的出厂默认值。工作表：`$P/cwork/keydefaults.txt`，列为 `键名 宽度 偏移 保存处地址 默认值`。反汇编用 macOS 自带 llvm `objdump`，只做读取。
- 可信度：**确认** = 资源、皮肤、菜单直接给出；**推断** = 由 SetDefaults 立即数加上“取值→选项”的对应关系推出（写出了推理）；**未知** = 静态无法确定。
- SetDefaults 的值是“注册表或 ini 中没有该键时”的出厂值。没有核实安装器是否另写 ini，这一点在未知项 U1 中列出。

---

## 1. 摘要（10 条最重要结论）

1. **首次运行窗口**：主窗口外框位置 (102,143)，尺寸 **780×671 px**；`MainWindowState=0`（普通窗口，非最大化）。播放列表默认**显示**（`PlayListWindowVisible=1`），并且**停靠在主窗口内的右侧**（`PlaylistAttachToMain2=1`）。浮动时的默认矩形是 (438,100) 260×408。〔推断，依据是 SetDefaults 立即数〕
2. **窗口模式下控制栏在视频下方，不覆盖视频**：`MainSkinArea` 中视频区为 `Align='Client'`，底部两条控制条为 `Align='Bottom'`（高 40+22 px）。`AutoHideSkin=0`（“视频区活动时自动隐藏主窗口皮肤”默认关闭）。OSC（叠加在视频上的屏幕控制）`SkinUseOsc=0`，默认关闭。〔确认（皮肤）+推断（默认值）〕
3. **默认音量 100%**，步进 5%，默认不静音。“启动音量”功能默认关闭，即沿用上次音量。〔推断〕
4. **播放结束**：重复播放默认关（`RepeatPlay2=0`），随机播放关（`RandomPlay=0`），“播放列表结束后关闭播放”关。也就是按顺序播放到列表末尾后停止。〔推断〕
5. **快捷键**（菜单模板直接给出）：Space 播放/暂停，←/→ 跳 5 s，Ctrl+←/→ 跳 30 s，Shift+←/→ 跳 60 s，Ctrl+Alt+←/→ 跳 300 s，↑/↓ 调音量，M 静音，Enter 或 Alt+Enter 全屏，Ctrl+O 打开文件，Ctrl+U 打开 URL，F6 播放列表开关，F7 控制面板，F5 首选项，Tab 完整播放信息，Shift+Tab 简短信息，Ctrl+E 保存帧，Ctrl+C 复制帧，Z/X/C 调倍速，Q/W/E/R/T/Y/U/I/O 调色彩。〔确认〕跳转秒数取自 `PlayMoveTime[0..3]={5,30,60,300}`。〔推断：键与秒数的对应按 Dialog(364) 中四行的顺序〕
6. **控制栏左侧两个跳转按钮是双功能**：左键点击跳 5 s，右键点击播放上一个/下一个文件（皮肤 `AltCmdID='ID_PROCESS_PREV/NEXT'`，日文 tooltip 写明“右クリック：前のメディアを再生”）。〔确认〕
7. **时间显示**：默认显示已播放时间（`SkinCurrentLeftTime=0`）。当前时间标签可以点击（`Cursor='HAND' Enabled='1'`），推断点击后在“已播放/剩余”之间切换。格式为“当前 / 总长”。〔确认（皮肤）+推断（切换行为）〕
8. **进度条**：默认不在拖动时暂停（`SeekbarPauseResume=0`），拖动时的时间提示与缩略图预览**默认关闭**（`PreviewSeekTime=0`、`PreviewThumbnail=0`），书签/章节标记默认显示（`DisplayBookmark=1`）。〔推断〕
9. **OSD**：常规提示消息默认开启（`MessageVisible=1`），覆盖跳转、音量、静音、倍速、暂停/继续、色彩、字幕同步、截图保存等。各条文案见 §5。Tab 打开的完整播放信息默认关闭，是渲染器统计叠加层，可能的字段见 §5.3。〔确认（文案）+推断（字段）〕
10. **工具窗口**：画面调节、均衡器、字幕微调、播放控制合在一个**独立的“控制面板”窗口**里（F7，`ID_CONTROL_BOX_WINDOW`，默认不显示，四个页签：音频/视频/字幕/播放）。这是带滑块的非模态皮肤窗口，改动立即生效，每个色彩滑块旁有独立的“Default”复位按钮。首选项（F5）里另有同类设置页。〔确认（皮肤）+推断（立即生效）〕

---

## 2. 默认值（首次运行）

### 2.1 主窗口与视频区

```css
/* 出厂默认值：SetDefaults @0x18057d820，bl≠0 分支 */
.main-window {
  left: 102px;  top: 143px;          /* MainX/MainY：0x18057d890 movl $0x66,0x140；0x18057d929 movl $0x8f,0x144 */
  width: 780px; height: 671px;       /* MainWidth2/MainHeight2：0x18057db4b movl $0x30c,0x148；0x18057db55 movl $0x29f,0x14c（覆盖了先前写入的 336×418） */
  state: normal;                     /* MainWindowState=0（0x158） */
  /* VideoWindowWidth/Height = 400×300（0x18057d90d/0x18057d917），推断为视频区的参考尺寸 */
}
.window-size-on-playback-start { mode: "1.0x video output size"; } /* PlayScreenSize=2（0x4e8）*/
.start-centered { value: off; }     /* PlayScreenMoveCenter=0；StartCenterPos=0 */
.stay-on-top { mode: "while playing in fullscreen"; } /* OnTopMode=3（0x4e4，0x18057e56d） */
.snap-to-edge { value: on; }        /* AutoAlignWindow=1（0x2405，0x180580a7b movl $0x10100,0x2404） */
```

| 项 | 默认 | 证据 | 可信度 |
|---|---|---|---|
| 窗口外框位置/尺寸 | (102,143) 780×671 | 见上方地址；键名 `MainX/MainY/MainWidth2/MainHeight2`，保存处 0x18059464c… | 推断（单位按 1x 物理像素理解；DPI 缩放是否另乘系数未知，见 U2） |
| 开播时窗口大小 | 1.0x 视频输出尺寸 | `PlayScreenSize=2`；“Default window size”下拉框的填充顺序在 0x18017eab6 起依次为字符串 1398 Do not use、1399 0.5x、1400 1.0x、1401 1.5x、1402 2.0x、1816–1818 全屏三种、3490 Custom、4035 Fit to AR、4126 Maximize（CB_ADDSTRING=0x143），索引 2 对应 1.0x | 推断 |
| 置顶 | 仅全屏播放时置顶 | `OnTopMode=3`；0x180338b30 处代码：取值 0=Never（字符串 1593/1090），1=Always（1088），2=While playing（1089），3=While playing in fullscreen（4561，对应 CmdID 0x2ad8=10968） | 推断（取值映射已确认） |
| 宽高比 | 默认显示 AR，保持宽高比 | 菜单 `M:115_0_7_5` “Default Display AR (Recommended)”；默认值 `ScreenRatio=0`、`FullScreenFitMode=0` | 推断 |
| 启动后自动播放上次文件 | 关 | `AutoPlayOnStart=0`（0x18057e5c4）；对应 Dialog(457) “Play the last played item” EN:1726 附近 | 推断 |
| 打开文件时 | 把同类文件一并加入列表 | `OpenWithSameName=1`；菜单 `M:101_0_15_5_0..2` 依次为 only selected / similar / all，取值 1 = similar | 推断 |

### 2.2 控制栏位置（窗口模式与全屏）

- 皮肤定义了三种区域：`MainSkinAreaOscFrame`、`MainSkinAreaOscInside`、`MainSkinArea`（skin:VideoSkin.xml:7,29,50）。普通窗口用 `MainSkinArea`，其中视频区为 `<Static Align='Client' Contain='CMD_VIDEO_WINDOW'/>`（VideoSkin.xml:83），底部控制条 `BottomVideo.txt` 用 `Align='Bottom' Height='40'` 和 `Height='22'`（BottomVideo.txt:1,89）。**所以窗口模式下控制栏在视频下方，挤占窗口高度，不覆盖视频。**〔确认〕
- `MainSkinAreaOscInside` 是 OSC 模式：控制条叠加在视频内（只包含 TopCommon/BottomVideo，没有边框）。OSC 默认关闭：`SkinUseOsc=0`（0x180580b8b，r12d=0），对应菜单 `M:101_0_14_5_0..2` Disable / Direct3D9 / Layered Window。〔推断〕
- “视频区活动时自动隐藏主窗口皮肤”默认关：`AutoHideSkin=0`（0x180580491）；菜单 `M:101_0_14_7` CmdID 10369。〔推断〕
- 全屏：`AutoResizeFullScreen=0`（“根据全屏弹出层自动缩放视频”关，EN:1007 / StringTable 1815）。推断全屏时控制栏以弹出层形式覆盖在视频上，视频不缩小。`AttachOpenFullScreen=0`（Dialog(456) “Pop up the docked window pane even if it's closed in windowed mode” 关，EN:1988）。〔推断〕
- OSC 开启时，“鼠标悬停在 OSC 上时不自动隐藏”默认开：`PersistOsc=1`（0x180580b92；Dialog(434) 3034 EN:1746）。〔推断〕

### 2.3 音量

```css
.volume {
  initial: 100%;          /* AudioVolume=100：0x18057e242 movl $0x64,0x2e8 */
  muted: false;           /* AudioMute=0：0x18057e472 */
  step: 5%;               /* AudioVolumeUnit=5：0x180580748 */
  max-ui-value: 100;      /* AudioVolumeMaxVal=100（0x2278）；另有 AudioVolumeMax=1000（0x2274，推断为放大上限，单位不明） */
  startup-volume: off;    /* AudioVolumeStart=0，sAudioVolumeStartValue=100：关闭时沿用上次音量 */
  control: "default (software)"; /* AudioVolumeControl=0 → 菜单 M:101_0_12_8_0 “Default volume control (Recommended)” */
}
```
可信度：推断（立即数确认，语义按键名、对话框标签 EN:1338 “Startup volume”、EN:1357 “Volume step:” 对应）。

### 2.4 播放列表默认值

| 项 | 默认 | 键（偏移，SetDefaults 地址） |
|---|---|---|
| 列表可见 | 是 | `PlayListWindowVisible=1`（0x1d8，0x18057db87） |
| 停靠到主窗口 | 是 | `PlaylistAttachToMain2=1`（0x130e，0x18057ed95 movl $0x1010101,0x130c）；Chat、Broadcast 同样默认停靠 |
| 浮动矩形 | (438,100) 260×408 | `PlayListX/Y/Width/Height`（0x18057d98b..0x18057d9a9） |
| 记住默认播放列表 | 是 | `SavePlayList=1`（0x18057f473） |
| 显示序号/时长/图标/扩展名/tooltip | 是/是/是/是/是 | `DisplayIdxPL/DisplayTimePL/DisplayIconPL/DisplayExtPL/DisplayTooltipPL=1`（0x18057f47c、0x18057f48d、0x18057f496） |
| 打开新文件时追加而非清空 | 否（会清空） | `AppendFilePL=0`（0x18057f4bc；Dialog(373) 3119 EN:926） |
| 不添加重复文件 | 否 | `NoSameFileAddPL=0` |
| 添加后检测时长 | 否 | `AutoDetectTimePL=0` |
| 手动播放后焦点回到主窗口 | 否 | `FocusSetPL=0`（Dialog(373) 3127） |
| 拖放插入到落点 | 否 | `DropMousePos=0`（Dialog(373) 3118） |
| 排序方式 | 0 | `SortModePL=0` |
| 查看方式 | 值 1（含义不明，见 U8） | `DisplayAsThumbnail=1`（0x18057f4ab） |
| 列表字体 | 9pt，字重 400 | `sPlayListFontSize=9`、`sPlayListFontWeight=400` |

可信度：推断（立即数确认，语义按 Dialog(373) 标签 EN:914-943 对应）。

---

## 3. 播放列表面板交互

### 3.1 打开与关闭

| 方式 | 内容 | 证据 | 可信度 |
|---|---|---|---|
| 菜单 | 右键菜单“Playlist...”，F6，CmdID 10011 | `M:101_0_24` | 确认 |
| 快捷键 | F6（主窗口）；列表右键菜单里“Close Playlist”也是 F6 | `M:111_0_17` | 确认 |
| 控制栏按钮 | 底部右侧 `ID_PLAYLIST_WINDOW`，23×22，`DownFlag='1'`（按下态表示开启） | skin:BottomVideo.txt:64 | 确认 |
| 停靠面板总开关 | `CMD_ATTACH_WINDOW_OPEN/CLOSE`（“打开/关闭停靠面板（聊天/直播列表/播放列表）”），41×40，位于最右 | BottomVideo.txt:59-60；StringTable.txt:376-377、593-594 | 确认 |
| 面板标题栏关闭按钮 | 浮动列表标题栏右侧 14×13 关闭按钮，执行 `ID_PLAYLIST_WINDOW` | skin:SubCommon.txt:573 | 确认 |
| 命令处理函数 | MFC 消息映射：10011 → 0x1803ac810 | DLL 0x1375ff0 | 确认 |

OSD 提示：`1044=Playlist Window : %s`（EN:3425）。〔确认〕

### 3.2 停靠与浮动

- 停靠形态：主窗口右侧容器 `CMD_CHAT_PLAYLIST_BROADCAST_TOGGLE`，有页签条（Broadcast list / Chat / Playlist），每个页签右上角有 6×5 的“分离”按钮（`CMD_ATTACH_PLAYLIST`，tooltip `$CMD_DETACH_PLAYLIST`）（skin:RightAttach.txt:1-21）。与视频区之间有 3 px 分隔条：`IsSplitter='1' MinSplitterSizeLT='318' MinSplitterSizeRB='317'`（RightAttach.txt:23），即视频侧至少 318 px，面板至少 317 px，可拖动调整。〔确认〕
- 浮动形态：独立的 `PlaylistSkinArea` 窗口，最小 300×250（`WidthLimit/HeightLimit`）。标题栏依次为：透明度滑块（`CMD_TRANS_PL`）、“附着到主窗口”按钮（`CMD_ATTACH_PLAYLIST`，tooltip `$CMD_ATTACH_MAIN`）、置顶切换（`CMD_TOPMOST_WINDOW`）、关闭（SubCommon.txt:549-581）。〔确认〕
- 吸附：`UseSideWindow=1`、`AutoAlignWindow=1`（贴边吸附），推断浮动列表会吸附主窗口边缘。〔推断〕
- 状态记忆：可见性、停靠状态、浮动矩形都作为配置键持久化（`PlayListWindowVisible`、`PlaylistAttachToMain2`、`PlayListX..Height`）。保存函数只在值与默认不同时写入，说明会记住上次的状态。〔推断〕

### 3.3 条目操作（列表右键菜单，MENU 111 “PL”）

只在列表获得焦点时生效的快捷键（全部来自 `M:111_*`，〔确认〕）：

| 操作 | 快捷键 | CmdID |
|---|---|---|
| Play | P | 10199 |
| Play within a new instance | — | 10792 |
| Album: Add/Edit/Delete | Ctrl+N / Ctrl+R / Ctrl+D | 10002/10003/10004 |
| Open playlist / Save playlist as | F3 / F2 | 10000/10206 |
| Add to playback queue / Clear queue | Q / Ctrl+Q | 10005/10006 |
| Add file(s) / Add folder / Add URL | Ctrl+I / Ctrl+O / Ctrl+U | 10196/10197/10198 |
| Edit title / Open file location / Search / Rename | Ctrl+E / Ctrl+F / Ctrl+S / Ctrl+M | 10211/10207/10490/10583 |
| Copy / Paste / Paste recent | Ctrl+C / Ctrl+V / Ctrl+Alt+V | 10007/10008/10948 |
| **Remove selected items** | **Del** | 10200 |
| Remove unselected / all / nonexistent | Ctrl+Del / Alt+Del / Ctrl+T | 10212/10201/10210 |
| Delete files（从磁盘删除） | Shift+Del；连同字幕/书签 Shift+Ctrl+Del | 10213/10209 |
| Sort: 升序/降序/标题/文件名/扩展名/大小/时长/日期/随机/按文件夹 | Ctrl+1…Ctrl+9, Ctrl+0 | 10202,10203,10980-10985,10988,10001 |
| Select All / Invert | Ctrl+A / Ctrl+J | 10204/10205 |
| View as: List (Recommended) / 缩略图 2–7 行 + 详情 / 重新载入缩略图 / 16:9 缩略图 | — | 10900-10906, 10908, 11075 |
| Detect file durations | F4 | 10640 |
| File information... | Ctrl+F1 | 10505 |
| Create Thumbnail Image(s)... | — | 10800 |
| Playlist settings... | F5 | 10315 |
| Close Playlist | F6 | 10446 |

- 单击、双击、Enter：菜单只给出 Play=P。双击和 Enter 的行为没有静态证据，按惯例推断双击即播放，见 U5。
- 拖动排序：列表底部有 置顶/上移/下移/置底 四个按钮（`CMD_PLAYLIST_TOP/UP/DOWN/BOTTOM`，25 px 宽，SubCommon.txt:590-593），并有 “Insert dragged files at the drop point” 选项（默认关）。列表内拖动重排是否支持不能静态确认，见 U6。〔确认（按钮）〕
- 列表底部还有 Add / Del / Sort 文字按钮、总时长标签 `CMD_PLAYLIST_TOTAL_TIME_MESSAGE`、搜索框 `CMD_PLAY_LIST_SEARCH`（SubCommon.txt:596-623）。〔确认〕

### 3.4 正在播放项的标记（颜色取自 PlayList.png 色样点，x=518）

```css
.playlist-item           { color: #aeaeae; }   /* TextColorY=98 */
.playlist-item.selected  { color: #f0f0f0; background: #2b2b2b; } /* SelTextColorY=64，SelBkgndColorY=14 */
.playlist-item.current   { color: #2282f2; }   /* CurTextColorY=81：正在播放的项用蓝色文字标记 */
.playlist-item.played    { color: #888888; }   /* PlayedTextColorY=114：已播放过的项变灰 */
.playlist-list           { background: #3b3b3b; } /* BkgndColor1/2Y=48，LineColorY=48 */
```
证据：skin:SubCommon.txt:550-553，取样用 PIL。〔确认（色样）。正在播放项是否另有图标未知，见 U7〕

### 3.5 列表结束、重复与随机

| 项 | 默认 | 证据 | 可信度 |
|---|---|---|---|
| 重复（视频） | 关 | `RepeatPlay2=0`（0x18057e2b9）；菜单 `M:101_0_9_11_1..3` Disable / Repeat playlist / Repeat playing item，循环切换 CmdID 10034 | 推断（0=Disable） |
| 重复（音频） | 关 | `RepeatPlayAudio=0`；Dialog(355) “Repeat audio playback:” EN:676 | 推断 |
| 随机 | 关（视频与音频分开设置） | `RandomPlay=0`、`RandomPlayAudio=0`；菜单 `M:101_0_9_14` 10069；Dialog(355) 3113/3114 | 推断 |
| 列表结束后关闭播放 | 关 | 菜单 `M:101_0_9_18` 10445；`CloseOnComplete=0` | 推断 |
| 播放结束后的动作 | 无 | `ExitMode=0`；菜单 `M:101_0_15_4_*` | 推断 |
| 播放失败时 | 0（下拉框第一项） | `PlaybackFailSkip=0`；选项文字见 StringTable 4696-4698（框中报错并停止 / OSD 报错并继续 / OSD 报错并停止） | 推断（第 0 项对应哪一句未知） |
| OSD 提示 | `1055=Repeat Playback : %s`、`1056=Shuffle Playback : %s` | EN:3436-3437 | 确认 |

---

## 4. 控制栏与主窗口交互

### 4.1 自动隐藏

| 情形 | 行为 | 证据 | 可信度 |
|---|---|---|---|
| 窗口模式 | 控制栏常驻，不隐藏（AutoHideSkin=0） | §2.2 | 推断 |
| 窗口模式 + 开启“自动隐藏主皮肤” | 视频区活动时隐藏皮肤；OSD 文案 `1860=Auto hide main window skin while video screen is active : %s` | EN:4053 | 确认（文案）/未知（触发条件与延时） |
| 全屏 | 控制栏以弹出层覆盖视频（不缩放视频） | §2.2 | 推断 |
| 隐藏延时 | 没有找到相关键或常量 | — | **未知 U3** |
| 鼠标指针 | 播放时隐藏指针，默认开 | `AutoHideMouse=1`（0x18057e5eb movw $0x101,0x53c）；菜单 `M:101_0_9_16` | 推断（延时未知） |
| 防屏保 | 开 | `PreventScreenSave=1`（0x53d） | 推断 |

### 4.2 鼠标

- 配置结构：21 种鼠标事件 × 8 种修饰键组合（无/Ctrl/Shift/Alt/Ctrl+Alt/Ctrl+Shift/Shift+Alt/Ctrl+Shift+Alt），每格存一个 CmdID。偏移从 0x1758 起，每个事件步长 0x40。事件顺序：LeftSClick, LeftDClick, RightClick, RightDClick, X1Click, X1DClick, X2Click, X2DClick, MiddleClick, MiddleDClick, WheelUp, WheelDown, WheelLeft, WheelRight, LeftPush, RightPush, X1Push, X2Push, MiddlePush, LeftDClickLeft, LeftDClickRight。证据：键名指针表 0x181a24240..0x181a242e0；保存循环 0x1805a365d `cmp $0x15`、0x1805a368d `cmp $0x08`；对话框标签 EN:2006-2013、4833-4846、5642-5644。〔确认〕
- **SetDefaults 把整张表清零**（0x18057fb58 起 memset 0x540 字节），也就是出厂时每格都是 0，表示“使用内置默认动作”。内置默认动作写在代码里，静态没有定位到，所以**单击暂停、双击全屏、滚轮调音量、右键菜单都无法静态确认**，见 U4。按 PotPlayer 的惯例推测：单击=播放/暂停，双击=全屏，滚轮=音量，右键=右键菜单（`M:101`），中键=未知。
- 相关开关：`MouseWorkAtVideo=1`（“左键单击动作只在视频区域生效”开，0x18057fb45 movl $0x101,0x1750；Dialog(366) 3034 EN:775）；`MouseMoveOnlyTitle=0`；`MousePanScanControl=0`（Alt/Ctrl/Shift+拖动缩放画面，关）；`MouseCaptionMove=0`。〔推断〕
- 窗口模式下拖动视频可以移动窗口：Dialog(366) 3038 “Prevent window from being dragged outside of title bar”，说明默认可以在标题栏以外拖动窗口。〔推断〕

### 4.3 进度条

```css
.seekbar {
  /* skin:BottomVideo.txt:109-113：Slider CMD_TIMEBAR_CTRL，Align=Client，高 10，滑块 11×10 */
  pause-while-dragging: off;       /* SeekbarPauseResume=0（0x18057e2e1）；Dialog(358) 3064 “Pause when seeking with mouse” */
  hover-time-tooltip: off;         /* PreviewSeekTime=0（0x18057e2c0 movl $0x1010000,0x338 → 0x338=0） */
  thumbnail-preview: off;          /* PreviewThumbnail=0（同一条指令 → 0x339=0） */
  thumbnail-include-chapter: on;   /* PreviewThumbnailChapter=1（0x33a） */
  thumbnail-include-time: on;      /* PreviewThumbnailTime=1（0x33b） */
  markers: on;                     /* DisplayBookmark=1（0x33c）：书签/章节标记；皮肤另有 A-B 重复、书签、无缝、广告的标记贴图 */
  seek-to-keyframe: on;            /* PlayUseKeyFrame=1（0x18057f75a）：方向键跳转默认按关键帧 */
}
```
- 菜单开关 `M:101_0_9_21` “Show time info on tooltip when seeking”；OSD `3868/3869`（EN:4823-4824）分别是“导航条上用鼠标拖动时显示时间 tooltip / 显示缩略图预览”。〔确认（开关存在）+推断（默认关）〕
- 点击是跳到点击位置还是步进：皮肤与字符串都没有给出，见 U9。

### 4.4 时间显示

- 布局：`CMD_CURRENTTIME_MESSAGE` + “/” + `CMD_TOTALTIME_MESSAGE`，Tahoma 11（BottomVideo.txt:15-19）。当前时间标签为 `Cursor='HAND' Enabled='1'`，可点击；总时长标签不可点击。〔确认〕
- 默认显示已播放时间：`SkinCurrentLeftTime=0`（0x18057e75e movw $0,0x604）；`SkinTimeAdjSpeed=0`（“按播放速度修正时间”关，Dialog(376) 3344）。首选项里有 “Reverse elapsed/remaining times defined in skin”（Dialog(376) 3343 EN:1020），字符串 `4335=Elapsed time/Remaining time`、`4336=Remaining time/Total time`（EN:5222-5223）是两种显示组合。推断点击当前时间标签会在“已播放/总长”与“剩余/总长”之间切换。〔推断〕

### 4.5 音量滑块与静音

- `Slider CMD_VOLUME_CTRL` 宽 70，左边是 13×10 的 `ID_AUDIO_MUTE` 按钮（BottomVideo.txt:103-105）。〔确认〕
- ↑/↓ 每次 5%（`AudioVolumeUnit=5`）；M 切换静音。OSD 显示 `1077=Volume: %d%%`、`1076=Mute : %s`。另有系统主音量与波形音量的快捷键：Ctrl+Alt(+Shift)+↑/↓/M（`M:101_0_12_9_*`）。〔确认（快捷键/文案）+推断（步进）〕

### 4.6 控制栏按钮（行为部分）

| 按钮 | 左键 | 右键（AltCmdID） | 证据 |
|---|---|---|---|
| Play / Pause | CMD_MEDIA_PLAY / CMD_MEDIA_PAUSE（两个独立按钮，推断按状态切换显示） | — | BottomVideo.txt:2-3 |
| Stop | CMD_MEDIA_CLOSE（关闭播放） | — | :4 |
| 后退 | ID_PLAYBACK_5SEC_BACK（后退 5 s） | ID_PROCESS_PREV（上一个文件） | :9 |
| 前进 | ID_PLAYBACK_5SEC_FORW（前进 5 s） | ID_PROCESS_NEXT（下一个文件） | :10 |
| 打开 | ID_BASE_OPEN（按“首选打开方式”执行，默认见 `M:101_0_1_4`） | — | :12 |
| 控制面板 | ID_CONTROL_BOX_WINDOW（开关，DownFlag） | — | :62 |
| 播放列表 | ID_PLAYLIST_WINDOW（开关，DownFlag） | — | :64 |
| 停靠面板 | CMD_ATTACH_WINDOW_OPEN/CLOSE | — | :59-60 |
| 流选择 | CMD_POPUPMENU_STREAM_SELECT（弹出菜单，有可选流时才显示 AutoVisible） | — | :78 |
| 3D / 360VR | 弹出菜单（仅 VideoSkin，AutoVisible） | — | :69-71 |
| CC | ID_CLOSED_CAPTION（开关） | — | :86 |
| 信息标签 | 解码 H/W/S/W、HDR、FourCC、音频编码、声道数、下载速率；窗口宽度达到 `VisParentMinWidth` 460/700/630/670/690/740 px 才显示 | — | :22-57 |

可信度：确认（皮肤）；右键语义由日文与匈牙利文 tooltip 确认（StringTable.txt:303-304、367-368：“B:后退, J:播放上一个”，B=左键，J=右键）。

### 4.7 键盘快捷键（与我们功能相关，全部〔确认〕，来源 `M:101_*`）

| 功能 | 快捷键 | CmdID |
|---|---|---|
| 播放/暂停 | Space | 10014 |
| 上一个/下一个 | PgUp / PgDn | 10067/10068 |
| 跳转 | ←/→（5 s），Ctrl+←/→（30 s），Shift+←/→（60 s），Ctrl+Alt+←/→（300 s） | 10059-10066 |
| 上一帧/下一帧 | D / F | 10242/10241 |
| 上/下一个关键帧 | Ctrl+Shift+←/→ | 10786/10787 |
| 跳到开头/中点/结尾前 30 s | BackSpace / Ctrl+BackSpace / Shift+BackSpace | 10243/10619/10620 |
| 跳到指定时间 | G | 10325 |
| 倍速：默认/减/加 | Z / X / C（步进 0.1x：`PlaySpeedUnit=0.1f`，0x18057e88e movl $0x3dcccccd；初始与上次倍速都是 1.0：0x18057e876） | 10246/10247/10248 |
| 音量 +/−、静音 | ↑ / ↓ / M | 10035/10036/10037 |
| 均衡器开关 | Shift+E | 10095 |
| 音频同步 −/+/复位 | Shift+, / Shift+. / Shift+/ | 10291/10292/10293 |
| 打开文件 / URL / 文件夹 | Ctrl+O（另有 F3 打开文件） / Ctrl+U / F2 | 10158(10443)/10159/10160 |
| 载入字幕 | Alt+O | 10146 |
| 关闭播放 | F4 | 10167 |
| 全屏 | Enter（`M:101_0_20`）、Alt+Enter；拉伸全屏 Ctrl+Enter | 10013/10308 |
| 窗口大小 0.5x/1x/1.5x/2x/最大 | 1/2/3/4/5；最小 ` | 10232-10236/10231 |
| 宽高比循环 | Ctrl+F6 | 10261 |
| 置顶循环 | Ctrl+T | 10337 |
| 首选项 / 播放列表 / 控制面板 | F5 / F6 / F7 | 10010/10011/10383 |
| 系统信息 | Ctrl+F1 | 10195 |
| 完整/简短/杂项信息 OSD | Tab / Shift+Tab / Scroll Lock | 10351/10449/10442 |
| 画面色彩 | Q 禁用/恢复上次；W/E 亮度 −/+1%；R/T 对比度；Y/U 饱和度；I/O 色调 | 10085-10093 |
| 画面效果 | Ctrl+P 垂直翻转，Ctrl+Z 水平翻转，Ctrl+B 柔化，Ctrl+R 锐化，Ctrl+L 色阶，Ctrl+H 去块，Ctrl+N 3D 降噪 | 10070… |
| 截图 | Ctrl+C 复制源帧，Ctrl+E 保存源帧，Ctrl+Alt+C/E 屏幕帧，Ctrl+G 连续截图，Alt+N 缩略图 | 10223/10224/10432/10433/10222/10353 |
| 字幕 | Alt+H 显示/隐藏，Alt+L 切换字幕，> / < 字幕同步 ±，/ 复位，Alt+PgUp/PgDn 字号，Alt+↑/↓/←/→ 移动位置，Alt+Home 复位位置，Alt+F 字体设置 | 10126… |

- Esc 退出全屏：菜单里没有，无法静态确认，见 U10。
- “Disable default hotkeys”（`M:101_0_15_12`，10339）可以整体关闭默认快捷键。〔确认〕

### 4.8 右键菜单

- 主右键菜单是 MENU 101 “Popup Menu”，761 个节点，顶层依次为 Open, Tools, Album, Close, Visualization, TV, DVD, Playback, Subtitles, Video, Audio, Filters, Skins, Misc, Frame Size, Aspect Ratio, Window Size, Fullscreen, Preferences, Playlist, Control Panel, System Info, About, Exit（`M:101_0_*`）。MENU 115 是精简版（Play/Pause, Close, Volume, Aspect Ratio, Fullscreen, System Info, About），推断用于托盘或迷你场景。〔确认（结构）/推断（115 的用途）〕
- 右键菜单的样式（皮肤化、透明度、缩放）：`SkinPopupMenuStyle=0`、`SkinPopupMenuTrans=9`、`SkinPopupMenuScale=1`（Dialog(376) 3289-3292）。〔推断〕

---

## 5. OSD 与播放信息

### 5.1 OSD 通用设置（Dialog(375) “General OSD settings”，EN:965-999）

```css
.osd-message {
  enabled: true;                 /* MessageVisible=1（0x180580074）“Show general info messages” */
  font-weight: 700;              /* MessageFontWeight=700（0x18058009d） */
  font-size: auto;               /* MessageFontSize=0（推断 0=自动；Dialog(375) 3040 “Auto font size”） */
  color: 0;                      /* MessageTextColor=0（推断 0=使用皮肤/默认色） */
  outline-color: 0xC8000000;     /* MessageOutlineColor=-939524096：推断 ARGB，即 alpha 0xC8 的黑色描边 */
  margin-left: 10px; margin-top: 10px; /* MessageLeftMargin/TopMargin=10（0x1805800d7…） */
  position: 0, 0;                /* MessageHoriPos/VertPos=0（推断：左上） */
}
.osd-info-font { size: 11; weight: 0; } /* OsdFontSize=11（0x180580185），OsdFontWeight=0 */
```
可信度：推断。

### 5.2 各操作的 OSD 文案（StringTable，〔确认〕）

| 操作 | 文案（EN 行） |
|---|---|
| 暂停/继续/关闭 | 1050 “Playback paused”、1051 “Playback resumed”、1049 “Playback closed”（EN:3430-3432） |
| 跳转 | 1052 “%s Backward”、1053 “%s Forward”、1054 “Seeking : %s” |
| 倍速 | 1100 “Playback speed : %.3gx” |
| 音量/静音 | 1077 “Volume: %d%%”、1076 “Mute : %s”；系统音量 1691/1692 |
| 色彩 | 1068-1075 “Contrast %d%% Increase (%.1f%%)” 等；1067 “Color Controls : Disabled” |
| 画面 | 1057/1058 翻转，1129-1134 去块/去隔行/柔化/锐化/降噪/色阶 |
| 均衡器 | 1078 “Equalizer : %s”、1079 “Load equalizer preset: %s”、1080 Normalizer |
| 宽高比 | 1006-1015 |
| 窗口 | 1004/1005 全屏/窗口模式，1082-1086 窗口大小，1088-1090、1593、4561 置顶 |
| 字幕 | 1101/1102 显示/隐藏，1103-1107 位置，1123/1124 字号，1126-1128 同步 |
| 音频同步 | 1724 “Audio sync : %g sec.” |
| 截图 | 1137 “Frame Capturing : Frame was saved as %s”，1138 “… copied to clipboard” |
| 重复/随机 | 1055/1056；AB 重复 1152/1153 |
| 播放列表窗口 | 1044 “Playlist Window : %s” |

### 5.3 播放信息叠加层（调试信息插件参照）

**三种信息 OSD**（菜单 `M:101_0_15_16..18`，〔确认〕）：

| 命令 | 快捷键 | CmdID / 处理函数 | 默认 |
|---|---|---|---|
| Display Full Playback/Audio Tag Info on Screen | **Tab** | 10351 → 0x1803b1fd0 → `0x180335500(this, true)`，切换的是视频渲染器侧的统计叠加（通过渲染器对象虚函数 +0x90/+0x138） | 关（`MessagePlayInfo=0`、`MessagePlayInfoFull=0`，0x1805800fe） |
| Show Short Playback Info on Screen | Shift+Tab | 10449 → 0x1803b1fe0 | 按需显示 |
| Display Misc Info on Screen | Scroll Lock | 10442 → 0x1803b1f50（OSD 1873/2014 “Display misc information : %s”） | 关 |

**简短信息（Shift+Tab）显示的内容**（Dialog(375) 3195 “Content of Short Playback Info”，〔推断：键名与勾选项一一对应〕）：
文件名 `ShortMessageFile=1`、字幕名 `ShortMessageCaption=1`、播放时间 `ShortMessageTime=1`、书签/章节 `ShortMessageChapter=1`、编码信息 `ShortMessageCodec=1`（0x1805801bc movl $0x1010000,0x200c；0x1805801c6 movw $0x101,0x2010）。

**杂项信息（Scroll Lock）显示的内容**（Dialog(375) 3194，〔推断〕）：
播放时间 `MessagePlayTime=1`、百分比 `MessagePlayPercent=1`、已播帧数 `MessagePlayFrame=0`、列表序号 `MessagePlayIndex=1`、文件名 `MessageFileName=1`、章节 `MessageChapter=1`、电源状态 `MessageBetery=0`、本地时间 `MessageNowTimeFull=0`。还有“只在全屏显示”的选项（Dialog(375) 3045/3047）。

**完整信息（Tab）可能的字段**：渲染器统计的标签是一组连续的本地化字符串（EN:5209-5251、5355-5359）。其中 Render Device(4322)、Refresh Rate(4323)、Resizer(4338)、Transform(4339)、Format(4340)、Range(4341)、Primaries(4342)、Transfer(4343)、ColorSpace(4344)、Location(4345，色度位置)、Mode(4346)、Fullscreen Exclusive(4347/4348)、Frames(4349)、Sync Offset(4350)、Jitter(4351)、Formats(4352)、Dropped(4359)、Present(4481)、Present Refresh(4482)、Sync Refresh(4483)、Sync Diff(4484)、VRAM(4485)、PotPlayer/OS Version(4486)。DLL 内置韩文字符串表里同一位置也有这些英文片段（UTF-16 @文件偏移 0x1c5e1e4–0x1c5e900：Present、Pixel Format、Dropped、Buffer Time、Color Convert、Resizer、Format、Pixel、Range、Primaries、Transfer、ColorSpace、ChromaLoc、Mode、Exclusive、Frame、Sync Offset、Jitter、Device、Refresh Rate、Current Time）。音频标签部分按菜单名“Audio Tag Info”推断。〔推断：没有找到直接引用这些字符串 ID 的代码，字段顺序和完整清单未知，见 U11〕

系统信息对话框（Ctrl+F1，“Playback/System Information” 1354）的分组：`[Used Filter List]`、`[Video Information]` Codec / Input type / Input size / Output type / Output size / Frame rate、`[Audio Information]` Codec / Sample rate / Bits per sample / Channels（EN:3685-3701）。〔确认〕这可以作为调试信息插件的静态字段来源。

---

## 6. 与我们功能域对应的工具窗口

### 6.1 控制面板（F7，ID_CONTROL_BOX_WINDOW）——画面、音频、字幕、播放的集中调节

- 独立皮肤窗口 `ControlBoxSkinArea` 301×227（触控版 301×466），默认不显示（`ControlBoxWindowVisible=0`），默认位置 Top=549、尺寸 307×201（`ControlBoxTop/Width/Height`：0x18057d9e5 movl $0x225,0x254；0x18057db73 movl $0x133,0x258；0x18057db7d movl $0xc9,0x25c）。标题栏有透明度滑块 `CMD_TRANS_CB`、置顶切换、关闭按钮（SubCommon.txt:189-201）。〔确认/推断〕
- 四个页签 `CMD_SELECT_PAGE PageIndex=0..3`：**音频 / 视频 / 字幕 / 播放**（SubCommon.txt:213-224）。〔确认〕
  - 音频页：10 段均衡器竖滑块 `CMD_EQULIZER_0..9`，主音量和波形音量竖滑块，均衡器开关 `ID_AUDIO_EQULIZER`，预设弹出菜单（`CMD_POPUPMENU_EQULIZER_PRESET`；MENU 106：Add… / Save / Delete / Default）。
  - 视频页：亮度 `CMD_COLOR_OFFSET`、对比度 `CMD_COLOR_GAIN`、饱和度 `CMD_COLOR_SAT`、色调 `CMD_COLOR_HUE` 四个滑块（154×12），**每个滑块右侧有一个 41×18 的“Default”按钮**（`CMD_COLOR_*_DEFAULT`，SubCommon.txt:258-277）；下方是柔化/锐化/去块开关（`ID_VIDEO_BLUR/SHARPEN/POSTPROC`）和“截图（带提示）”`ID_VIDEO_IMAGE_CAPTURE_PROMPT`。
  - 字幕页：同步 快/慢/复位、位置 上/下/左/右、字号 +/−、字体选择。
  - 播放页：±5 s / ±30 s / ±1 min / ±5 min、倍速 慢/默认/快、A-B 重复 设 A / 设 B / 复位。
- 应用时机：滑块直接绑定命令 ID，与快捷键 W/E 等走同一套命令，推断**拖动时立即生效，没有确定/取消按钮**。〔推断〕
- 复位：色彩逐项复位（Default 按钮）；Q 键在“关闭/恢复上次色彩调节”之间切换（10085）；均衡器预设菜单有 “Default”，均衡器对话框有 “0 dB” 按钮（Dialog(363) 3024）。〔确认〕

### 6.2 画面调节的默认值与刻度

`VideoLumGain=128`、`VideoLumOffset=0`、`VideoHUE=0`、`VideoSaturaction=64`（0x18057e317 movl $0x80,0x3b4；0x18057e321 movq $0x80,0x3b8；0x18057e33a movl $0x40,0x3cc）。每项占两个 int（推断分别是软件值和硬件值）。推断内部刻度：对比度 128 为中性，饱和度 64 为中性，亮度与色调 0 为中性；UI 和 OSD 用百分比显示（“Brightness %d%% Increase (%.1f%%)”）。`VideoPropertyHwFirst=0`（优先软件色彩调节；Dialog(384) 3083 “Use hardware color controls (if possible)”）。色阶：`LevelControl=0`，输入 16–235、输出 0–255，`LevelGamma=100`。〔推断〕
首选项里另有 Dialog(384) “Color Controls”页（Brightness/Contrast/Saturation/Hue 加四个 Default 按钮，EN:1168-1186）。〔确认〕

### 6.3 音频与均衡器

- 均衡器默认关（`AudioEqulizer=0`、`AudioEqulizerSuperEQ=0`，0x18057e4ee）。均衡器对话框 Dialog(363)：Preset、Enable、SuperEQ、“0 dB”，频段 60/170/310/600/1K/3K/6K/12K/14K/16K（EN:722-746）。〔确认（标签）+推断（默认）〕
- Normalizer 默认关（Shift+N），`AudioNormalizeAmp=75`、`AudioNormalizePercent=100`。〔推断〕
- 均衡器既出现在控制面板（非模态），也出现在首选项树里；菜单只提供开关 Shift+E 和预设子菜单（`M:101_0_12_14/15`）。〔确认〕

### 6.4 字幕设置

- 字幕默认显示（`CaptionVisible=1`，0x18057e79a movl $0x100,0x760 → 0x761=1）。〔推断〕
- 调节方式有三种：菜单与快捷键直接生效（OSD 1101-1128）；控制面板的字幕页；首选项里的 Subtitle 页（Dialog “Subtitle Settings” EN:515 等）。“Font Settings...” Alt+F（10139）是对话框。〔确认〕

### 6.5 截图

- 只用菜单和快捷键完成，没有截图窗口：Ctrl+E 直接保存到截图文件夹，成功后 OSD 1137；Ctrl+C 复制到剪贴板，OSD 1138；“Save … As...”（10357/10434）弹出保存对话框。〔确认（菜单/文案）〕
- 默认格式 JPEG：`VideoCaptureFormat=1`（0x18057f027），菜单 BMP/JPEG/PNG 依次为 10354/10355/10356，取值 1 对应 JPEG。质量 100（`VideoCaptureQuality=100`）。“截图包含 OSD 消息和字幕”默认关（`CaptureWithCaption=0`）。“截图时修正宽高比”默认开（`CaptureArCorrect=1`）。连续截图默认 10 张（`VideoCaptureFrames=10`）。〔推断〕
- 截图文件夹：菜单 Set/Open Image Storage Folder（10334/10225）；“存到播放文件所在文件夹”默认关（`VideoCaptureFolderSelf=0`）。〔推断〕

### 6.6 非模态窗口的通用规则

- “Always keep modeless dialog boxes on top”默认关（`UseModelessOnTop=0`，Dialog(436) 3063）。说明 PotPlayer 的这类工具对话框是非模态的。〔推断〕
- 首选项（F5）是树形对话框，各页的改动是否立即生效要看具体页面，见 U12。

---

## 7. 未知项清单（需 Windows 实机确认）

| # | 未知项 | 需要怎么确认 |
|---|---|---|
| U1 | 出厂默认值是否被安装器写入的 ini 或注册表覆盖（例如列表可见、窗口尺寸） | 全新安装后检查 `HKCU\Software\DAUM\PotPlayer64` 或 `PotPlayer64.ini`，并对首屏截图 |
| U2 | 780×671 是否按 DPI 缩放；它是外框尺寸还是客户区尺寸；是否包含停靠的播放列表 | 在 100%/150% 缩放下首次启动，测量窗口和视频区尺寸 |
| U3 | 全屏时控制栏与鼠标指针的自动隐藏延时、触发区域（底部多少 px 会唤出） | 全屏后停止移动鼠标并计时；把鼠标移到底部和右侧边缘观察 |
| U4 | 鼠标内置默认动作：单击（是否暂停）、双击（是否全屏）、滚轮（音量还是跳转）、中键、X1/X2、左/右半区双击 | 在 Preferences > Mouse 页查看每一项显示的默认值，并实际操作验证 |
| U5 | 播放列表单击/双击/Enter 的行为（双击是否立即播放、Enter 是否播放） | 实机操作 |
| U6 | 播放列表内能否用鼠标拖动重排，拖到其它专辑页签的效果 | 实机操作 |
| U7 | 正在播放项除蓝色文字外，是否还有图标或背景标记；暂停状态是否不同 | 截图 |
| U8 | `DisplayAsThumbnail=1` 的含义（默认是“列表”还是“缩略图 2 行”） | 首次运行时查看列表右键 View as 的勾选项 |
| U9 | 进度条点击是直接跳到点击位置还是步进；拖动时视频是否实时刷新；时间 tooltip 默认真的关闭吗 | 实机操作 |
| U10 | Esc 是否退出全屏或关闭窗口；双击全屏后的恢复方式 | 实机操作 |
| U11 | Tab 完整信息叠加层的实际字段、顺序、位置、字体与刷新率 | 播放视频时按 Tab 截图（软解与硬解各一张） |
| U12 | 首选项各页（色彩、音频、字幕）改动是立即预览还是要点“应用”；“Default”按钮是否一并复位硬件值 | 实机操作 |
| U13 | 播放失败选项 `PlaybackFailSkip=0` 对应哪一种动作 | 查看 Preferences > Playback 的下拉框 |
| U14 | 时间标签点击后的切换顺序（已播放 ↔ 剩余 ↔ 其它） | 实机点击 |

**未知项总数：14**
