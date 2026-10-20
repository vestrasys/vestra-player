# A：PotPlayer 默认皮肤的外观与布局（静态提取）

分析对象：`$P/skin-default/`（来自 `x/Skins/Default.dsf`）以及 `$P/skin-frame/`（来自 `x/Skins/WindowFrame.dsf`）。
下文路径中的 `SD/` 表示 `$P/skin-default/`，`SF/` 表示 `$P/skin-frame/`，`DLL` 表示 `$P/x/PotPlayer64.dll`（地址均为**文件偏移**），`EN.ini` 表示 `$P/x/Language/English.ini`。
像素基准：皮肤坐标都是 1x 像素（96 DPI）。颜色是用 PIL 在皮肤引用的 `ColorX/ColorY` 坐标或 `ImageLeft/Top/Width/Height` 切片上采样得到的，采样点周围 3x3 像素颜色一致，除非另有说明。
可信度标记：**[确认]** 表示皮肤文件或像素直接给出；**[推断]** 后面写有推理过程；**[未知]** 的条目汇总在第 3 节。

---

## 1. 摘要（最重要的 10 条）

1. **采用哪个分支**：桌面版默认走非 Touch 分支（`#else`），主窗口用 `<MainSkinArea>`。控件按声明顺序停靠，从外到内依次是：3px 黑边框、31px 标题栏、右侧附着面板（含 3px 分隔条）、底部两行控制栏（22px 进度条行在上，40px 按钮行在下），剩下的区域是视频。在这种模式下控制栏**位于视频之外**，不覆盖画面。只有用户开启 “On Screen Control（OSC）” 时才切换到 `MainSkinAreaOscFrame` 加 `MainSkinAreaOscInside`，此时标题栏和控制栏改为覆盖在视频上并自动隐藏。[确认/推断，见 2.1]
2. **主窗口最小宽度 320px**，高度不设下限（`HeightLimit='0'`）。[确认] `SD/VideoSkin.xml:54,56`
3. **底部控制区总高 62px**，即进度条行 22px 加按钮行 40px；底边框另有 3px。标题区为 3px 边框加 31px 标题栏，共 34px。[确认]
4. **配色是纯暗色系**：栏背景 `#1e1e1e`，视频和空闲区 `#101010`，边框和分隔线 `#000000`。悬停色 `#272727`，按下色 `#181818`。主文字 `#f0f0f0`，次要文字 `#777777`。**强调色是黄色 `#fae100`**，用于进度条已播放部分、音量、图标悬停、滑块悬停和菜单选中文字；按下时为 `#c8b400`。蓝色 `#2282f2` 只用于“正在播放”的列表项和控制面板里的滑块。[确认，像素采样]
5. **进度条**：可见轨道只有 **2px 高**，放在 10px 高的槽里；已播放部分为 `#fae100`，未播放轨道为 `#000000`。滑块是 8px 圆点，所在单元 11x10：常态 `#f0f0f0`，悬停 `#fae100`，按下 `#c8b400`，带柔和黑色阴影。进度条占满按钮行上方那一行（Align=Client），左右各留 10px。[确认]
6. **音量条**：宽 70px，高 10px，同样是 2px 线，黄色填充、黑色轨道，滑块 11x10。位于进度条行的最右端，左边是 13x10 的静音按钮。[确认] `SD/BottomVideo.txt:103-105`
7. **按钮行从左到右**：播放/暂停、停止、后退 5 秒（右键：上一个）、前进 5 秒（右键：下一个）、打开文件，这 5 个按钮各 41x40。然后间隔 10px，接着是 `当前时间 / 总时长`（Tahoma 11，当前时间 `#f0f0f0`，斜杠和总时长 `#777777`）和若干信息徽标（HW/SW、HDR、视频编码、音频编码、声道）。右侧依次是 CC、流选择、360°/3D、浏览器、播放列表、控制面板，这些是 23x22 的图标，间距 8px；最后一个是 41x40 的“打开/关闭附着面板”按钮。[确认]
8. **按钮图片**：每个按钮在图里有 4 帧，横向排列、步距等于按钮宽度，依次为常态、悬停、按下、禁用。图标常态 `#777777`，悬停 `#fae100`，按下 `#c8b400`，禁用 `#3b3b3b`。大按钮（41x40）变的是底色（`#1e1e1e` / `#272727` / `#181818`），字形保持 `#f0f0f0`，右侧有 1px `#000` 分隔线。[确认，像素]
9. **播放列表**默认附着在主窗口右侧，与视频之间有 3px 可拖动分隔条。分隔条左侧最少保留 318px，面板最少 317px。面板顶部是 31px 标签条（直播/聊天/播放列表），每个标签宽 105px。列表背景 `#3b3b3b`，没有斑马纹；选中行背景 `#2b2b2b`，文字 `#f0f0f0`；正在播放的行文字为蓝色 `#2282f2`；普通文字 `#aeaeae`，已播放文字 `#888888`。底部按钮栏高 35px，滚动条宽 20px。行高和字体由程序设置决定，皮肤没有给出。[确认/未知]
10. **右键菜单**的皮肤样式（`CustomPopupMenuDef`）：边框和底色 `#444444`，外围 1px `#000`。文字 `#bbbbbb`；选中项背景 `#181818`，选中文字为黄色 `#fae100`；禁用文字 `#777777`；分隔线 `#27282f`。勾选标记是 14x14 圆角方块（`#34353f` 底加白勾），单选标记 12x12。另有一套浅色备选样式（`CustomPopupMenuAlt`）。菜单项高度皮肤没有给出。[确认/未知]

---

## 2. 分区域事实

### 2.1 皮肤入口与分支选择

