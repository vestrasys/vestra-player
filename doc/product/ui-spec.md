# 界面规格

本文规定 Vestra Player 的界面：尺寸、颜色、几何和基本行为，发给 API 维护者和插件作者。交互细节（状态、时序、各模式下的行为）见[交互规格](interaction-spec.md)，两者冲突时以交互规格为准；菜单树的完整清单见[菜单与界面归属](menus.md)；功能范围见[功能需求](feature-requirements.md)。改动按[社区工作流](../process/community-workflow.md)进行。

## 0. 怎么读

- **编号**：`UI-<区域>-<序号>`，例如 `UI-BAR-03`。代码注释和其它文档按编号和小节号引用本文，编号不改、不复用。负责方写成 `宿主`（Rust 宿主与 `host.slint`）、`SDK`（插件 API；核心能力汇总在 §10）、或插件名。
- **单位**：Slint 逻辑像素。macOS 上 1 逻辑像素 = 1 点；Windows 上按 96 DPI 的 1x 像素，随系统缩放。
- **颜色**：写主题 token 名（§2.1），括号里是默认配色 `vestra` 的值。插件 `.slint` 只用 token，不写颜色值。
- **样式代码**：用 CSS 写，便于和 Slint 属性逐项对照（`height` ↔ `height`，`background` ↔ `background`，`gap` ↔ `spacing`，`padding` ↔ `padding`）。
- **标题栏**：主窗口使用系统标题栏（macOS 保留红黄绿按钮）。自绘标题栏的计划见交互规格 IA-CORE-07。快捷键的 Ctrl 在 macOS 上同时接受 Cmd。

## 1. 设计决定

| # | 问题 | 决定 |
| --- | --- | --- |
| D1 | 视觉身份 | 方向 A「V-Line × Aurora」（2026-10-24 用户选定，[方案](identity/proposal.md)）：配色取标志（`src/assets/brand/logo.png`）的完整色谱，洋红强调 `#f72aa6`、青色表示正在播放、进度用琥珀→洋红→紫→青渐变；默认深色 `vestra`，可选浅色 `vestra-light`（视频区仍是深色）；图标是 V-Line 圆头线条（§2.5）；圆角 4/8px（§2） |
| D2 | 画面、声音、字幕的调节工具 | 三个独立的插件小窗口，都按 §7.1 的面板页规格；不做把它们收进同一窗口的容器（C12） |
| D3 | 截图 | 「保存当前画面」Ctrl+E 直接保存到截图文件夹并给出屏幕提示，不弹确认；「另存为…」才弹保存对话框。连拍、导出片段是独立对话框 |
| D4 | 菜单入口 | 主窗口内没有菜单按钮，菜单只靠右键；不做 macOS 顶部菜单栏 |
| D5 | 调试信息 | 不常驻显示。「播放信息」插件用 Tab 开关一个叠加在视频上的信息层，默认关（§9.2） |
| D6 | 播放列表停靠 | 默认附着在主窗口右侧，可分离为浮动窗口再附着回去（C14） |

## 2. 全局视觉规范（宿主提供，所有插件使用）

### 2.1 颜色表

宿主有两套配色，都叫 Aurora、取自标志的完整色谱：`vestra`（深色，默认）和 `vestra-light`（浅色，2026-10-24 加入）。颜色由宿主下发给每个视图（主窗口外壳与面板、浮动侧栏、工具窗口、选项窗口），插件通过 `@vestra/theme.slint` 的 `Theme` 读取；`Theme` 的颜色是只读的 `out` 属性。用户在「设置 ▸ 外观 ▸ Vestra 深色 / Vestra 浅色」或选项窗口「外观」页切换，所有窗口立即生效，选择跨启动保存。