| 事实 | 证据 | 可信度 |
|---|---|---|
| 默认皮肤文件是 `Skins/Default.dsf`；DLL 中有回退名 `Default.dsf` | `x/Skins/Default.dsf`；DLL ASCII 0x137cf58 | 确认 |
| `VideoSkin.xml` 用于视频模式，`AudioSkin.xml` 用于纯音频模式；可通过“以视频皮肤启动”选项切换 | DLL UTF-16 0x13a5508 `LastSkinXmlNameVideo`，0x13a5538 `LastSkinXmlNameAudio`；`EN.ini` 3109 `101_0_14_9=Use video skin at program start` | 确认 |
| `#if<SkinType==Touch>` 比较的是字符串 `touch`，来源于注册表或配置项 `LastSkinTypeTouch`，菜单项是“Enable touch feature”。桌面默认没有开启触控功能，所以走 `#else` 分支 | DLL ASCII 0x137cc48 `touch`、0x137cc80 `<SkinType`；DLL UTF-16 0x13a5458 `LastSkinTypeTouch`；`EN.ini` 3147 `101_0_15_14=Enable touch feature`，1921 `Enable Touch Feature (requires at least Windows 7)` | 推断：默认值没有直接在静态数据中看到 |
| `#if<CodePage==949>` 只在韩文代码页下改用韩文 logo 和菜单按钮图；英文和中文走 `#else`（`PotPlayerEng.png`，菜单按钮图 `ImageTop=43`） | `SD/Logo.txt:13-17`，`SD/TopCommon.txt:3-7`；中文 ini 中 `CodePage=936` | 确认 |
| `#if<SkinName==VideoSkin>` 成立，因此 3D 和 360° 按钮会被包含；DvdSkin 和 WdmSkin 的分支不生效 | `SD/BottomVideo.txt:67-84` | 确认 |
| 皮肤一共定义了三种主窗口区域：`MainSkinArea`（常规）、`MainSkinAreaOscFrame`（OSC 外框）、`MainSkinAreaOscInside`（OSC 覆盖层） | `SD/VideoSkin.xml:8,32,52`；DLL ASCII 0x137cfc8 `MainSkinArea`、0x137cfd8 `OscFrame`、0x1385cc0 `MainSkinAreaOscInside` | 确认 |
| OSC 的开关在“皮肤 → On Screen Control”菜单：Disable / Direct3D9 / Layered Window。开启 OSC 后，外框只保留边框、附着面板和视频，标题栏和控制栏移到 Inside 层并覆盖在视频上；另有选项“鼠标悬停时不自动隐藏 OSC” | `EN.ini` 3102-3106（`101_0_14_5_*`）；`EN.ini` 1746 `Dialog(434) 3034=Disable auto hide of OSC while mouse cursor is over it`；`SD/VideoSkin.xml:14-30` 对比 `34-48` | 确认（结构）/ 未知（默认是否开启 OSC） |
| 另有“视频画面激活时自动隐藏主皮肤”选项（CmdID 10369） | `EN.ini` 3107；`menu-tree.md:626`；DLL UTF-16 0x13abf38 `AutoHideSkin` | 确认（存在）/ 未知（默认值） |
| `skin-frame`（“Window Frame Skin”）与默认皮肤的区别：`UseWindowFrame='3'`（使用系统窗口边框），去掉 `Top.txt`、`BottomLeft.txt`、`Right.txt` 这几条 3px 自绘边框，其余布局相同 | `SF/VideoSkin.xml:12,47,56-60`；`SF/Title.txt` | 确认 |

**常规模式（MainSkinArea，非 Touch）的子控件声明顺序**，见 `SD/VideoSkin.xml:71-83`：
`Top.txt` → `BottomLeft.txt` → `Right.txt` → `TopCommon.txt` → `RightAttach.txt` → `TopCast.txt` → `BottomVideo.txt` → `Popup.txt` → `BottomVod.txt` → `Static Align='Client' Contain='CMD_VIDEO_WINDOW'`

停靠语义：先声明的 Top/Bottom/Left/Right 控件占据最外圈，后声明的依次往内收；同一方向上，先声明的更靠外。这一点由 `Main.png` 中的主窗口模板图（0,487 起，353x246）直接验证：模板从上往下依次是 3px 黑边、31px 标题栏（最后 1 行为 `#000`）、31px 直播信息条区域、视频区 `#101010`、22px 进度条行（上下各有 1px `#000`）、40px 按钮行、3px 黑边；右侧有 3px 分隔条和面板。**[确认]**

推论：标题栏（`TopCommon`）声明在附着面板（`RightAttach`）之前，所以**标题栏横跨整个窗口宽度，包括面板上方**。控制栏（`BottomVideo`）声明在面板之后，所以**控制栏只在视频的下方，不延伸到面板下方**；面板从标题栏下沿一直到底边框。**[推断，依据声明顺序与停靠语义]**

```css
/* 主窗口整体（常规模式，1x 像素） */
.main-window      { min-width: 320px; /* WidthLimit, VideoSkin.xml:54 */ min-height: none; /* HeightLimit=0 */
                    background: #101010; }
.frame-border     { border: 3px solid #000000; }      /* Top.txt:1, BottomLeft.txt:1-2, Right.txt:1；切片为纯 #000 */
.titlebar         { height: 31px; background: #1e1e1e; border-bottom: 1px solid #000000; /* 位于 31px 之内 */ }
.video-area       { flex: 1; background: #101010; }   /* 模板视频区与空闲 logo 底色 Main.png(50,600) 均为 #101010 */
.seek-row         { height: 22px; background: #1e1e1e; border-top: 1px solid #000; border-bottom: 1px solid #000; }
.button-row       { height: 40px; background: #1e1e1e; }
/* 布局：上 3+31=34px，下 22+40+3=65px，左右各 3px */
```

**九宫格切片（推断）**：带图片的 `Static` 上的 `LeftMargin/RightMargin`（例如 `Top.txt` 的 20/20、`BottomVideo` 的 10/10、播放列表头部的 100/10）应理解为图片的固定端帽宽度，中间部分拉伸，而不是内容内边距。依据是：3px 顶边框设了 20/20 的 margin，但模板里边框横贯全宽；另外按钮行第一个按钮紧贴左端，没有 10px 留白。**[推断]**

**`AltImageLeft/Top`（备用切片）**：标题栏的备用切片底色是 `#151515`，常态是 `#1e1e1e`；底部两行的备用切片和常态相同，都是 `#1e1e1e`。推断备用切片用于**窗口失去焦点（inactive）**时的外观。**[推断：没有找到直接说明]**

### 2.2 标题栏（TopCommon.txt）

`SD/TopCommon.txt:1`：`Align='Top' Height='31' IsTitleBar='1'`，可拖动；切片 `Main.png(3,490,347x31)`，底色 `#1e1e1e`，最后一行 `#000`。[确认]

从左到右（Align=Left 的控件按声明顺序排列）：

| 顺序 | 元素 | 尺寸 / 间距 | 命令 / 说明 | 证据 |
|---|---|---|---|---|
| 1 | 留白 | 1px | | `TopCommon.txt:2` |
| 2 | 主菜单按钮（文字为 “PotPlayer ∨” 的图片） | 89x30 | `CMD_MAIN_MENU`；四帧在 `Main.png(0/89/178/267,43)`：常态透明底加文字 `#f0f0f0`，悬停底 `#272727`，按下底 `#181818`，禁用底 `#1e1e1e`、文字 `#3d3d3d` | `:6` 与像素 |
| 3 | 留白 | 5px | | `:8` |
| 4 | 文件类型文字 | 自动宽度；字体 `#DEFAULT_UI_FONT` 13；颜色 `#777777`（Main 864,53） | `CMD_FILE_TYPE_MESSAGE`；窗口宽度不足 300px 时隐藏（`VisParentMinWidth='300'`） | `:9` |
| 5 | 留白、竖分隔 | 8px，然后 3x10 图（`#444444`） | 有文件类型时才显示 | `:10-11` |
| 6 | 直播成人/限制图标 | 22x22、26x26 | 只在直播时显示，与本地播放无关 | `:13-17` |
| 7 | 标题文字（Client，占满剩余宽度） | 字体 13，`#f0f0f0`（Main 864,494）；文字过长时滚动：每 50ms 移动 1px | `CMD_TITLE_MESSAGE` | `:41` |

从右往左（Align=Right，先声明的在最右）：

| 从右起 | 元素 | 尺寸 | 命令 |
|---|---|---|---|
| 1 | 留白 | 10px | |
| 2 | 关闭 | 14x13（图标 9x9 左右） | `ID_APP_EXIT` |
| 3 | 留白 | 10px | |
| 4 | 全屏 | 14x13 | `CMD_FULL_WINDOW` |
| 5 | 留白 | 10px | |
| 6 | 最大化 / 还原（二选一显示） | 14x13 | `CMD_MAX_WINDOW` / `CMD_RESTORE_WINDOW` |
| 7 | 留白 | 10px | |
| 8 | 最小化 | 14x13 | `CMD_MIN_WINDOW` |
| 9 | 留白 | 10px | |
| 10 | 置顶状态（4 种图标按状态切换，`AutoVisible` 取 1 到 4） | 14x13 | `ID_ETC_NEXTONTOP`（循环切换置顶方式） |
| 11 | 留白 | 10px | |
| 12 | 竖分隔线 | 1x31，`#000` | |
| 13 | 留白 | 8px | |

证据：`SD/TopCommon.txt:19-39`。窗口按钮四帧的步距是 14px（`Main.png` y=20 处实测的像素区间为 2-10、16-24、30-38、44-52），颜色依次为 `#777777`、`#fae100`、`#c8b400`、`#3b3b3b`。[确认]

```css
.titlebar { height: 31px; background: #1e1e1e; border-bottom: 1px solid #000; padding: 0 10px 0 1px; }
.titlebar .main-menu-btn { width: 89px; height: 30px; }
.titlebar .main-menu-btn:hover  { background: #272727; }
.titlebar .main-menu-btn:active { background: #181818; }
.titlebar .file-type { font: 13px var(--ui-font); color: #777777; margin-left: 5px; }
.titlebar .title     { font: 13px var(--ui-font); color: #f0f0f0; flex: 1; /* marquee 1px/50ms */ }
.titlebar .sys-btn   { width: 14px; height: 13px; color: #777777; margin-left: 10px; }
.titlebar .sys-btn:hover  { color: #fae100; }
.titlebar .sys-btn:active { color: #c8b400; }
.titlebar .sys-btn:disabled { color: #3b3b3b; }
.titlebar .divider { width: 1px; height: 31px; background: #000; margin: 0 10px 0 8px; }
/* 右侧顺序（从左到右）：[8px] | [10] 置顶 [10] 最小化 [10] 最大化/还原 [10] 全屏 [10] 关闭 [10] */
```

`TopCast.txt` 的 31px 信息条只在直播状态下显示（`State='CMD_VIEWER_MODE CMD_BROADCAST_MODE'`，`SD/TopCast.txt:1`），本地播放时不出现。[确认]

### 2.3 底部控制栏（BottomVideo.txt）

两个 `Align='Bottom'` 的 Static：第一个（`:1`）是 40px 的按钮行，位于最底；第二个（`:89`）是 22px 的进度条行，位于按钮行上方。顺序由模板图验证（2.1 节）。[确认]

#### 2.3.1 进度条行（22px）`SD/BottomVideo.txt:89-115`

切片 `Main.png(3,668,306x22)`：第 1 行和最后 1 行为 `#000`，中间 `#1e1e1e`。[确认]

从左到右：留白 2px + 3px（`:91,100`），然后是**进度条**（Align=Client），再往右依次是留白 10px（`:107`）、**静音按钮** 13x10（`:105`）、留白 4px（`:104`）、**音量滑块** 70x10（`:103`），最右端留白 10px（`:102`）。