两套配色的色值见 SDK 文档站 [Reference: Theme → Colours](https://vestrasys.github.io/vestra-player-sdk/documents/Theme.html#colours)（SDK `THEME_PALETTES`，与宿主 `theme.rs` 由 host-agreement 核对）；下表只记播放器界面怎么用每个 token。

| token | 用途 |
| --- | --- |
| `bg-bar` | 控制栏、页签条、窗口背景 |
| `bg-video` | 视频区、空闲画面（两套都是深色） |
| `bg-list` | 播放列表内容区、浮动面板、选项树 |
| `bg-panel` | 工具页面、列表底栏、菜单、悬停提示 |
| `line` | 1px 分隔线与外框、进度条和滑块的轨道、菜单分隔线、侧栏分隔条 |
| `hover-bg` | 悬停底色（底栏按钮的圆角底、页签）；搜索框底色 |
| `press-bg` | 按下底色 |
| `selected-bg` | 列表选中行、菜单当前项、选项树选中项 |
| `text` | 主文字、图标常态、面板滑块 thumb |
| `text-dim` | 次要文字、小图标常态、总时长、快捷键文字 |
| `text-list` | 列表文字、按钮文字、菜单文字 |
| `text-played` | 列表中播放过的项 |
| `accent` | 悬停图标、开启状态、勾选框、书签、当前项竖条、滑块外环 |
| `accent-press` | 按下的强调色 |
| `accent-disabled` | 禁用的强调色 |
| `icon-disabled` | 禁用图标、禁用滑块 |
| `playing` | 正在播放的列表项及其左侧竖条；AB 循环区间与标记 |
| `button-bg` | 面板内小按钮、标签 |
| `button-hover` |  |
| `button-press` |  |
| `button-disabled` |  |
| `danger` | 失败状态、删除类按钮 |
| `accent-fill` | 进度类填充（进度条已播放段、音量、滑块）、页签和 OSD 的渐变短线 |

对比度（WCAG）：深色 `text`/`bg-bar` 16.6:1，`accent`/`bg-bar` 5.1:1，`playing`/`bg-list` 7.7:1；浅色 `text`/`bg-bar` 16.1:1，`accent`/`bg-bar` 4.6:1，`playing`/`bg-list` 5.0:1。

固定颜色（不随配色）：

- 视频区上的东西（屏幕提示、信息层，§9）：白字 `#ffffff` / `#f5f3fa`，半透明深色底板 `#0b0a10c4`。视频区在两套配色里都是深色，所以这里不用 `Theme.text`（浅色配色里是深色字）。
- 强调色底上的勾（菜单勾选框、复选框、任务列表）是白色 `#ffffff`；底栏滑块的芯是白色。

std-widgets 控件（`LineEdit`、`ComboBox`、`CheckBox`、`ScrollView`、`ListView` 的滚动条等）：宿主以 `fluent-dark` 风格编译所有 Slint，选浅色配色时把 std-widgets 的 `Palette.color-scheme` 改为 `light`（`ui/palette.slint` 的 `VestraPalette.color-scheme`），这些控件随之变成 fluent 的浅色，切回深色时复原，不需要重新编译。仍有的差别：fluent 控件用自己的中性灰和蓝色强调（焦点下划线、复选框），不是 Aurora 的颜色，两套配色都一样。

### 2.2 字体

| 用途 | 规格 |
| --- | --- |
| 界面默认字体 | 各平台的系统 UI 字体（macOS 中文为 PingFang SC、Windows 为微软雅黑），宿主不指定字体族；缺字时由 Slint 按字符回退（2026-10-22 用户裁决） |
| 等宽字体（`Theme.font-mono`） | 宿主按平台设置：macOS Menlo、Windows Consolas、Linux DejaVu Sans Mono |
| 时间、徽标 | 11px，数字等宽（`font-feature: tnum`） |
| 字号档位 | 只用这几档：9（`font-xs`，极小标签）、11（`font-body`，正文、列表、时间）、12（`font-tab`，页签、菜单）、13（`font-title`，窗口标题、页签标题） |

### 2.3 控件尺寸

```css
button          { height: 21px; min-width: 66px; padding: 0 8px; }   /* 对话框普通按钮；首选宽度 = max(最小宽度, 文字宽 + 内边距) */
button.primary  { height: 24px; }                                    /* 底部主操作按钮 */
button.small    { height: 18px; min-width: 41px; }                   /* 面板内「默认」按钮 */
input, select   { height: 21px; }
checkbox-row    { height: 18px; }
label           { height: 14px; }                                    /* 表单左列标签右对齐 */
.form-row       { gap: 26px; }
.dialog         { padding: 14px; }
.control        { border-radius: 4px; }                              /* radius-control：按钮、菜单项、悬停提示、页签悬停底、面板滑块 thumb */
.panel          { border-radius: 8px; }                              /* radius-panel：菜单、浮层、底栏按钮的悬停底、OSD 底板 */
```

控件库 `@vestra/widgets.slint` 提供 `Button`、`Slider`、`ToolTipArea` 等，尺寸取上面的 token；图标在 `@vestra/icons.slint`（§2.5）。

### 2.4 宿主提供的主题

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-THEME-01 | 提供可导入的主题 global `Theme`（§2.1–2.3 的全部 token），插件 `.slint` 用 `import { Theme } from "@vestra/theme.slint"` 取值，不写颜色值（主题文件除外） | 宿主 + SDK（C2） |
| UI-THEME-02 | 所有插件窗口跟随配色：控件库按 `Theme` 着色；std-widgets 在深色配色下是 fluent 深色，在浅色配色下由宿主切到 fluent 浅色（§2.1）；视频区两套配色都是深色 | 宿主（C2） |
| UI-THEME-03 | 默认字体为系统 UI 字体，CJK 按字符回退，中英混排无方框字 | 宿主 |

### 2.5 图标（V-Line）

所有图标来自 `@vestra/icons.slint`（宿主控件库，SDK 1.5 起带只读副本）：`Icon { name: "play"; color: Theme.text; }`。源文件是 [icons-v-line.json](identity/icons-v-line.json)。

- **线条**：只描边、不填充；圆头（`round` cap）、圆角（`round` join）；线宽 1.5px，12px 以内的小图标 1.25px（排序箭头 1.1px）。
- **V 形**：标志的 V 贯穿整套图标：快退、快进是 « »，上移、下移是 ∧ ∨，置顶、置底是 ∧ ∨ 加横线，子菜单箭头、下拉箭头、滚动箭头、排序箭头都是小 V，播放列表开关的第三行末尾带一个小 V。
- **绘制框**：每个图标有自己的绘制框（`Icons.box-width` × `Icons.box-height`），就是它的默认大小；和被替换的旧图标完全相同，所以按钮和行的布局不变。线条离框边至少半个线宽，不出框。
- **颜色**：图标颜色由调用方给（`color`），规则与旧图标相同：常态 `text` 或 `text-dim`，悬停/开启 `accent`，按下 `accent-press`，禁用 `icon-disabled`；强调色底上的勾是白色。
- **不用字体字符当图标**：✓ › ▲ ▼ ▾ × ▶ 都换成了图标（不同系统的字体画出来不一样）。
- **清单**：图标名与绘制框见 SDK 文档站 [Reference: Theme → Icons](https://vestrasys.github.io/vestra-player-sdk/documents/Theme.html#icons)。

## 3. 主窗口外壳（宿主）

布局：右侧附着面板（播放列表）占满窗口全高，与视频之间是 3px 可拖分隔条；底部控制栏 62px（22 + 40）**只在视频下方**，不延伸到播放列表下方，也不覆盖画面（全屏除外）。视频贴边，四周没有空隙。

```css
.window   { width: 780px; height: 671px; min-width: 320px; min-height: 182px; background: var(--bg-video); }
.video    { flex: 1; min-width: 318px; background: var(--bg-video); }
.bottom   { height: 62px; under: video-only; }
.splitter { width: 3px; background: var(--line); cursor: ew-resize; }
.sidebar  { width: 317px; min-width: 317px; full-height: true; }      /* 默认宽 317，拖动后记住 */
.floating-sidebar { width: 300px; height: 408px; min-width: 300px; min-height: 250px; }
```

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-SHELL-01 | 视频上方没有任何控件（没有命令按钮、说明文字行） | 宿主 |
| UI-SHELL-02 | 没有常驻的诊断文字；播放信息由播放信息插件按需显示（§9.2） | 宿主 |
| UI-SHELL-03 | 视频贴边，背景 `bg-video` | 宿主 |
| UI-SHELL-04 | 默认窗口 780×671（内容区），最小 320×182（高度的变化见交互规格 IA-CORE-13） | 宿主 |
| UI-SHELL-05 | 侧栏占满视频右侧的全部高度，底栏只在视频下方：打开播放列表时，列表底边与窗口底边对齐，底栏左右端与视频对齐 | 宿主（composition） |
| UI-SHELL-06 | 侧栏与视频之间 3px 可拖分隔条（`line`）；视频至少 318px，侧栏至少 317px；拖动后的宽度跨启动保存 | 宿主 |
| UI-SHELL-07 | 侧栏默认宽 317px | 宿主 |
| UI-SHELL-08 | 窗口宽度放不下视频 318px + 分隔条 + 侧栏 317px 时，保持用户的显示选择：先压缩视频到 318px，再不够才隐藏侧栏，菜单勾选仍为「显示」 | 宿主 |
| UI-SHELL-09 | 全屏：控制栏覆盖在视频底部，shown only while the pointer is in its band at the bottom edge and hidden 500 ms after it leaves (FS-BAR-1)；播放时同时隐藏鼠标指针（「播放 ▸ 播放时隐藏鼠标指针」，默认开）。全屏时播放列表的弹出方式见交互规格 G-01 | 宿主 |

## 4. 底部控制栏（基础播放插件 playback）

两行，总高 62px，全部是图标按钮，没有文字按钮。

```css
/* 进度条行 */
.seek-row  { height: 22px; background: var(--bg-bar); border-top: 1px solid var(--line); /* 只有上沿一条线 */
             padding: 0 10px 0 5px; }
.seekbar   { flex: 1; height: 10px; margin: 0 10px; }            /* 10px 点击区 */
.seekbar .track  { height: 2px; border-radius: 1px; background: var(--line); }
.seekbar .played { height: 2px; border-radius: 1px; background: var(--accent-fill); } /* 不可跳转时 icon-disabled */
.seekbar .thumb  { width: 8px; height: 8px; border-radius: 50%; background: #ffffff; border: 2px solid var(--accent); } /* 按下时环为 accent-press */
.seekbar .mark   { width: 1px; height: 6px; }   /* 章节 text-dim、书签 accent、标记 playing；AB 循环为 2px playing 带，两端 1×8 */
.mute-btn  { width: 13px; height: 10px; color: var(--text-dim); margin-left: 10px; } /* 悬停 accent，按下 accent-press */
.volume    { width: 70px; height: 10px; margin-left: 4px; }      /* 2px 线，accent-fill 填充，同款 thumb（白芯 + 2px accent 环） */

/* 按钮行 */
.button-row { height: 40px; background: var(--bg-bar); }
.big-btn    { width: 41px; height: 40px; color: var(--text); }     /* 按钮之间没有分隔线；图标在左 40px 内居中 */
.big-btn::hover-plate { left: 4px; top: 4px; width: 32px; height: 32px; border-radius: 8px; }
            /* 悬停时圆角底 hover-bg，按下 press-bg；禁用图标 icon-disabled */
/* 左→右：播放/暂停（按状态二选一）| 停止 | 后退（右键：上一个）| 前进（右键：下一个）| 打开 */
.time      { margin-left: 10px; font-size: 11px; }  /* 「00:12」text + 「 / 15:00」text-dim；只有当前时间可点击，切换已播放/剩余 */
/* 右端：播放列表开关 */
.icon-btn  { width: 23px; height: 22px; color: var(--text-dim); margin-right: 8px; } /* 悬停或开启 accent，按下 accent-press */
```

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-BAR-01 | 两行：进度条行 22px + 按钮行 40px，背景 `bg-bar`，只有上沿一条 1px `line`（与视频分开），总高固定 62px | playback |
| UI-BAR-02 | 底栏不显示来源路径和状态文字。来源名在窗口标题（C9），状态用屏幕提示（C5） | playback + 宿主 |
| UI-BAR-03 | 进度条按上方 CSS：10px 点击区内 2px 圆头线，未播放 `line`，已播放 `accent-fill`；8px 圆形 thumb，白芯 + 2px `accent` 环，按下时环为 `accent-press` | playback |
| UI-BAR-04 | 静音图标 13×10 + 音量条 70×10 在进度条行最右端；音量条固定 70px，不拉伸 | playback |
| UI-BAR-05 | 按钮行左侧 5 个 41×40 图标按钮：播放/暂停、停止（关闭来源）、后退、前进、打开；在后退/前进上右键 = 上一个/下一个 | playback（上一个/下一个用 C6） |
| UI-BAR-06 | 图标用 `@vestra/icons.slint`（§2.5）；常态 `text`，禁用 `icon-disabled`；悬停/按下点亮按钮中间 32×32 的圆角底，按钮之间没有分隔线 | playback |
| UI-BAR-07 | 时间是文字标签 `当前 / 总长`：当前 `text`，斜杠和总长 `text-dim`，11px，紧跟打开按钮后 10px；点击当前时间在「已播放 / 剩余」之间切换 | playback |
| UI-BAR-08 | 底栏没有倍速按钮和内嵌编辑框。倍速用菜单与 Z/X/C；跳转用「跳转到…」对话框（G，§7.5）；音量用滑块、↑/↓、M | playback |
| UI-BAR-09 | 按钮行右端 23×22 播放列表开关（F6），开启时 `accent`，与菜单勾选、侧栏状态一致 | playback 放按钮，读取与切换侧栏状态（C7） |
| UI-BAR-10 | 窗口窄于 320px 时右侧按钮隐藏。信息徽标（HW/SW、编码、声道等）见交互规格 IA-PB-05 | playback |
| UI-BAR-11 | 载入失败时状态写进屏幕提示；重试是菜单命令「重开当前/最后文件」（Ctrl+Y） | playback |

## 5. 播放列表（播放列表插件 playlist + 宿主）

### 5.1 开关与停靠

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-PL-01 | 开关命令「播放列表」带勾选，快捷键 F6，在右键菜单顶层的窗口入口组（§6.2）；列表获得焦点时 F6 也关闭列表 | playlist（菜单位置）+ 宿主（F6，C3） |
| UI-PL-02 | 底栏开关按钮（UI-BAR-09）、菜单勾选、页签条上的关闭按钮三者状态一致 | playlist + playback（C7） |
| UI-PL-03 | 默认显示，附着在右侧；显示状态、停靠状态和宽度跨启动保存 | 宿主（持久化，C8） |
| UI-PL-04 | 可分离为浮动窗口（默认 300×408，最小 300×250）、再附着回主窗口（C14）；分离/附着按钮在页签条右端 | 宿主（C14）+ playlist（按钮） |

### 5.2 列表外观

```css
.pl-tabs   { height: 31px; background: var(--bg-bar); }          /* 只有一个「播放列表」页签时也显示 */
.pl-tab    { width: 105px; height: 31px; font-size: 12px; color: var(--text); }  /* 无色块；文字下方居中一条 28×2 accent-fill 圆头短线（底边上 1px） */
.pl-close  { width: 14px; height: 13px; }                        /* 页签条右端 */
.pl-list   { background: var(--bg-list); }                       /* 无斑马纹、无卡片、行无圆角 */
.pl-item   { height: 20px; padding: 0 6px; font-size: 11px; color: var(--text-list); } /* 单行：序号. 标题 … 时长 */
.pl-item.selected { background: var(--selected-bg); color: var(--text); }
.pl-item.current  { color: var(--playing); }                     /* 正在播放；左侧 2px playing 竖条（上下各留 3px，圆头） */
.pl-item.played   { color: var(--text-played); }                 /* 播放过 */
.pl-bottom { height: 35px; background: var(--bg-panel); border-top: 1px solid var(--line); padding-left: 6px; }
/* 左→右：置顶 上移 下移 置底（每个 25px 槽，图标 14×12）| 6 | 添加 | 4 | 删除 | 4 | 排序 | 搜索框（高 24，hover-bg；窄时折叠为放大镜图标）| 总时长 11px text-list */
.pl-btn    { height: 18px; width: 41px; border-radius: 4px; background: var(--button-bg); color: var(--text-list); }
           /* 图标标签 add / remove / sort（10×10）居中；悬停提示「添加」「删除」「排序」 */
```

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-PL-05 | 顶部 31px 页签条（单个「播放列表」页签，105px 宽），没有大标题；关闭按钮在页签条右端（14×13 图标）。侧栏有别的插件的面板时，页签条改由宿主画（同样式，每个面板一个页签，播放列表在第一个，按钮作用于整个侧栏），播放列表隐藏自己的页签条（[panel-slots.md](../architecture/panel-slots.md)） | playlist + 宿主（页签） |
| UI-PL-06 | 列表项单行：`序号. 标题` 左对齐，时长右对齐，背景 `bg-list`，无卡片无圆角，行高 20px | playlist |
| UI-PL-07 | 状态用颜色表达：正在播放 `playing`（加左侧 2px 竖条）、播放过 `text-played`、选中 `selected-bg` 底；没有状态文字行。失败用屏幕提示 + 行尾小图标 | playlist |
| UI-PL-08 | 列表上方没有按钮行。上一项/下一项归底栏右键和 PgUp/PgDn，播放由双击/Enter 完成，添加归底栏「添加」按钮的弹出菜单 | playlist |
| UI-PL-09 | 底部 35px 按钮栏：置顶/上移/下移/置底四个图标按钮（25px 槽）+ 添加 / 删除 / 排序三个 41×18 的图标按钮（＋ − ⇅，有悬停提示）+ 搜索框（约 24px 高）+ 右侧列表总时长 | playlist |
| UI-PL-10 | 底部没有路径和消息文字；完整路径是鼠标悬停提示，消息用屏幕提示 | playlist（悬停提示用 C2 的 `ToolTipArea`） |
| UI-PL-11 | 滚动条随配色（std-widgets 的深色或浅色，§2.1） | playlist（C2） |

### 5.3 列表交互

| 编号 | 操作 | 要求 | 负责方 |
| --- | --- | --- | --- |
| UI-PL-12 | 双击条目 | 播放 | playlist |
| UI-PL-13 | Enter / P | 播放所选 | playlist + 宿主（列表焦点下的按键，C3） |
| UI-PL-14 | Delete | 移除所选 | 同上 |
| UI-PL-15 | Ctrl+↑/↓、Ctrl+Home/End | 上移/下移/置顶/置底 | 同上 |
| UI-PL-16 | Ctrl+A、Shift/Ctrl+点击 | 多选；DEL、移动作用于所选 | playlist |
| UI-PL-17 | 拖动 | 拖动重排（搜索过滤时关闭） | playlist |
| UI-PL-18 | 列表右键菜单 | 本插件的命令，与主菜单同一样式（清单见 [menus.md](menus.md) 的列表右键菜单表） | playlist（C11） |
| UI-PL-19 | 列表结束 | 默认播完停止；「播放 ▸ 播放列表」下有循环（不循环/全部/单个，单选）和随机（勾选），默认关 | playlist |
| UI-PL-20 | 打开新文件时 | 按「播放 ▸ 播放列表 ▸ 打开文件时」的设置处理列表，默认值见交互规格 IA-PL-01 | playlist |

## 6. 右键菜单

### 6.1 结构原则

- 常用的直接命令（打开文件、关闭、全屏、窗口入口）放在顶层；领域功能放子菜单。
- 没有「文件」「视图」这两个子菜单：打开类放在「打开 ▸」，窗口开关直接放顶层。
- 命令按 `group` 分组，组间画分隔线；组内按 `order` 排序（C4）。

### 6.2 顶层结构

```
打开文件...            Ctrl+O / F3      playback
打开 ▸                                  playback、playlist、subtitles、audio、history 等（打开链接 Ctrl+U、打开文件夹 F2、打开字幕 Alt+O、最近播放 ▸ …）
关闭                    F4               核心
重开当前/最后文件       Ctrl+Y           playback
复制到剪贴板 ▸                           playback
——
播放 ▸                                  核心、playback、playlist、subtitles、window（播放/暂停 Space、跳转 ▸、跳转到... G、速度 ▸、上一个 PgUp、下一个 PgDn、播放列表 ▸ …）
字幕 ▸                                  subtitles
——
视频 ▸                                  video、capture
——
声音 ▸                                  核心（音量 ↑/↓、静音 M）、audio
模拟器 ▸                                nes
——
全屏                    Enter            宿主
屏幕 ▸                                  window
——
播放列表 ✓              F6               playlist
播放信息 ✓              Tab              info（§9）
播放属性...             Ctrl+F1          info
插件工坊...                              workshop
插件商城...                              store
——
设置 ▸                                  preferences、hotkeys、plugins、visualizer（选项... F5、快捷键...、外观 ▸ …）
——
关于 Vestra Player...   F1               info
——
退出                    Cmd+Q            宿主
```

子菜单的完整内容与各项的插件归属见 [menus.md](menus.md)。

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-MENU-01 | 各插件的命令路径与快捷键按 §6.2；新增或移动入口时同步更新 [menus.md](menus.md) | 各插件 |
| UI-MENU-02 | 顶层可放直接命令（不只是子菜单），有分隔线和分组顺序（C4） | 宿主 + SDK（C4） |
| UI-MENU-03 | 没有隐藏底栏的命令；全屏时底栏自动隐藏（UI-SHELL-09） | playback |
| UI-MENU-04 | 「字幕 ▸ 显示字幕」是勾选项，Alt+H | subtitles |
| UI-MENU-05 | 速度菜单：「正常」Z、「减速 0.1」X、「加速 0.1」C，另有固定倍率单选组（与「正常」同组）和「自定义…」 | 核心 + playback |
| UI-MENU-06 | 勾选项用勾选框，互斥选项（倍速、循环方式、截图格式、配色）用单选圆点 | 宿主（C4） |

### 6.3 样式

```css
.ctx-menu  { background: var(--bg-panel); border: 1px solid var(--line); border-radius: 8px; padding: 2px;
             box-shadow: 0 0 8px var(--menu-shadow); }   /* vestra #00000080，vestra-light #00000033（theme.rs，MenuChrome.shadow） */
.ctx-item  { height: 24px; border-radius: 4px; padding: 0 8px 0 4px; font-size: 12px; color: var(--text-list); }
.ctx-item.lit      { background: var(--selected-bg); color: var(--text); }   /* 悬停或键盘当前项；左边缘 2×14 accent 竖条（y 5） */
.ctx-item:disabled { color: var(--text-dim); }
.ctx-shortcut      { font-size: 12px; color: var(--text-dim); text-align: right; }
.ctx-arrow         { width: 10px; }                                          /* 子菜单箭头：submenu 图标 5×8，x 3 */
.ctx-sep   { height: 7px; } .ctx-sep::after { height: 1px; margin: 0 8px; background: var(--line); }
.ctx-check { width: 14px; height: 14px; border-radius: 4px; background: var(--accent); /* 白色 menu-check 图标 */ }
.ctx-radio { width: 12px; height: 12px; border-radius: 50%; border: 1.5px solid var(--accent); /* 中心 6px accent 圆点 */ }
.ctx-scroll-arrow  { height: 12px; }   /* 放不下时上下各一条，scroll-up / scroll-down 图标 8×5 */
.ctx-tip   { max-width: 360px; padding: 5px 8px; background: var(--bg-panel); border: 1px solid var(--line); border-radius: 4px; font-size: 11px; }
```

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-MENU-07 | 菜单配色、圆角、快捷键文字按上方 CSS。快捷键按系统习惯书写（`platform::KEY_LABELS`）：Windows、Linux 写 `Ctrl+Shift+S`、`Alt+→`；macOS 用符号，按 ⌃⌥⇧⌘ 顺序、不加「+」（`⇧⌘S`、`⌥→`），`Ctrl` 绑定写成 ⌘（它在 macOS 上响应 ⌘），Esc、Enter、Tab、退格、Del、PgUp、PgDn、Home、End 写成 ⎋ ↩ ⇥ ⌫ ⌦ ⇞ ⇟ ↖ ↘。本文与 menus.md 的快捷键一律用 Windows 写法 | 宿主 |
| UI-MENU-08 | 项高 24px，字号 12px；分隔行 7px（中间 1px 线）；行高、分隔、内边距、箭头四个常量在 `menu-view.slint` 与 `menu.rs` 保持一致 | 宿主 |

## 7. 工具窗口（画面、声音、字幕、截图等插件）

### 7.1 面板页规格

画面调节、音频工作台、字幕微调用同一套小窗口规格，改动立即生效，没有「应用」「确定」，每项有独立的「默认」复位：

```css
.tool        { width: 301px; background: var(--bg-bar); }          /* 内容区左右各 8px，可用宽 285 */
.cbox-tabs   { height: 31px; } .cbox-tab { width: 74px; font-size: 12px; color: var(--text-dim); }
             /* 选中：text，文字下 28×2 accent-fill 短线；未选中悬停：内缩 (3,4) 的 radius-control 圆角底 hover-bg */
.cbox-page   { background: var(--bg-panel); }
.row         { height: 24px; }                                       /* 行距 24 */
.row label   { width: 65px; text-align: right; font-size: 11px; color: var(--text); }
.row slider  { width: 154px; height: 12px; }  /* 2px 线，轨道 line，填充 accent-fill，thumb 14×12 text、圆角 radius-control */
.row default { width: 41px; height: 18px; }
.toggle      { width: 46px; height: 22px; }                          /* 翻转、旋转等成组按钮 */
.choice      { width: 51px; height: 22px; } .choice.wide { width: 69px; }
.eq-slider   { width: 14px; height: 70px; gap: 23px; }               /* 竖向均衡器滑块 */
```

### 7.2 窗口清单

| 插件 | 窗口 | 尺寸 |
| --- | --- | --- |
| video | 画面调节（页签） | 301×245 |
| video | 自定义比例 | 240×98 |
| video | 着色器列表 | 301×260 |
| audio | 音频工作台（页签） | 301×260 |
| subtitles | 字幕微调、字幕设置 | 宽 301 |
| subtitles | 手动调整同步 | 350×121 |
| capture | 连拍、导出任务 | 宽 438 |
| capture | 导出片段 | 宽 400 |
| capture | 截图设置 | 宽 360 |
| playback | 跳转到、时间跨度设置等输入框 | 宽 357 |
| playback、playlist、store | 输入链接 | 首选宽 397，最小 260 |
| marks | 章节/书签 | 380×420，最小 320×300 |
| info | 播放属性 | 460×420，最小 400×320 |

### 7.3 共同要求（所有工具窗口）

插件作者的规则见 SDK 文档站 [Interface guidelines → Tool windows](https://vestrasys.github.io/vestra-player-sdk/documents/Interface_guidelines.html#tool-windows)；本表是内置插件的验收项。

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-TOOL-01 | 窗口内部没有大号标题（窗口标题栏已有标题） | 所有工具窗口插件 |
| UI-TOOL-02 | 主题样式（随配色）和 §2.3 控件尺寸：按钮 21px、输入框 21px、表单行距 24–26px、内边距 8–14px、正文 11px | 所有工具窗口插件（C2） |
| UI-TOOL-03 | 滑块用控件库的面板样式：2px 线、`accent-fill` 填充、thumb 14×12 | 所有用滑块的插件 |
| UI-TOOL-04 | 改动立即生效 + 每项「默认」按钮：不出现「应用」「确定」 | video、audio、subtitles |
| UI-TOOL-05 | 状态与错误文字一句话（原文英文，按界面语言显示）；不显示调试信息（任务 id、修订号、generation） | 所有插件 |

### 7.4 各窗口规格（D2：独立窗口，按 §7.1）

```css
/* 画面调节（video） */
.picture { width: 301px; height: 245px; }
.picture .row { label: 65px; slider: 154x12; default-btn: 41x18; gap: 6px; }  /* 行宽 272 */
/* 分页签；色彩页的行：亮度、对比度、饱和度、色调、Gamma */

/* 音频工作台（audio） */
.audio { width: 301px; height: 260px; }
.audio .tabs { tab: 74x31; }
.audio .eq   { vertical-sliders: 14x70; gap: 23px; }
.audio .toolbar { enable-check: 14px box + 11px text; preset: popup-menu; }

/* 字幕微调（subtitles） */
.subtitle { width: 301px; }
/* 轨道下拉 + 加载；字号 − + 默认（51×22、41×22）；位置；同步；颜色/描边/字体在「字幕设置…」 */
```

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-TOOL-06 | 画面调节按上方规格：行布局「标签 + 滑块 + 默认按钮」，没有大标题和状态长文；窗口不超过 320×280 | video |
| UI-TOOL-07 | 音频工作台按上方规格：竖向均衡器、预设弹出菜单；窗口不超过 320×280 | audio |
| UI-TOOL-08 | 字幕分为「字幕微调」小窗（同上规格，不超过 320×280）和「字幕设置…」（字体、颜色、描边、阴影、底框、边距、同步步长、备用编码） | subtitles |
| UI-TOOL-09 | 快捷键：画面 Q（复位）、W/E 亮度、R/T 对比度、Y/U 饱和度；均衡器 Shift+E；字幕 Alt+H、`>` `<` `/` 同步；按键生效并有屏幕提示 | video、audio、subtitles（C3） |

### 7.5 对话框类窗口

| 编号 | 对话框 | 要求 | 负责方 |
| --- | --- | --- | --- |
| UI-DLG-01 | 跳转到（G） | 独立小窗（宽 357），一组时间定位输入 | playback |
| UI-DLG-02 | 打开链接（Ctrl+U） | 所有「输入链接」窗口用控件库的 `UrlInputRow`：输入框 +〔打开〕〔取消〕，回车确定、Esc 取消；不做历史列表。打开链接、列表「添加链接…」、插件商城和链接类插件（YouTube、Bilibili）共用 | playback、playlist、store、链接类插件 |
| UI-DLG-03 | 截图 | 无窗口：Ctrl+E 直接保存到截图文件夹（默认 PNG），Ctrl+C 复制到剪贴板，「另存为…」弹保存对话框；成功后屏幕提示；保存文件夹在截图设置里选 | capture |
| UI-DLG-04 | 连拍 | 独立对话框（宽 438）：存储、格式/尺寸、数量、范围分组，底部按钮 24px 高 | capture |
| UI-DLG-05 | 导出片段 | 独立对话框（宽 400），按 §2.3 规格分组 | capture |
| UI-DLG-06 | NES、插件工坊 | 只按 §7.3 共同要求 | nes、workshop |

## 8. 键盘与鼠标

### 8.1 快捷键

默认绑定（完整清单与冲突检查见 [menus.md](menus.md)）：

| 功能 | 默认按键 | 负责方 |
| --- | --- | --- |
| 播放/暂停 | Space | 核心 |
| 后退/前进 5 秒 | ← / → | 核心 |
| 后退/前进 30 秒、60 秒、300 秒 | Ctrl / Shift / Ctrl+Alt + ←/→ | playback |
| 上一个/下一个 | PgUp / PgDn | 核心（C6） |
| 音量 +/− | ↑ / ↓ | 核心 |
| 静音 | M | 核心 |
| 倍速 正常/减/加 0.1 | Z / X / C | 核心 |
| 跳转到 | G | playback |
| 打开文件 / 链接 / 字幕 | Ctrl+O（F3）/ Ctrl+U / Alt+O | playback、subtitles |
| 关闭 | F4 | 核心 |
| 全屏 | Enter、Alt+Enter、F11；Esc 退出全屏 | 宿主 |
| 播放列表 | F6 | playlist |
| 播放信息 | Tab；简要信息 Shift+Tab | info |
| 截图 保存/复制 | Ctrl+E / Ctrl+C | capture |
| 画面色彩 | Q W E R T Y U | video |
| 均衡器 | Shift+E | audio |
| 字幕 显示/同步 | Alt+H / > < / | subtitles |

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-KEY-01 | 宿主按绑定表（C3）把任意按键（含修饰键、功能键、字母、方向键）路由到命令；同一输入被两个命令声明时报告冲突 | 宿主 + SDK（C3） |
| UI-KEY-02 | 播放列表获得焦点时的按键（Enter、Delete、Ctrl+↑/↓ 等）只作用于列表 | 宿主 + SDK（C3 的焦点范围） |

### 8.2 鼠标

| 编号 | 动作 | 默认 | 负责方 |
| --- | --- | --- | --- |
| UI-MOUSE-01 | 双击视频 | 切换全屏 | 宿主 |
| UI-MOUSE-02 | 滚轮 | 音量 +/− | 核心（C3 绑定表） |
| UI-MOUSE-03 | 单击视频 | 默认不绑定；用户可在「设置 ▸ 鼠标…」绑定 | 宿主（C3） |
| UI-MOUSE-04 | 右键 | 菜单 | 宿主 |
| UI-MOUSE-05 | 窗口模式下拖动视频 | 移动窗口 | 宿主 |

侧键 X1/X2 默认为上一个/下一个。

## 9. 屏幕提示与播放信息

### 9.1 屏幕提示（OSD）

跳转、音量、静音、倍速、暂停、色彩、字幕同步、截图等操作在视频左上角显示一行提示，例如「音量: 80%」「播放速度 : 1.5x」「已暂停」。

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-OSD-01 | 宿主提供 `plugin.osd.show(text)`：左上角，边距 10px；半透明深色圆角底板 `#0b0a10c4`，白字 `#ffffff` 半粗（600），底边一条 2px `accent-fill` 渐变线（左右各缩进一个圆角）；字号为视频区高度的 3%，取整后限制在 12–48px（交互规格 G-10），底板内边距和圆角随字号缩放（字号 18px 时左右 12、上下 5、圆角 8）；显示 2 秒后 300ms 淡出，新提示替换旧提示 | 宿主 + SDK（C5） |
| UI-OSD-02 | 各插件的操作反馈用屏幕提示：playback（跳转、音量、静音、倍速、暂停）、video（色彩）、subtitles（显示、同步）、capture（保存）、playlist（切换、列表结束、失败） | 各插件 |

### 9.2 播放信息插件（info，D5）

| 编号 | 要求 | 负责方 |
| --- | --- | --- |
| UI-INFO-01 | 「播放信息」插件：右键菜单顶层「播放信息 ✓」、Tab 开关，默认关；Shift+Tab 简要信息 | info（C10） |
| UI-INFO-02 | 叠加层在视频左上角、屏幕提示底板下方 4px（左边距 10px），每块与屏幕提示同样的半透明圆角底板（`#0b0a10c4`，圆角随字号缩放），文字 `#f5f3fa`，字号随屏幕提示缩放（屏幕提示 16px 时为 11px，限制在 9–28px），每秒刷新；内容一行一项、中文标签：文件名、时间 / 时长、视频编码与分辨率、帧率、音频编码 / 采样率 / 声道、后端、已显示帧数、丢帧、会话与帧计数 | info |

播放属性（Ctrl+F1）是独立窗口，分视频信息（编码、尺寸、帧率）和音频信息（编码、采样率、位深、声道）。

## 10. 核心能力（宿主 + SDK）

本节是界面所依赖的核心能力清单。原则：核心只提供多个插件共用、或插件做不到的能力；每项只做最小接口；被新机制取代的旧接口删除，不保留兼容层。接口见[核心界面设计](../architecture/core-ui-design.md)与 [API 参考](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html)。

### 10.1 能力清单

| 编号 | 能力 | 要求 | 覆盖的条目 |
| --- | --- | --- | --- |
| C1 | 主窗口外壳 | 视频贴边、`bg-video`；侧栏占视频右侧全高，3px 可拖分隔条（视频 ≥318、侧栏 ≥317，默认 317）；底栏只在视频下方；默认窗口 780×671，最小 320×182；全屏时底栏覆盖, shown only on hover over its band (FS-BAR-1); the pointer hides after 3 seconds idle；窄窗口先压缩视频再隐藏侧栏 | UI-SHELL-01–09 |
| C2 | 主题 | 可导入的 Slint 主题（§2.1 两套配色、§2.2 字号、§2.3 控件尺寸、§2.5 图标）+ 随配色的控件（按钮、滑块、输入框、复选框、下拉框、滚动条、悬停提示）；默认字体为系统 UI 字体 | UI-THEME-01–03、UI-TOOL-02/03、UI-PL-10/11 |
| C3 | 输入绑定表 | 命令声明 `bindings`。可绑定输入：全部按键 × 修饰键（Ctrl/Shift/Alt/Cmd）；视频区鼠标的左键单击/双击、左半区/右半区双击、右键双击、中键、侧键 X1/X2、滚轮上/下/左/右、长按，均可加修饰键。作用范围：全局，或某个面板/窗口获得焦点时。宿主维护插件默认值 + 用户覆盖层（持久化）；同一输入被两个命令声明时报告冲突、不静默覆盖。右键保留给菜单 | UI-KEY-01–02、UI-MOUSE-01–05、§8.1 |
| C4 | 菜单结构 | 命令声明 `path`（任意深度，含顶层直接命令）、`group`（推荐骨架：打开 / 领域 / 画面 / 窗口入口 / 退出）、`order`；同组按 `order` 排序，组间分隔线；`radio` 单选组；快捷键文字由绑定表生成；样式按 §6.3 | UI-MENU-02/06–08、§6.2 |
| C5 | 屏幕提示 | `plugin.osd.show(text)`，规格见 UI-OSD-01 | UI-OSD-01–02、UI-BAR-02/11、UI-PL-07/10 |
| C6 | 播放序列 | 一个插件登记为序列提供者：`plugin.player.provideSequence({ next, previous, canNext, canPrevious })`（同时只能有一个）；任何插件调用 `player.next()` / `player.previous()`，读取 `player.state.can.next/previous`；没有提供者时为不可用 | UI-BAR-05、PgUp/PgDn、侧键 |
| C7 | 侧栏状态共享 | 任何插件可读取和切换侧栏是否显示（`plugin.panels.sidebar`），不需要知道是哪个插件挂的 | UI-BAR-09、UI-PL-02 |
| C8 | 界面状态持久化 | 宿主跨启动保存：窗口尺寸与位置、侧栏显示状态、停靠状态与宽度、用户绑定覆盖层 | UI-PL-03、UI-SHELL-06、C3 |
| C9 | 窗口标题 | 宿主把当前来源名显示在窗口标题 | UI-BAR-02 |
| C10 | 视频叠加层与播放统计 | 插件在视频上显示只读信息面板（跟随视频区，不接收输入）；`player.stats`：视频编码、分辨率、帧率、音频编码 / 采样率 / 声道、后端、已显示帧数、丢帧 | UI-INFO-01–02 |
| C11 | 面板内右键菜单 | 插件在自己的面板 / 窗口里右键时弹出本插件的一组命令，样式与主菜单相同 | UI-PL-18 |

### 10.2 按 §1 决定取舍的核心项

| 编号 | 能力 | 对应决定 | 结论 |
| --- | --- | --- | --- |
| C12 | 控制面板容器（带页签的小窗口，插件挂载页面） | D2 | 不做 |
| C13 | macOS 顶部菜单栏同步显示命令树 | D4 | 不做 |
| C14 | 侧栏面板分离为浮动窗口、再附着回主窗口（默认附着；浮动窗口默认 300×408、最小 300×250；记住状态与位置） | D6 | 做 |

### 10.3 实施记录

C1–C11 与 C14 已于 2026-10-20 实现，见[本轮记录](../process/rounds/2026-10-core-ui.md)。

## 11. 各插件负责的条目

插件改动界面时对照本表。核心能力不够用时，按[社区工作流](../process/community-workflow.md) §4 向 API 维护者提请求；维护者先和作者讨论现有接口或插件内实现能否解决，确有必要才改核心。验收按工作流 §6.2 GUI 测试。

| 插件 | 条目 |
| --- | --- |
| 全部插件 | UI-MENU-01（菜单位置按 §6.2），§8.1 快捷键，UI-OSD-02（屏幕提示） |
| playback | UI-BAR-01–11，UI-MENU-03/05，UI-DLG-01/02 |
| playlist | UI-PL-01/02/04–20，UI-DLG-02 |
| video | UI-TOOL-01/03–06/09 |
| audio | UI-TOOL-01/03–05/07/09 |
| subtitles | UI-TOOL-01/03–05/08/09，UI-MENU-04 |
| capture | UI-TOOL-01/05，UI-DLG-03–05 |
| store、链接类插件 | UI-TOOL-01/05，UI-DLG-02 |
| nes、workshop | UI-TOOL-01/05，UI-DLG-06 |
| info | UI-INFO-01–02 |

## 12. 待定项

（无）本文的取值都已确定。交互方面尚未确定的项见[交互规格](interaction-spec.md)附录。