```css
.seek-row { height: 22px; display: flex; align-items: center; padding: 0 10px 0 5px; }
.seekbar  { flex: 1; height: 10px; /* 槽 */ margin: 0 10px; /* 滑块 LeftMargin/RightMargin=10，推断为滑块行程内缩 */ }
.seekbar .track  { height: 2px; background: #000000; }   /* BackImage Main(0,193) 第 197-198 行不透明 */
.seekbar .played { height: 2px; background: #fae100; }   /* Image Main(0,179) 第 183-184 行不透明 */
.seekbar .alt    { height: 2px; background: #444444; }   /* AltImage Main(0,207)，用途见未知项 */
.seekbar .thumb  { width: 8px; height: 8px; border-radius: 50%;  /* 绘制在 11x10 单元内，四周有 1px 半透明黑阴影（alpha 14-49） */
                   background: #f0f0f0; }
.seekbar .thumb:hover  { background: #fae100; }
.seekbar .thumb:active { background: #c8b400; }
.seekbar .thumb:disabled { background: #7d7000; }        /* 帧位于 (1/12/23/34, 239) */
.seekbar .marker-a, .marker-b { width: 6px; height: 10px; } /* 白色三角，带黑描边：A 在 Main(148,241)，B 在 (155,241) */
.seekbar .bookmark { width: 6px; height: 10px; }            /* 白色五边形旗标 Main(171,241) */
.mute-btn  { width: 13px; height: 10px; color: #777777; }   /* :hover #fae100；:active #fbe41a；:disabled #7d7000 —— 帧位于 Main(175/188/201/214,167) */
.volume    { width: 70px; height: 10px; /* 两端行程内缩 5px（LeftMargin/RightMargin=5） */ }
.volume .track { height: 2px; background: #000; }          /* BackImage Main(175,207) 第 211-212 行 */
.volume .level { height: 2px; background: #fae100; }       /* Image Main(175,194) 第 198-199 行不透明，196-201 行为透明的 #fae100 */
.volume .thumb { /* 与进度条滑块相同：11x10，四帧位于 Main(85/96/107/118,239) */ }
```

命令：进度条 `CMD_TIMEBAR_CTRL`，音量 `CMD_VOLUME_CTRL`，静音 `ID_AUDIO_MUTE`。另外进度条支持 A-B 循环、书签、无缝点、广告这些标记（`:109-113`）。[确认]

#### 2.3.2 按钮行（40px）`SD/BottomVideo.txt:1-87`

切片 `Main.png(3,690,306x40)` 是纯 `#1e1e1e`。[确认]

**左侧（从左到右）**：

| # | 元素 | 尺寸 | 命令（AltCmdID 为右键动作） | 证据 |
|---|---|---|---|---|
| 1 | 播放（与暂停二选一显示，推断按播放状态切换） | 41x40 | `CMD_MEDIA_PLAY` | `:2` |
| 1' | 暂停 | 41x40 | `CMD_MEDIA_PAUSE` | `:3` |
| 2 | 停止 | 41x40 | `CMD_MEDIA_CLOSE` | `:4` |
| 3 | 后退 | 41x40 | `ID_PLAYBACK_5SEC_BACK`；右键 `ID_PROCESS_PREV` | `:9`；StringTable 日文版 `:303`：“右クリック：前のメディアを再生”（右键：播放上一个） |
| 4 | 前进 | 41x40 | `ID_PLAYBACK_5SEC_FORW`；右键 `ID_PROCESS_NEXT` | `:10`；`StringTable.txt:304` |
| 5 | 打开文件 | 41x40 | `ID_BASE_OPEN` | `:12` |
| 6 | 留白 | 10px | | `:14` |
| 7 | 当前时间 | 自动宽度（最小 50）；Tahoma 11；`#f0f0f0`（Main 864,528）；向下偏移 1px；手形光标，可点击 | `CMD_CURRENTTIME_MESSAGE` | `:15` |
| 8 | 留白 5px，`/`，留白 5px | Tahoma 11，`#777777`（Main 875,528） | | `:16-18` |
| 9 | 总时长 | Tahoma 11，`#777777` | `CMD_TOTALTIME_MESSAGE` | `:19` |
| 10 | 留白 | 5px | | `:20` |
| 11 | HW/SW 解码徽标（可点击） | 自动宽度；底图 41x14，九宫格端帽 3px；左右内边距各 5；Tahoma 11，`#777777`，悬停 `#fae100`；窗口宽度 ≥460 时显示 | `CMD_VIDEO_OUT_HW_SW_MESSAGE` | `:22-26` |
| 12 | HDR 徽标 | 同上；宽度 ≥700 时显示 | `ID_VIDEO_PS_SMPTE_2084` | `:28-33` |
| 13 | 视频编码 FOURCC（不可点） | 同上；宽度 ≥630 时显示 | `CMD_VIDEO_CODEC_FOURCC_MESSAGE` | `:35-40` |
| 14 | 音频编码（不可点） | 宽度 ≥670 时显示 | `CMD_AUDIO_CODEC_TYPE_MESSAGE` | `:42-47` |
| 15 | 输出声道（可点击） | 宽度 ≥690 时显示 | `CMD_AUDIO_OUT_NUM_CHANNEL_MESSAGE` | `:49-54` |
| 16 | 下载速率（网络流） | Tahoma 11，`#777777`；宽度 ≥740 时显示 | `CMD_DOWNLOAD_RATE_MESSAGE` | `:56-57` |

徽标两帧的底色：`Main(485,34)` 常态 `#333333`，`(526,34)` 悬停 `#3b3b3b`。[确认，像素]

**右侧（从左到右阅读；源文件中 Align=Right，先声明的在最右）**：

| # | 元素 | 尺寸 | 命令 | 显示条件 | 证据 |
|---|---|---|---|---|---|
| 1 | CC 字幕开关 | 23x22，左侧间距 8 | `ID_CLOSED_CAPTION`（DownFlag：有按下态） | AutoVisible，且 `VisMinLeft=410` | `:85-86` |
| 2 | 选择流（弹出菜单） | 23x22，间距 8 | `CMD_POPUPMENU_STREAM_SELECT` | AutoVisible，350 | `:77-78` |
| 3 | 360° 菜单 | 23x22，间距 8 | `CMD_POPUPMENU_VIDEO_360VR` | AutoVisible，350 | `:70-71` |
| 4 | 3D 菜单 | 23x22，间距 8 | `CMD_POPUPMENU_VIDEO_3D` | AutoVisible，350 | `:68-69` |
| 5 | 浏览器（聊天区） | 23x22，间距 8 | `CMD_USE_EXPLORER` | `AutoVisible=100`，350 | `:66` |
| 6 | 播放列表开关 | 23x22，间距 8 | `ID_PLAYLIST_WINDOW`（DownFlag） | `VisMinLeft=320` | `:64-65` |
| 7 | 控制面板开关 | 23x22，与右侧间距 10 | `ID_CONTROL_BOX_WINDOW`（DownFlag） | 320 | `:62-63` |
| 8 | 打开 / 关闭附着面板（二选一） | 41x40，贴最右 | `CMD_ATTACH_WINDOW_OPEN` / `CMD_ATTACH_WINDOW_CLOSE` | 320 | `:59-60` |

`VisMinLeft=N` 的含义推断为：按钮的左边缘 x 坐标小于 N 时隐藏（窗口变窄时右侧按钮从左往右依次消失）。`VisParentMinWidth=N` 表示父容器宽度小于 N 时隐藏。`AutoVisible='1'` 表示命令可用时才显示（例如 CC 和流选择只在有对应流时出现）。**[推断：依据属性名与阈值的递增规律]**

```css
.button-row { height: 40px; background: #1e1e1e; display: flex; align-items: stretch; }
.big-btn    { width: 41px; height: 40px; background: #1e1e1e; border-right: 1px solid #000; /* 第 41 列为 #000 */
              color: #f0f0f0; /* 字形很小且居中：play 7x8@(17,16)，pause 8x10@(16,15)，stop/prev/next 8x8@(16,16)，open 8x7@(16,17) */ }
.big-btn:hover    { background: #272727; }
.big-btn:active   { background: #181818; }
.big-btn:disabled { color: #3b3b3b; }
.time-current { font: 11px Tahoma; color: #f0f0f0; margin-left: 10px; cursor: pointer; padding-top: 1px; }
.time-sep, .time-total { font: 11px Tahoma; color: #777777; margin-left: 5px; }
.badge { height: 14px; padding: 0 5px; background: #333333; font: 11px Tahoma; color: #777777; margin-left: 5px; }
.badge:hover { background: #3b3b3b; color: #fae100; }
.icon-btn { width: 23px; height: 22px; color: #777777; margin-left: 8px; align-self: center; }
.icon-btn:hover    { color: #fae100; }
.icon-btn:active, .icon-btn.down { color: #fae100; } /* 第 3 帧与悬停同为 #fae100 —— Main(50,391) */
.icon-btn:disabled { color: #3b3b3b; }
/* 23x22 图标四帧步距 23px（Main y=391 实测像素区间 2-19、25-42、48-65、71-88） */
```

**覆盖或位于视频外、自动隐藏**：常规模式下控制栏在视频之外，皮肤没有声明任何自动隐藏属性。覆盖与自动隐藏只发生在 OSC 模式（`MainSkinAreaOscInside`）或开启“自动隐藏主皮肤”选项时，由程序逻辑控制。隐藏延迟、全屏下的显示条件，皮肤里都没有。[确认/未知]

`BottomVod.txt`（25px）只在 `State='CMD_WEB_VOD'` 时出现，用于网络点播的作者、日期、播放数等信息，本地播放时不显示。[确认] `SD/BottomVod.txt:1`

### 2.4 Logo / 空闲界面与加载界面（Logo.txt）

```css
.idle-logo { position: absolute; inset: 5px; background: #101010; }  /* State=CMD_DEFAULT_LOGO，Logo.txt:11 */
.idle-logo img { /* PotPlayerEng.png，596x442（文件实际 596x443，RGB），98% 像素为 #101010；IsLogo=1，Align=Client */ }
.loading { position: absolute; inset: 5px; }                           /* State=CMD_LOADING_LOGO，:22 */
.loading .spinner { width: 74px; height: 74px; centered; transform: translateY(-30px);
                    /* 12 帧，纵向排列在 Loding.png(0, 0..814) 中，每帧 50ms，CMD_ANIMATION */ }
.loading .message { font: bold 15px var(--ui-font); color: #777777; centered; transform: translateY(30px); } /* :38 */
```

Logo 图片是否缩放（Align=Client 时是拉伸还是居中原尺寸）无法确定，见未知项。

### 2.5 播放列表

#### 2.5.1 附着在主窗口右侧（RightAttach.txt）

- 容器：`Static Align='Right' Contain='CMD_CHAT_PLAYLIST_BROADCAST_TOGGLE' Width='1'`，宽度在运行时决定。[确认] `SD/RightAttach.txt:1`
- 分隔条：宽 3px（`#000`），可拖动（`IsSplitter='1'`）。左侧（视频一侧）至少保留 `MinSplitterSizeLT='318'`，右侧面板至少 `MinSplitterSizeRB='317'`（skin-frame 中为 314）。[确认] `:23`；`SF/RightAttach.txt:23`
- 记忆宽度的配置键：`PlaylistAttachSize1/2`、`ChatAttachSize1/2`、`BroadcastAttachSize1/2`；默认值未知。DLL UTF-16 0x13a7470-0x13a74f0。
- 顶部标签条：高 31px，切片 `Main(519,1176,314x31)`，底色 `#1e1e1e`，九宫格端帽为左 250、右 10。其中有三个标签：直播列表（`CMD_BROADCAST_LIST_TAB_SELECT`）、聊天/浏览器（`CMD_CHAT_TAB_SELECT`）、播放列表（`CMD_PLAYLIST_TAB_SELECT`）。每个标签只在对应面板处于附着状态时显示（`State='CMD_*_ATTACH'`），宽 105、高 31。标签文字为 `#DEFAULT_UI_FONT` 12，水平垂直居中，常态 `#777777`（Main 864,84），悬停和按下 `#e9e9ea`（864,342）。每个标签右上角在 x=98 处有 6x5 的“分离”按钮（`CMD_ATTACH_PLAYLIST` 等）。[确认] `:2-21`
- 标签底图四帧 `Main(0/105/210/315,1176)`：`#1e1e1e`、`#272727`、`#333333`、`#1e1e1e`。推断依次为常态、悬停、选中/按下、禁用。[确认像素/推断语义]

```css
.attach-panel  { width: var(--PlaylistAttachSize, ?); min-width: 317px; }
.attach-splitter { width: 3px; background: #000; cursor: col-resize; }  /* video side min 318px */
.attach-tabs   { height: 31px; background: #1e1e1e; }
.attach-tab    { width: 105px; height: 31px; font: 12px var(--ui-font); color: #777777; text-align: center; }
.attach-tab:hover  { background: #272727; color: #e9e9ea; }
.attach-tab.active { background: #333333; color: #e9e9ea; }
.attach-tab .detach { position: absolute; left: 98px; top: 0; width: 6px; height: 5px; }
```

#### 2.5.2 播放列表皮肤区域（SubCommon.txt:549-629，PlayList.png 668x482）

整体：`WidthLimit='300' HeightLimit='250'`（独立窗口的最小尺寸）；外边距 6/6/6/6。[确认] `:549,565`

模板 `PlayList.png(0,217,320x259)` 自上而下（像素实测）：3px `#000` 边框 → 30px `#1e1e1e` 头部 → 1px `#000` → 30px `#1e1e1e`（相册标签区）→ 1px `#000` → 150px `#3b3b3b` 列表区 → 1px `#000` → 40px `#333333` 底栏 → 3px `#000` 边框。左右边框各 3px `#000`。[确认]

**头部**（`:568-581`，高 34 = 3px 边框 + 30 + 1px 线；九宫格端帽左 100、右 10）：
- 左侧：留白 10px，标题 “Playlist”（`$PLAYLIST_TITLE`），`#DEFAULT_UI_FONT` 11 粗体，`#f0f0f0`（PlayList 518,64）。
- 右侧，从右往左：留白 10，关闭 14x13（`ID_PLAYLIST_WINDOW`），留白 10，置顶图钉 14x13（`CMD_TOPMOST_WINDOW`，两种状态），留白 10，附着到主窗口 14x14（`CMD_ATTACH_PLAYLIST`），留白 10，透明度滑块宽 50（`CMD_TRANS_PL`；轨道图 28x8 黄 `#fae100` / 底 `#1e1e1e`，滑块是 15x8 的药丸形，常态 `#f0f0f0`、悬停 `#fae100`）。

**相册（多列表）标签**（`:554-561`）：

```css
.album-tab        { width: 85px; height: 31px; background: #333333; color: #888888; font-size: 9px; } /* AlbumTabFontSize=9；文字色 PlayList(518,146) */
.album-tab:hover  { background: #3b3b3b; }
.album-tab.active { background: #3b3b3b; color: #f0f0f0; /* 底边无黑线，与列表区连为一体；PlayList(518,130) */ }
.album-tab .close { width: 10px; height: 9px; }                 /* AlbumTabDel (0,121) */
.album-tab-scroll-left, .album-tab-scroll-right, .album-tab-popup { width: 19px; height: 18px; }
.search-icon { width: 15px; height: 16px; }                     /* 位于右侧，SearchIconRight=1 */
```

**列表项颜色**（`:550-553`，在 `PlayList.png` x=518 处采样）：

```css
.pl-list            { background: #3b3b3b; }          /* BkgndColor1 = BkgndColor2 = #3b3b3b → 无斑马纹 */
.pl-item            { color: #aeaeae; }               /* TextColor (518,98) */
.pl-item.played     { color: #888888; }               /* PlayedTextColor (518,114) */
.pl-item.selected   { background: #2b2b2b; color: #f0f0f0; } /* SelBkgndColor (518,14) / SelTextColor (518,64) */
.pl-item.focus      { outline: 1px solid #2b2b2b; }   /* FrameColor (518,14)；推断为焦点框 */
.pl-item.current    { color: #2282f2; }               /* CurTextColor (518,81)，即正在播放的项 */
.pl-grid-line       { color: #3b3b3b; }               /* LineColor = 背景色 → 实际不可见 */
.pl-item            { height: ?; font: ?; }           /* 皮肤未定义；由 sPlayListFontName/Size/Weight 等配置决定 */
```

**底部按钮栏**（`:589-624`，高 35px，切片 `(3,432,314x35)` = 1px `#000` + 34px `#333333`；端帽左 250、右 20）：

| 从左起 | 元素 | 尺寸 | 命令 |
|---|---|---|---|
| | 留白 | 6 | |
| 1-4 | 移到顶部、上移、下移、移到底部 | 每个占宽 25 的槽，图 21x18，向下偏移 1 | `CMD_PLAYLIST_TOP/UP/DOWN/BOTTOM` |
| | 留白 | 6 | |
| 5 | ADD | 自动宽度（最小 35），高 18，左右内边距各 4，端帽 5 | `CMD_PLAYLIST_ADD` |
| | 留白 | 4 | |
| 6 | DEL | 同上 | `CMD_PLAYLIST_DEL` |
| | 留白 | 4 | |
| 7 | SORT | 同上 | `CMD_PLAYLIST_MENU_SORT` |
| | 留白 | 4 | |
| Client | 搜索框 | 底图 110x29 `#282828`，九宫格端帽 2；向下偏移 6，高度减 11（约 24px 高）；输入框内缩 2px | `CMD_PLAY_LIST_SEARCH` |
| 右侧 | 总时长文字（右侧留白 8，左侧留白 4） | `#DEFAULT_UI_FONT` 11，`#aeaeae` | `CMD_PLAYLIST_TOTAL_TIME_MESSAGE` |

```css
.pl-bottom { height: 35px; background: #333333; border-top: 1px solid #000; padding-left: 6px; }
.pl-btn    { height: 18px; min-width: 35px; padding: 0 4px; background: #555555; font: 11px var(--ui-font); color: #aeaeae; }
.pl-btn:hover  { background: #5c5c5c; color: #f0f0f0; }   /* PlayList(518,198) */
.pl-btn:active { background: #444444; color: #b1b1b1; }   /* PlayList(518,180) */
.pl-btn:disabled { background: #2a2a2a; }
.pl-arrow-btn  { width: 21px; height: 18px; background: #555555; color: #aeaeae; } /* 四帧：#555/#5c5c5c/#444/#2a2a2a，字形 #aeaeae/#b1b1b1/#8b8b8b/#575757 */
.pl-search { height: ~24px; background: #282828; }
```

**滚动条**（`SD/Common.txt:62-115` 的 `ScrollBarPlaylist`，图 `ScrollBarPL.bmp` 150x128）：

```css
.pl-scrollbar { width: 20px; background: #0a0a0a; }                 /* PageUp/Down 切片 (0,111/112) */
.pl-scrollbar .arrow { width: 20px; height: 19px; background: #333333; border: 1px solid #161618; }
.pl-scrollbar .thumb { width: 20px; background: #333333; border: 1px solid #161618; /* 端帽 4px，中间 9px 拉伸 */ }
.pl-scrollbar .thumb:hover { /* VertActive 帧 (126,39..)，与常态颜色相同（#333333） */ }
```

注：通用的 `ScrollBar`（`ScrollBar.bmp`，宽 12、箭头长 11）是浅色系（`#ffffff` / `#e6e6e6` / `#bababa`，track 为 `#f4f4f4`），`Common.txt:7-60`。聊天滚动条宽 10，`Common.txt:117-170`。[确认]

### 2.6 右键菜单样式

说明：`Popup.txt` 不是通用的右键菜单，而是直播用的 64x58 小浮窗（`CMD_POPUP_WINDOW`，里面是“分享 / 举报”两个文字按钮，字号 12，常态 `#777777`、悬停 `#fae100`），见 `SD/Popup.txt:2-9`。**通用的皮肤化右键菜单定义在 `Logo.txt:63-66`**（每个主窗口区域都通过 `#include<Logo.txt>` 引入）。`Common.txt` 只定义滚动条。[确认]

`CustomPopupMenuDef`（暗色，默认）`SD/Logo.txt:63-64`，坐标都在 `Main.png` 上：

```css
.ctx-menu { background: #444444; border: 1px solid #000000; } /* Frame (689,307) 56x24：外圈 1px #000，内部 #444444；推断为九宫格拉伸 */
.ctx-item            { color: #bbbbbb; }                  /* TextColor (724,124) */
.ctx-item:hover, .ctx-item.selected { background: #181818; color: #fae100; } /* SelColor (770,312) / SelTextColor (864,99) */
.ctx-item:disabled   { color: #777777; }                  /* DimTextColor (724,99) */
.ctx-separator       { height: 1px?; background: #27282f; } /* LineColor (770,327)；线粗未知 */
.ctx-check { width: 14px; height: 14px; border-radius: 3px; background: #34353f; /* 中间白色对勾 */ } /* Check (644,313) */
.ctx-radio { width: 12px; height: 12px; /* 外环 #34353f，中心点 #2282f2 */ }                          /* Radio (667,313) */
.ctx-scrollbar { /* ScrollbarIndex=1 → 推断为第 1 套非默认滚动条，即 ScrollBarPlaylist（20px 暗色） */ }
.ctx-item { height: ?; padding: ?; font: ?; } /* 皮肤未给出 */
.ctx-submenu-arrow { /* 皮肤未给出图片 → 推断由程序绘制 */ }
```

`CustomPopupMenuAlt`（浅色备选，`:65-66`）：底色 `#ffffff`，边框 `#000`；文字 `#1e1e1e`；选中背景 `#e9e9ea`，选中文字 `#212228`；禁用文字 `#777777`；分隔线 `#dfdfdf`；勾选标记为 14x14 `#2282f2` 方块加白勾；`ScrollbarIndex=2`。对应偏好设置中的“Context menu design / style / Transparency / Size of skinned context menu”（`EN.ini:1015-1018`（Dialog(376) 3289-3292））。[确认]

### 2.7 公共样式

**字体**
- `#DEFAULT_UI_FONT` 是一个占位符。DLL 中这个字面量（UTF-16，0x13a1808）紧挨着 `Language\`、`English.ini`、`StringTable` 这些字符串。语言文件的 `[Common]` 节里有 `DefaultMessageFont`：英文为 `Segoe UI`（`EN.ini:3`），简体中文为 `宋体`（`Chinese(Simplified).ini:4`），韩文 ini 中没有这一项。DLL 里还有 `Segoe UI`（0x13a3540）和 `Malgun Gothic`（0x149e328）。**推断：`#DEFAULT_UI_FONT` 取当前语言的 `DefaultMessageFont`，没有配置时回退到 DLL 内置字体。**
- 底部栏的时间和信息文字固定用 `Tahoma`；音频皮肤的大号时间用 `Arial`（`SD/BottomAudio.txt:6-7`）。[确认]
- 字号用到的值有：9（相册标签、音频信息）、11（时间、徽标、播放列表、控制面板）、12（标签、直播）、13（标题、文件类型、控制面板页签）、15（加载信息，粗体）、33（音频大时间）。
- 字号单位推断为像素高度（字符单元高）。依据是皮肤为 Label 给的 Height 与 FontSize 一致或接近：33→34、15→15、12→16、9→11（`SD/BottomAudio.txt:6-12`）。**[推断]**

**颜色表**（PotPlayer 默认皮肤）

```css
:root {
  --bg-bar: #1e1e1e;        /* 标题栏 / 控制栏 / 面板头 */
  --bg-bar-inactive: #151515; /* 标题栏 AltImage（推断为失去焦点） */
  --bg-video: #101010;      /* 视频区、空闲 logo */
  --bg-list: #3b3b3b;       /* 播放列表 */
  --bg-panel: #333333;      /* 播放列表底栏、控制面板页面、未选中相册标签 */
  --line: #000000;          /* 边框 3px、分隔线 1px */
  --hover-bg: #272727;      /* 大按钮/菜单按钮/标签悬停 */
  --press-bg: #181818;      /* 大按钮按下；右键菜单选中底色 */
  --text: #f0f0f0;          /* 主文字、当前时间、标题、选中项 */
  --text-dim: #777777;      /* 次要文字、总时长、图标常态 */
  --text-list: #aeaeae;     /* 列表文字、按钮文字 */
  --text-played: #888888;
  --accent: #fae100;        /* 黄色：已播放进度、音量、悬停图标、菜单选中文字 */
  --accent-press: #c8b400;
  --accent-disabled: #7d7000;
  --icon-disabled: #3b3b3b;
  --playing: #2282f2;       /* 正在播放项；控制面板滑块填充 */
  --warn: #ff3217;          /* Main(864,129)，直播“举报数” */
}
```

**滑块的几种样式**

| 用途 | 尺寸 | 填充色 / 轨道色 | 滑块 | 证据 |
|---|---|---|---|---|
| 进度条 | 可变宽 × 10，可见线高 2 | `#fae100` / `#000` | 11x10，8px 圆点 | `BottomVideo.txt:109` |
| 音量 | 70x10，可见线高 2 | `#fae100` / `#000` | 11x10 | `:103` |
| 窗口透明度（Slider.png） | 50 宽，图 28x8 | `#fae100` / `#1e1e1e` | 15x8 药丸形 | `SubCommon.txt:580` |
| 控制面板色彩（亮度、对比度等） | 154x12，可见线高 2 | `#2282f2` / `#1e1e1e` | 14x12 圆点 `#f0f0f0` | `SubCommon.txt:259` 与 Control.png 像素 |
| 均衡器（竖向） | 14x70 | `#2282f2` / `#1e1e1e` | 14x12 | `SubCommon.txt:242-253` |

**控制面板**（与我们的画面调节、均衡器、字幕、跳转/倍速插件对应）：`ControlBoxSkinArea`，固定尺寸 301x227，见 `SD/SubCommon.txt:189-369`。
- 头部 34px，底色 `#1e1e1e`；标题 11 粗体 `#f0f0f0`；右侧依次是关闭、置顶、透明度。
- 4 个页签，各 74x31：音频、视频、字幕、播放。页签文字 13：常态 `#777777`，悬停/选中 `#f0f0f0`。页签底色：常态 `#1e1e1e`，悬停或选中 `#333333`。
- 页面区 295x159，底色 `#333333`。
  - 视频页：每行 = 标签（65 宽，右对齐，11，`#f0f0f0`）+ 154 宽滑块 + “默认”按钮（41x18，`#555555`），行距 24px。依次是亮度、对比度、饱和度、色调，下面是柔化、锐化、去块复选框，以及截图按钮。
  - 音频页：10 段均衡器，竖滑块间距 23px，另有主音量和波形音量两个竖滑块。
  - 字幕页：字体选择（160x22）、字号增减、上下左右移动（每个 51x22）、同步的慢 / 快 / 重置（每个 69x22）。
  - 播放页：跳转 -1分 / -5秒 / +5秒 / +1分（每个 51x22，右键分别为 5 分 / 30 秒 / 30 秒 / 5 分）、速度慢 / 快 / 默认（每个 69x22）、A-B 循环。

  [确认]

**通用对话框皮肤**（`WindowSkinArea`，Window.png）：按钮 39x21，标签 70x26，对话框字体 `#DEFAULT_UI_FONT` 9；窗口类型 0/1/2 的皮肤边距为左 3、上 34、右 3、下 3。`SD/SubCommon.txt:726-744`。[确认]

---

## 3. 未知项清单（需要 Windows 实机或更深的反汇编才能确认）

| # | 未知项 | 需要什么才能确认 |
|---|---|---|
| U1 | OSC（On Screen Control）的默认值：Disable 还是开启？这决定首次启动时控制栏是覆盖视频还是在视频外 | 全新安装后查看菜单“皮肤 → On Screen Control”的勾选状态，并截图主窗口 |
| U2 | “自动隐藏主皮肤”（`AutoHideSkin`，CmdID 10369）的默认值，以及隐藏延迟、触发区域 | 实机勾选或不勾选后观察，并用秒表计时；或反汇编 0x13abf38 名称表的使用者 |
| U3 | 全屏时控制栏如何出现：鼠标移到底部才出现还是任意移动都出现、出现与隐藏的时长、是否带动画 | 实机全屏录屏 |
| U4 | 主窗口默认初始尺寸和位置（`VideoWindowRect`），以及 `HeightOffset='-1'` 的含义 | 全新安装后截图并读取注册表 `HKCU\Software\DAUM\PotPlayer64` |
| U5 | 附着播放列表的默认宽度（`PlaylistAttachSize1/2`），首次启动时面板是否默认打开 | 全新安装后实测；读取注册表 |
| U6 | 播放列表的行高、默认字体和字号（`sPlayListFontName/Size/Weight`），列表有哪些列（标题、时长）及各自对齐方式 | 实机截图，在 100% DPI 下量像素；或反汇编设置默认值 |
| U7 | 右键菜单的项高、左右内边距、字体字号、分隔线粗细与上下留白、子菜单箭头的样式（皮肤只给了颜色和勾选图） | 实机截图右键菜单（100% DPI），量像素 |
| U8 | 时间文字的格式（`HH:MM:SS` 还是 `MM:SS`、短片是否省略小时、是否显示毫秒），点击当前时间后是否切换为剩余时间 | 实机播放长度不足 1 小时和超过 1 小时的文件各截图，并点击当前时间 |
| U9 | 进度条 `AltImage`（`#444444` 2px）的用途：缓冲进度、不可跳转时的样式，还是其他 | 实机播放网络流或不可 seek 的源时观察 |
| U10 | `AltImageLeft/Top` 是否确实对应“窗口失去焦点”（标题栏 `#151515`） | 实机切换窗口焦点，对比截图 |
| U11 | 皮肤 `FontSize` 的确切单位（像素高度还是磅值）以及 DPI 缩放规则 | 实机在 100% 和 150% DPI 下分别截图，测量 “Tahoma 11” 的字高 |
| U12 | Logo 图（596x442）在 Align=Client 下是居中原尺寸还是按比例缩放 | 实机调整空闲窗口大小并截图 |
| U13 | 播放/暂停、最大化/还原、打开/关闭附着面板这些按钮对的显示切换规则（推断是按状态二选一） | 实机观察 |
| U14 | 附着模式下面板具体显示哪些部分（是否包含独立窗口的 34px 头部，还是只显示 `CMD_PLAYLIST_HOLDER` 及以下） | 实机截图附着状态下的播放列表 |
| U15 | `VisMinLeft` 和 `VisParentMinWidth` 的精确判断规则（按左边缘还是按父宽度），以及窄窗口下按钮消失的顺序 | 实机把窗口从 800px 拖窄到 320px 并逐步截图 |

**未知项数量：15**
