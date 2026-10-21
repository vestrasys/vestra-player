# 交互对齐整改：按细粒度审计对齐 PotPlayer

日期：2026-10-21。基线：main `4d50401`。依据：[交互细粒度差异审计](interaction-gap-audit.md)（下文写作「审计」，行号写作「审计 G-01」「审计 F-07」等）。

本文是发给 API 维护者和插件作者的整改依据，体例沿用[功能需求](feature-requirements.md)：先写核心（宿主），再按插件分章。每一条写明现状（代码位置）、目标行为、实现要点、是否需要核心新增 API、跨平台说明、验收条目。实施按[社区工作流](../process/community-workflow.md)进行：本文里标【核心新增】的接口只是请求草案，每条都要按 §4.1 单独提交、走 §4.3 的协商流程。

## 0. 依据、裁决与范围

### 0.1 依据与证据写法

- PotPlayer 的规格一律取自审计各行的「PotPlayer 规格」一列，出处写法与审计相同：`cfg:键名`（`config-defaults.txt`）、`Dialog(n) 控件号`（附英文 ini 行号 `EN:n`）、`S:id`（字符串表）、`M:key`（菜单树）、`A §`、`B §`、`C §`（[研究报告](research/potplayer-260819/ui/README.md)）。可信度沿用审计：**确认**（皮肤或资源直接给出）、**推断**（由默认值立即数或对话框文字推出）。
- Vestra 的现状以 `main 4d50401` 的代码为准，给出文件与行号。行号会随改动漂移，实施前以函数名、属性名为准重新定位。
- PotPlayer 只给出了行为、没有给出参数（阈值、延时、动画时长）的地方，本文写明我们采用的值，并标「**Vestra 取值，待 Q-xx 校准**」。这些值在 Windows 实机观察后再改，不阻塞实施。

### 0.2 用户裁决（2026-10-21）

1. **以审计里的 PotPlayer 规格为准。** 以前的设计（[界面整改](ui-alignment.md)的 D 决定、`UI-*` 条目，以及功能需求里与之配套的默认值）同样来自粗粒度逆向，没有经过用户逐条审阅。凡是与审计冲突的，一律按审计的 PotPlayer 规格改。
2. **审计标为「刻意偏离」的 4 条逐条重新判断**（§0.3）：只是旧设计的选择、不是平台或架构限制的，改为对齐 PotPlayer；确属平台限制的，保留偏离并写明原因。
3. **资料无法确定的项不改**，列入附录 A，等 Windows 实机观察。

**因此作废或改写的旧决定**：

| 旧条目 | 旧决定 | 新决定 | 本文条目 |
| --- | --- | --- | --- |
| UA §0「macOS 适配原则」 | 保留系统标题栏，不仿制自绘标题栏 | 自绘 31px 标题栏；macOS 只保留系统的红黄绿按钮 | IA-CORE-07 |
| UA D4 | 删除 Commands 按钮，菜单只靠右键 | 标题栏左端恢复主菜单按钮（`CMD_MAIN_MENU`） | IA-CORE-07 |
| UA D2、C12、§7.4 标题「D2 选 (a)」、UI-BAR-09 后半句 | 不做控制面板容器，三个工具窗口各自独立 | 宿主提供 F7 控制面板（音频 / 视频 / 字幕 / 播放 四个页签），插件把小页面挂进去；原工具窗口保留为「完整设置」 | IA-CORE-09、IA-PB-06、IA-PB-08、IA-VID-01、IA-AUD-01、IA-SUB-01、IA-MK-01 |
| UA UI-BAR-10 前半句 | 信息徽标暂不做 | 底栏按窗口宽度显示 HW/SW、HDR、FOURCC、音频编码、声道、下载速率 | IA-PB-05 |
| UA UI-PL-20；功能需求 A13 的默认值「仅打开该文件」与差异说明 | 打开新文件不改变列表 | 打开本地文件时先清空列表，再按「相似文件」加入（`AppendFilePL=0`、`OpenWithSameName=1`） | IA-PL-01 |
| 功能需求 A15 行为第 1 条 | 拖到视频区：追加到列表末尾并播放 | 先清空列表再加入并播放 | G-11 |
| 功能需求 A15 行为第 2 条、验收第 4 条 | 拖到列表：插到落点所在行之前 | 追加到末尾；「插入到拖放位置」为选项，默认关 | G-12 |
| 功能需求 B30「默认关」 | 窗口模式播放时不隐藏指针 | 默认开 | G-06 |
| 功能需求 H04 第 5 条 | 开始播放时自动调整窗口不在本轮 | 默认按 1.0 倍视频尺寸调整窗口 | G-02 |
| 功能需求 H05 第 6 条与默认值 | 不提供「全屏时置顶」，默认不置顶 | 四种模式，默认「全屏播放时置顶」 | IA-WIN-03 |
| 功能需求 H07 与「3a 形态：迷你尺寸」的置顶规则；menus.md「屏幕 ▶ 画中画（macOS）／迷你尺寸（其它系统）」 | `` ` `` 在 macOS 上是系统画中画；迷你尺寸强制置顶 | `` ` `` 在所有系统上都是迷你尺寸，不强制置顶；画中画改为独立命令，无默认按键 | IA-WIN-01、IA-WIN-02 |
| 功能需求 I06「默认开」 | 进度条悬停时间默认开 | 默认关（`PreviewSeekTime=0`） | IA-PB-01 |
| UA UI-SHELL-04 最小高度 182 | 视频区至少 120 高 | 取消视频区高度下限，窗口最小高度只由标题栏与底栏决定 | IA-CORE-13 |
| 参考文档 §5.5 `volume-up` 夹在 0–200；底栏音量滑块 0–200 | 界面与按键都能到 200% | 界面与按键上限 100（`AudioVolumeMaxVal=100`） | G-13a、G-13b |
| 参考文档 §6.3 背景层「`loading`、`error` 保持上一状态」 | 加载中视频区保持原样 | 加载中背景层生效，显示加载动画 | G-16a、G-16b |
| 参考文档 §6.3 `reason: "fullscreen"` | 全屏时侧栏不可用 | 改为「全屏覆盖层当前收起」 | G-01 |
| 参考文档 §6.5「粗体 16px」 | OSD 固定字号 | 按视频区高度自动 | G-10 |

没有列在表里的旧决定继续有效，例如 D1（配色，属于外观而非交互，且有外观模仿风险）、D3（截图）、D5（播放信息插件）、D6（播放列表可分离）、UI-SHELL-08（窄窗口先压缩视频）、UI-SHELL-09 的 3000ms（审计 F-03 无法确定，不改）。

### 0.3 四条「刻意偏离」的重新判断

| 审计行 | 旧偏离 | 判断 | 结论 |
| --- | --- | --- | --- |
| W-06 标题栏 | 用 macOS 系统标题栏（UA §0） | 是设计选择。两个系统都能做无边框窗口加自绘标题栏（winit `with_decorations(false)`、macOS 透明标题栏加全尺寸内容视图）。只有一部分受平台限制：macOS 的红黄绿按钮是系统控件，位置和行为（全屏、平铺菜单、缩放）由系统负责，仿制会失去这些系统行为 | **改为对齐**（IA-CORE-07）。保留的部分：macOS 上不画 PotPlayer 右侧的最小化 / 最大化 / 关闭三个按钮，由系统的红黄绿按钮代替；PotPlayer 的「全屏」按钮仍画。Windows 上完整照 PotPlayer |
| P-01 `` ` `` 键 | macOS 上是系统画中画，替代迷你尺寸（H07） | 是设计选择。迷你尺寸在 macOS 上完全可以做（Windows 上已经在用） | **改为对齐**（IA-WIN-01）。画中画保留为 Vestra 额外的命令，不占 `` ` `` |
| B-05 的一部分 | 不做控制面板（D2）；徽标暂不做（UI-BAR-10） | 都是设计选择。控制面板可以照「设置页」的方式由宿主提供容器（`plugin.settings` 已经证明这种形态可行），不违反插件独立 | **改为对齐**（IA-CORE-09、IA-PB-06）。**仍不做**：360°、3D、浏览器（聊天区）三个按钮。理由不是平台限制，而是 Vestra 没有这三项功能，按钮没有命令可接；PotPlayer 这些按钮带 `AutoVisible`（推断：命令不可用时不显示），所以没有功能时不显示按钮，与 PotPlayer 在功能不可用时的外观一致。若以后要做这三项功能，属于功能需求，不在本文范围 |
| B-06 信息徽标 | 暂不做（UI-BAR-10） | 是设计选择，数据都能从 `player.stats()` 拿到 | **改为对齐**（IA-PB-05）。下载速率需要统计里补一个字段（IA-CORE-16） |

审计 F-13 / P-07（置顶的「全屏播放时」）、W-10 / S-01（最小高度）不在「刻意偏离」之列，但旧设计给过理由（macOS 全屏是独立空间、视频区至少 120 高），同样按上面的标准重新判断：都不是平台限制，改为对齐（IA-WIN-03、IA-CORE-13）。

### 0.4 范围

**在范围内**：审计第 1–10 节判定为「差异」「缺失」的格子，以及上面四条偏离。

**不在范围内**：

- 判定为「一致」的格子。
- 判定为「无法确定」的格子：一律不改，列入附录 A。审计 G-09（纯音频皮肤）同样等实机观察（Q-12）。
- 只有 Vestra 有、PotPlayer 没有对应的项（审计 P-04、P-06、MN-12、L-23）。例外：P-05（画中画时双击全屏）是 Vestra 自身的缺陷，收在 IA-CORE-12。
- 外观（配色、圆角、图标字形）。
- 新功能（360°、3D、直播、聊天）。

### 0.5 与其它文档的关系

- **功能需求**（[feature-requirements.md](feature-requirements.md)）：管「有什么功能」；本文管「交互细节」。两者冲突时以本文为准。§0.2 表里点名的条目，由交互设计负责人在对应整改落地后改写原文，并在原文加一行「已被 interaction-alignment.md 某条取代」。
- **界面整改**（[ui-alignment.md](ui-alignment.md)）：§0.2 表里点名的条目作废，其余仍有效。
- **菜单**（[menus.md](menus.md)）：本文新增或移动的菜单项（`` ` ``、F7、置顶模式、画中画、自动隐藏控制栏、播放列表设置等）在实施后同步更新 menus.md 与冲突检查。
- **插件 API 参考**（[reference.md](../plugins/reference.md)）：各章末的「本章核心 API 请求」是请求草案；维护者裁决后由维护者改参考文档。语义变化（不改签名、只改行为）同样要写进参考文档的变更说明。
- **社区工作流**：每条【核心新增】都先写出「不改 API 的办法」及其代价（§4.3 第 2 步），维护者据此裁决。

### 0.6 编号、优先级与写法

- **编号**：审计里已有的 G-xx 沿用；跨核心与插件的拆成 `G-xxa`（核心）、`G-xxb`（插件）。其余按章新编 `IA-<章>-nn`：`CORE`（核心）、`PB`（playback）、`PL`（playlist）、`WIN`（window）、`INFO`、`VIS`（visualizer）、`HK`（hotkeys）、`VID`（video）、`AUD`（audio）、`SUB`（subtitles）、`MK`（marks）。每条注明对应的审计行。
- **优先级**：**P1** 影响日常使用（审计第 1 节的高优先级差异）；**P2** 状态不一致或常用选项；**P3** 补齐与低频项。轮次划分见附录 C。
- **跨平台**：按工作流 §1.5，系统相关代码只放在 `src/host/src/platform/` 的平台模块里，插件 API 不出现系统专有概念。每条都写明 macOS 与 Windows 的做法；Linux 走 `platform/other.rs`，做不到的返回统一的 `unsupported`。

---


### 0.x 用户裁决（2026-10-21，第二次）

- **IA-PL-01 照做**：打开文件时先清空列表，再自动加入同文件夹里的同类文件。用户的使用习惯就是这样，这也是他保留 PotPlayer 的主要原因。PotPlayer 判断「同类文件」的具体规则另行逆向确认（见 IA-PL-01 的规则小节），以逆向结果为准。
- **IA-CORE-07 改写**：做自绘标题栏，但**不加主菜单按钮**，入口仍是右键菜单。
- **IA-CORE-09 系列（F7 控制面板）取消**：保留现在画面、声音、字幕三个独立工具窗口（原 D2），各插件章节里依赖控制面板的条目一并取消。

## 核心（宿主）

**位置**：`src/host/`（`src/host/src/*.rs`、`src/host/ui/host.slint`、`src/host/ui/menu-view.slint`、平台模块 `src/host/src/platform/`）

本章是宿主自己的改动。大部分不改插件 API，只改宿主行为；改了行为语义的，写明对参考文档的影响。

### G-01 全屏时附着的播放列表从右边缘弹出（P1）

**审计行**：G-01、F-07、L-05、M-31、S-09，B-22 的一部分。

**现状**：全屏时附着侧栏被强制隐藏：`window_state.rs:284-285` 在 `self.fullscreen` 时给出 `Reason::Fullscreen`。F6、底栏的列表按钮、菜单勾选只翻转偏好，画面上看不出变化，底栏按钮却仍然亮着（`plugins/playback/bar.ts:93` `sidebarOn: slot?.preferred`）。

**PotPlayer 规格**：全屏时附着面板不会消失，鼠标移到屏幕右边缘时以弹出层显示。证据：Dialog(456) 3102 中文说明「全屏时鼠标移到右边会显示窗口」；`cfg:AttachOpenFullScreen=0` 即 Dialog(456)「Pop up the docked window pane even if it's closed in windowed mode」默认不勾（EN:1988，C §2.2），只决定窗口模式下关着的面板在全屏时是否也弹出（推断）。

**目标行为**：

1. 适用条件：全屏，侧栏已挂载、处于附着状态（`detached` 为假）。浮动侧栏不受影响（审计 F-08 无法确定，不改）。
2. 两种状态：**收起**（不显示）和**弹出**（以覆盖层盖在视频右侧，不挤压视频）。进入全屏时为收起。
3. 弹出条件：用户偏好为显示（`preferred`），且指针进入屏幕右边缘 **4 px** 以内（Vestra 取值，待 Q-13 校准；阈值本身见 D-02）。偏好为隐藏时右边缘不触发（对应 `AttachOpenFullScreen=0`）。
4. 弹出层几何：从屏幕顶端到底端占满全高，宽度取用户在窗口模式下拖出的侧栏宽度（`sidebar_user_width`），并且不超过「屏幕宽 − 318」。弹出期间底栏（覆盖在视频底部）的宽度收窄到「屏幕宽 − 弹出层宽」，与窗口模式「底栏只在视频下方」一致（Vestra 取值，待 Q-13 观察底栏是否被盖住）。
5. 动画：向左滑出 150 ms，收起时向右滑回 150 ms（Vestra 取值，待 Q-13 校准；审计用语为「滑出」，但时长没有证据）。
6. 收起条件（任一满足）：
   - 指针离开弹出层（x 小于弹出层左边缘）500 ms 后（Vestra 取值，待 Q-13）；
   - 弹出层没有锁（见第 7 条）且 3000 ms 内没有活动（与底栏共用 `IDLE_MS`）；
   - 用户按 F6 或点底栏的列表按钮把偏好改为隐藏；
   - 退出全屏。
7. 锁：面板通过 `Panel.setInteraction` 报告的 `focused`（例如搜索框有焦点）、`dragging`（拖动排序）、`popupOpen`（列表右键菜单）为真时不收起，与底栏的锁规则相同。这样 playlist 现有的 `setInteraction` 调用（审计 L-23 说它「实际没有作用」）在全屏下开始起作用，插件不用改。
8. 全屏中按 F6：偏好从隐藏变为显示时立即弹出，并保持到指针进入后再离开，或 3000 ms 无活动；从显示变为隐藏时立即收起。
9. 焦点：弹出不抢键盘焦点；用户点击弹出层后焦点进入面板；收起时如果焦点在面板里，交回视频区。
10. 指针：指针在弹出层上方时不隐藏（与底栏相同）。
11. `SlotState` 语义：`effective` 表示「现在看得见」，所以弹出时为 `true`、收起时为 `false`；`reason: "fullscreen"` 的含义改为「全屏覆盖层当前收起」，不再表示「全屏时不可用」。是否处于覆盖形态由插件按 `window.state.fullscreen && !state.detached` 自己判断。

**实现要点**：

- `window_state.rs`：`Policy` 增加侧栏覆盖层状态（收起 / 弹出、弹出原因、指针离开的时刻），新增 `pointer(x, y, now)` 输入；`evaluate` 在全屏时按上面的规则给出 `Reason::Fullscreen` 或 `None`，`Visibility.overlay` 在全屏时对侧栏也置真。锁（`Locks`）对侧栏也生效（现在只看底栏）。补单元测试：边缘触发、偏好为隐藏不触发、离开后延时收起、锁住不收起、F6 立即弹出 / 收起。
- `composition.rs:161-177`：全屏时侧栏不再放在 `HorizontalLayout` 里，而是放进一个绝对定位的覆盖层 `Rectangle`（`x: root.width - w`，`y: 0`，全高），带 `animate x { duration: 150ms; }`；窗口模式的布局不变。
- `host.slint`：视频区 `TouchArea` 已经把指针坐标输出为 `pointer-x / pointer-y`；宿主在每轮 `tick`（`main.rs` 中 `evaluate` 之前）把坐标交给 `Policy::pointer`。弹出层上方要另加一个只用于悬停检测的 `TouchArea`（同 G-04），输出 `sidebar-hover`。
- 底栏宽度：composition 在全屏且侧栏弹出时把底栏的 `width` 设为 `root.width - sidebar-actual`。

**核心 API**：不新增。改的是 `SlotState.reason` 的语义（参考文档 §6.3 要改写），属于行为变化，维护者按 §4.2 第 3 条通知受影响的插件（playback 读 `preferred` 画按钮，见 G-18；playlist 的 `setInteraction` 在全屏下开始生效）。

**跨平台**：纯宿主逻辑，无系统调用。macOS 全屏是独立空间，指针移到屏幕顶端会唤出系统菜单栏，与右边缘不冲突。Windows 全屏为无边框置顶窗口，右边缘在多显示器时可能是两块屏幕的交界，阈值按主窗口所在屏幕的右边缘计算即可（指针坐标本来就是窗口坐标）。

**验收**：

1. 窗口模式开着列表，进入全屏：列表不显示，视频占满屏幕。
2. 指针移到屏幕最右侧：列表从右边滑出，盖住视频右侧，视频不缩小；底栏右端与列表左边缘对齐。
3. 指针移回视频中部：约半秒后列表滑回。
4. 列表弹出时点搜索框输入文字，把指针移开：列表不收起；按 Esc 清空搜索、焦点交回列表后再移开：列表收起。
5. 窗口模式关着列表（F6），进入全屏，指针移到右边缘：不弹出。全屏中按 F6：立即弹出；再按 F6：立即收起。
6. 底栏的列表按钮在第 5 条各步中的亮 / 暗与偏好一致。
7. 列表处于浮动状态时进入全屏：浮动窗口保持原样，右边缘不触发。

### G-04 全屏时指针停在底栏上，底栏不隐藏（P1）

**审计行**：G-04、F-05。

**现状**：只有视频区 `TouchArea` 里的指针移动、按键、滚轮、点击算作活动（`host.slint:97-98`，宿主 `WindowInput("activity")`）。底栏的锁只在拖动时设置（`plugins/playback/bar.ts:96-97` `focused: false`）。所以指针停在底栏上甚至在底栏上来回移动，3 秒后底栏照样消失。

**PotPlayer 规格**：`cfg:PersistOsc=1`，Dialog(434) 3034「Disable auto hide of OSC while mouse cursor is over it」（EN:1746）默认勾选（推断）。注意：C §2.2 把这一项写在 OSC 模式下；审计把它推广到全屏覆盖的控制栏，本文按审计执行。

**目标行为**：

1. 指针在底栏范围内（含进度条悬停提示画到视频区的部分以外的整个底栏矩形）时，底栏视为被锁住，不进入空闲隐藏。
2. 指针在底栏范围内移动同样算活动：离开底栏后重新开始 3000 ms 计时。
3. 指针在底栏上时不隐藏指针（现在已经如此，`host.slint:96` 的隐藏只作用于视频区）。
4. 宿主设置页（IA-CORE-06）提供开关「指针在控制栏上时不自动隐藏」，默认开。

**实现要点**：

- `composition.rs:183-190` 生成底栏时，在 `bottom-keys` 外层加一个只做悬停检测的 `TouchArea`（不处理点击，`pointer-event` 一律 `reject` 不吃事件；Slint 的 `TouchArea` 放在内容之下、靠 `has-hover` 判断即可），输出 `bottom-hover` 属性，`changed mouse-x / mouse-y` 时调用 `root.window-input("activity")`。
- `window_state.rs`：`Locks` 增加 `hovered`，由宿主每轮从 `bottom-hover` 读入（宿主自己的锁，与插件的 `setInteraction` 合并，插件的写入不能清掉它）。`Locks::active` 包含 `hovered`。
- 侧栏覆盖层（G-01）用同一个机制。

**核心 API**：不新增。

**跨平台**：纯宿主逻辑。

**验收**：

1. 全屏播放，指针移到底栏上静止 10 秒：底栏不消失，指针不消失。
2. 指针在底栏上左右移动 10 秒：底栏不消失。
3. 指针从底栏移到视频中部并静止：3 秒后底栏和指针都消失。
4. 关掉宿主设置页的开关后重复第 1 条：3 秒后底栏消失。

### G-03 右键菜单可以超出主窗口，放不下时滚动（P1）

**审计行**：G-03、MN-03、MN-04、MN-14。

**现状**：菜单画在主窗口内部（`host.slint:177-194` 的 `MenuView`）。`menu.rs:115-130` 的 `place()` 和 `levels()` 中子菜单的定位（`menu.rs:161-175`）把每一级都限制在窗口之内，翻转也按窗口边缘判断；没有滚动。窗口最小 320×182（`host.slint:11`），顶层菜单约 25 项 × 24 px，高 600 px 以上，小窗口、迷你尺寸、浮动列表窗口（`composition.rs` 的 `floating_source` 用同一个 `MenuView`）里菜单被截断，下半部分点不到。

**PotPlayer 规格**：Win32 弹出菜单（RT_MENU 101，MFC `CCmdUI`，B §3.5、B U5），是顶层弹出窗口，可以超出主窗口，按屏幕边缘翻转（推断）。Win32 菜单在高于屏幕时显示上下滚动箭头（系统行为）。

**目标行为**：

1. 菜单的每一级按**屏幕可用区域**（主窗口所在显示器的工作区：macOS 不含菜单栏与 Dock，Windows 不含任务栏）定位，不受主窗口大小限制。
2. 顶层在指针处弹出；右侧放不下时向左展开，下方放不下时向上展开（贴住指针），仍放不下时贴住工作区边缘。
3. 子菜单在所属行右侧展开，与父级重叠 2 px（现有 `OVERLAP`）；工作区右侧放不下时翻到父级左侧；下方放不下时整体上移到贴住工作区底边。
4. 某一级高于工作区时，该级高度等于工作区高度，上下两端各显示一条 12 px 的滚动箭头条（▲ / ▼）；指针停在箭头条上时每 50 ms 滚动一行，滚轮按行滚动，键盘 ↑↓ 移出可见区时自动滚动（Vestra 取值，Win32 菜单的滚动速度未提取）。
5. 菜单打开时主窗口失去焦点不再关闭菜单（菜单窗口本身会获得焦点），改为：点击菜单以外的任何地方、切换到其它应用、按 Esc（见 IA-CORE-01）时关闭。
6. 浮动列表窗口与面板菜单（`Panel.showMenu`）用同一套规则。

**实现要点**（分两步，第一步是保底，第二步完成后保底仍保留给无法建窗口的情况）：

- **第一步：窗口内滚动**（不需要平台代码）。`menu.rs` 的 `panel()` 在高度超过可用高度时截断为可用高度，并给出 `scroll: true`、`content-height`；`menu-view.slint` 中每一级改用 `Flickable` 承载行，加上下箭头条与滚轮处理。`MenuLevel` 增加 `scroll`、`viewport-y` 字段。单元测试：25 项菜单在 182 高的窗口里每一项都能通过滚动到达。
- **第二步：菜单覆盖层窗口**。宿主打开菜单时创建一个无边框、透明、置顶、不在任务栏显示的顶层窗口，覆盖指针所在显示器的整个工作区；`MenuView` 画在这个窗口里，坐标换算为「工作区坐标」。点击覆盖层上菜单以外的地方等于点外部，关闭菜单（这次点击被菜单吃掉，与 Win32 菜单相同）。好处是只建一个窗口、不用管多级窗口之间的焦点切换。
  - `menu.rs` 的 `levels()` 把 `window: Point` 参数改为工作区矩形，`place()` 与子菜单翻转按工作区计算。
  - `main.rs` 的 `menu_frame()`（`main.rs:3485` 附近）把指针位置从窗口坐标换算为屏幕坐标：窗口原点加指针坐标，除以缩放。
  - 覆盖层窗口的 Slint 组件由宿主生成，与 `floating_source` 同一类做法；键盘事件仍由 `MenuView` 的 `FocusScope` 处理。
  - `main.rs:1157-1162` 在主窗口失去焦点时关闭菜单的规则改为：应用失去激活（覆盖层与主窗口都不是活动窗口）时关闭。
- 「另一种做法」：每一级一个小窗口（更接近 Win32 的实现）。缺点是多个窗口之间的悬停、焦点和关闭判断更复杂，不推荐。

**核心 API**：不新增。

**跨平台**（平台模块新增 `menu_overlay(attrs)` 一类函数，公共代码不直接调用系统接口）：

- macOS：覆盖层窗口的层级设为弹出菜单层（`NSPopUpMenuWindowLevel`），集合行为加 `fullScreenAuxiliary` 与 `moveToActiveSpace`，使它在全屏空间里也能显示；透明背景用 winit `with_transparent(true)`。
- Windows：winit `with_decorations(false)`、`with_skip_taskbar(true)`、`with_window_level(AlwaysOnTop)`、`with_transparent(true)`；工作区用 `MonitorInfo.rcWork`。
- 工作区查询：两个平台都放进 `platform::screen_for` 的返回值（`ScreenFacts` 增加工作区矩形）。
- 其它系统：只做第一步（窗口内滚动）。

**验收**：

1. 把主窗口拖到 320×182，右键视频：顶层菜单完整显示在窗口外，每一项都能点到。
2. 主窗口靠屏幕右边缘时右键：菜单向左展开；子菜单在屏幕右边缘翻到左侧。
3. 主窗口靠屏幕底边时右键：菜单向上展开，不被 Dock / 任务栏挡住。
4. 在 768 高的屏幕上打开项数超过屏幕高度的子菜单（可用测试插件注册 40 项）：出现上下箭头，停在箭头上会滚动，滚轮可滚动，↓ 键到底会自动滚动。
5. 浮动列表窗口（300×250）里右键：列表菜单完整显示。
6. 全屏（macOS 全屏空间）里右键：菜单正常显示在全屏空间里。
7. 菜单打开时点击菜单以外的视频：菜单关闭，视频不响应这次点击；用 ⌘Tab / Alt+Tab 切走：菜单关闭。

### G-15 子菜单展开延时；右键松开时弹出菜单（P1）

**审计行**：G-15、MN-02、MN-05、M-03。

**现状**：悬停立即展开子菜单（`menu-view.slint:97-98` 的 `has-hover` 直接触发 `hovered`，宿主收到 `menu-hover` 立即改 `menu_path`）；右键按下就弹出菜单（`host.slint:126-130`）。斜着移向子菜单时，经过的相邻项会抢先展开，容易误触。

**PotPlayer 规格**：Win32 弹出菜单：子菜单在悬停一段时间后展开（系统参数 `SPI_GETMENUSHOWDELAY`，Windows 默认 400 ms）；右键松开时才弹出菜单（`WM_CONTEXTMENU` 在松开后到达）；鼠标事件表里另有「右键按住」`RightPush`，说明按下与松开分开判定（C §4.2 的 21 种事件）（推断）。

**目标行为**：

1. **右键松开时弹出**：视频区右键按下只记录位置，松开时在**松开的位置**弹出主菜单。按下后移出视频区再松开：不弹出。
2. **展开延时**：指针停在一个子菜单行上满「展开延时」后才展开；在此之前离开该行则取消。
3. **切换延时**：某个子菜单已展开时，指针移到同级的其它行，等同样的延时后才关闭已展开的子菜单（并展开新行的子菜单，若有）；在此期间指针进入已展开的子菜单则取消切换。这就是 Win32 菜单斜向移动不误触的机制，不需要另做「方向判定」。
4. 点击子菜单行立即展开；键盘 → / Enter 立即展开；键盘 ↑↓ 移到命令行时立即关闭更深的级别（现状不变）。
5. 展开延时：Windows 读系统设置 `SPI_GETMENUSHOWDELAY`（默认 400 ms）；macOS 没有对应的系统设置，取 400 ms（PotPlayer 在 Windows 默认设置下的值）。
6. 面板菜单（`Panel.showMenu`）是单层菜单，不涉及延时。插件自己画的面板何时调用 `showMenu`（按下还是松开）由插件决定，建议也在松开时调用（见 playlist 章说明）。
7. 「右键按住」的默认动作未知（Q-18），本条不做；按住右键时只是不弹出菜单，直到松开。

**实现要点**：

- `host.slint:126-130`：把弹出条件从 `PointerEventKind.down` 改为 `up`，并且只有按下也发生在视频区时才弹出（`TouchArea` 记一个 `right-armed` 私有属性）。
- 延时放在宿主而不是 Slint：`menu-hover` 回调照常送达，宿主把「目标路径」记为待定，并记下时刻；`tick` 中到时再提交 `menu_path` 并重建菜单层级；收到新的 `menu-hover` 时替换待定目标。指针进入已展开的子菜单（`menu-hover` 的 `level` 大于当前展开深度）时清除待定的切换。点击（`menu-invoke` 的分组 token）立即提交。
- 平台模块新增 `menu_show_delay() -> Duration`：Windows 用 `SystemParametersInfoW(SPI_GETMENUSHOWDELAY)`，macOS 与其它系统返回 400 ms。

**核心 API**：不新增。

**跨平台**：见第 5 条与实现要点。macOS 用户习惯系统菜单几乎立即展开，400 ms 会显得慢一些；这是按 PotPlayer 对齐的结果，若要改，属于新的用户裁决。

**验收**：

1. 右键按下不松开：菜单不出现；松开：菜单在松开处出现。
2. 在右键菜单上，指针在「视频 ▶」上停 0.2 秒后移开：子菜单不展开；停 0.5 秒：展开。
3. 「视频 ▶」展开后，指针从「视频」行斜着划过「声音」行进入视频子菜单：子菜单不被切换。
4. 「视频 ▶」展开后，指针停在「声音」行上约 0.4 秒：视频子菜单关闭、声音子菜单展开。
5. 点击「视频 ▶」行：立即展开。
6. Windows 上把系统菜单延时改为 100 ms 后重启播放器：子菜单在约 0.1 秒后展开。

### IA-CORE-01 菜单里按 Esc 逐级关闭（P2）

**审计行**：K-14。

**现状**：`menu-view.slint:146-148` 按 Esc 时调用 `dismissed()`，关闭整个菜单。

**PotPlayer 规格**：系统菜单按 Esc 只关闭最深的一级，到顶层再按才全部关闭（Win32 系统行为，推断）。

**目标行为**：子菜单展开且键盘光标在子菜单里时，Esc 等同 ←（关闭最深一级，光标回到父级的展开行）；只有顶层一级时 Esc 关闭整个菜单。用鼠标展开的子菜单（键盘光标仍在父级）按 Esc 时同样先关最深一级。

**实现要点**：`menu-view.slint` 的 Esc 分支改为：`root.levels.length > 1` 时执行与 `LeftArrow` 相同的逻辑（`cursor-level` 退一级，调用 `back()`），否则 `dismissed()`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：打开「视频 ▶ 比例 ▶」两级子菜单，按 Esc 三次：依次关闭「比例」、「视频」、整个菜单。

### IA-CORE-02 菜单助记符（P3）

**审计行**：K-11。

**现状**：菜单支持 ↑↓←→、Enter、Space、Esc，没有助记符。

**PotPlayer 规格**：系统菜单支持助记符，菜单模板文字带 `&`（例如 MENU 102 的「&Undo」）。中文版 PotPlayer 的助记符写在括号里，如「打开文件(&O)」（推断，按中文 Windows 程序惯例）。

**目标行为**：菜单打开时按一个字母键：跳到当前级里下一个助记符为该字母的项；该字母在当前级里只有一项时直接执行（命令）或展开（子菜单），与 Win32 相同。菜单项文字后面以下划线字母显示助记符（例如「打开文件(O)」中 O 带下划线）。

**实现要点**：宿主在 `menu.rs` 的行数据里加 `mnemonic` 字段，`menu-view.slint` 的键盘处理匹配。

**核心 API**【核心新增】：

```ts
interface CommandOptions {
  // …现有字段
  mnemonic?: string;   // 单个 A–Z 或 0–9，不区分大小写；菜单里显示为标题后的「(X)」
}
```

按 §4.3 先想的不改 API 的办法：宿主自动从标题取首个拉丁字母作助记符。代价：中文标题大多没有拉丁字母，得不到助记符；与 PotPlayer 模板里手工指定的字母也对不上。所以仍然提出请求，但优先级最低，可以只给顶层与二级菜单的宿主命令先做。

**跨平台**：macOS 系统菜单没有助记符，这是 PotPlayer 的 Windows 行为，按对齐要求两个系统都做。

**验收**：给「打开文件…」声明 `mnemonic: "O"`，打开菜单按 O：执行打开文件。

### G-05 在画面上拖动移动窗口（P1）

**审计行**：G-05，M-11 的一部分。

**现状**：视频区左键只用于聚焦、单击判定和双击（`host.slint:105-124`，宿主 `left-press` 回调 `main.rs:1612-1645`），没有拖动窗口。

**PotPlayer 规格**：按住左键拖动画面可以移动窗口：`cfg:MouseMoveOnlyTitle=0`，即 Dialog(366) 3038「Prevent window from being dragged outside of title bar」（EN:779）默认不勾（C §4.2）。

**目标行为**：

1. 窗口模式下，在视频区按下左键并移动超过 4 px（与 `CLICK_SLOP` 相同）时开始拖动窗口，由系统接管移动。
2. 开始拖动时取消待定的单击（`Mouse:LeftClick` 不触发），并且这次按下不再参与双击判定。
3. 全屏、菜单打开、画中画小窗中不拖动；迷你尺寸照常可以拖动。
4. 带修饰键（Alt / Ctrl / Shift）的拖动不移动窗口，留给 IA-CORE-17（画面帧位、字幕移动）。
5. 宿主设置页（IA-CORE-06）提供「仅在标题栏上移动窗口」，默认关；打开后视频区拖动不移动窗口。

**实现要点**：

- `host.slint` 视频区 `TouchArea` 的 `pointer-event` 在左键 `down` 时记下位置（所有情况都记，不只在有 `LeftClick` 绑定时），`moved` 中判断 `self.pressed` 且距离超过 4 px 且无修饰键、`!root.host-fullscreen`、`!root.menu-open` 时调用新回调 `drag-window()`，每次按下最多触发一次。
- 宿主收到后清除 `InputShared.click`，调用 `shell.window().with_winit_window(|w| w.drag_window())`（`slint::winit_030::WinitWindowAccessor`，`main.rs:1504` 已在用）。
- 拖动结束后系统可能不再把松开事件送给 Slint，`TouchArea.pressed` 可能残留：下一次 `pointer-event` 收到 `down` 时以新的按下为准；宿主在拖动开始后忽略这次按下对应的双击。

**核心 API**：不新增。

**跨平台**：winit 的 `drag_window()` 在 macOS（`performWindowDragWithEvent`）与 Windows（`WM_NCLBUTTONDOWN` + `HTCAPTION`）都实现，调用放在平台无关代码里即可；Windows 上这样拖动会触发系统的窗口贴靠（Aero Snap，`cfg:Win7AeroSnap=1`），与 PotPlayer 一致。其它系统 winit 不支持时返回错误，宿主忽略。

**验收**：

1. 窗口模式，在视频上按住左键拖动：窗口跟随移动，松开后停在新位置。
2. 给 `Mouse:LeftClick` 绑定「播放/暂停」后拖动窗口：松开后不暂停。
3. 拖动 2 px 后松开：按单击处理（若有绑定）。
4. 全屏中拖动：窗口不动。
5. Windows 上把窗口拖到屏幕左边缘：系统贴靠为半屏。
6. 打开「仅在标题栏上移动窗口」：视频区拖动不移动窗口。

### G-10 屏幕提示与信息块的字号随视频区缩放（P1）

**审计行**：G-10、O-03，W-08 的字号部分。

**现状**：OSD 固定 16 px 粗体（`host.slint:141-146`，参考文档 §6.5）；信息块固定 11 px（`host.slint:157-169`，位置固定在 y = 36）。4K 全屏下太小，320 宽的窗口里又显得过大。

**PotPlayer 规格**：`cfg:MessageFontSize=0`，对应 Dialog(375) 3040「Auto font size」（推断 0 = 自动，C §5.1）。计算公式未提取（D-08）。另有 `cfg:MessageAutoSize=0`（含义未确认，默认关）。

**目标行为**：

1. OSD 字号 = `clamp(12px, round(视频区高度 × 3%), 48px)`（Vestra 取值，待 D-08 提取公式后校准）。示例：默认窗口（视频区约 609 高）18 px；1080 高的全屏 32 px；视频区 120 高时 12 px。
2. 描边宽度 = `max(1px, 字号 / 16)`；左、上边距保持 10 px（`cfg:MessageLeftMargin/TopMargin=10`）。
3. 信息块字号按同一比例缩放：`clamp(9px, 11px × 字号 / 16, 28px)`，内边距同比例；信息块的 y 改为「10 + OSD 行高 + 10」，不再固定 36。
4. 宿主设置页（IA-CORE-06）提供「自动字号」开关，默认开；关时使用固定字号（可调，默认 16）。

**实现要点**：`host.slint` 增加 `private property <length> osd-size: clamp(12px, round(canvas.height * 0.03 / 1px) * 1px, 48px);`，OSD 与信息块引用它。参考文档 §6.5 的「粗体 16px」改为「粗体，字号随视频区」。

**核心 API**：不新增（行为变化，改参考文档）。

**跨平台**：逻辑像素计算，无差异。Windows 高 DPI 下按逻辑像素算，与 macOS Retina 一致。

**验收**：默认窗口按 ↑：OSD 约 18 px；全屏（1080p 屏幕）按 ↑：约 32 px；拖到 320×182：约 12 px，且不超出视频区；按 Tab 时信息块紧贴 OSD 行下方，字号同比例变化。

### G-13a 音量键的上限改为 100（P1）

**审计行**：G-13（核心部分）、M-18、B-17。

**现状**：核心动作 `volume-up` / `volume-down`（`core_actions.rs:358-367`）把音量夹在 0–200；↑ 和滚轮能把音量加到 200%。

**PotPlayer 规格**：`cfg:AudioVolumeMaxVal=100`（界面上的音量最大为 100，C §2.3）；步进 `cfg:AudioVolumeUnit=5`。另有 `AudioVolumeMax=1000`，推断为放大上限，单位与开关方式未知。

**目标行为**：

1. `volume-up`：当前音量低于 100 时加 5，结果不超过 100；当前音量已经 ≥ 100（别的插件用 `setOutput` 设到了 100 以上）时不再增加，OSD 照常显示「音量: N%」。
2. `volume-down`：减 5，不低于 0（不变）。
3. `player.setOutput` 的取值范围仍为 0–200（不改接口）：为以后「音量放大」选项保留。放大开关的语义要等 `AudioVolumeMax` 查清（附录 B 补充项），本轮不做。

**实现要点**：`core_actions.rs:360` 的 `clamp(0.0, 200.0)` 改为：`if facts.volume >= 100.0 && delta > 0.0 { facts.volume } else { (facts.volume + delta).round().clamp(0.0, 100.0) }`；同步改测试 `core_actions.rs:731-736`（现在断言 198 → 200）。参考文档 §5.5 表格「夹在 0–200」改为「上限 100」。

**核心 API**：不新增（核心动作行为变化）。

**跨平台**：无差异。

**验收**：音量 95% 时按 ↑ 两次：100%、100%；滚轮上推同样停在 100%。

### G-16a 加载中背景层生效（P1）

**审计行**：G-16（核心部分）、I-07。

**现状**：`composition.rs:268-274` 的 `no_video` 在 `loading` 和 `error` 时沿用上一次的值（参考文档 §6.3：「换文件时不闪烁」），所以加载中视频区保持上一状态，没有任何加载提示。

**PotPlayer 规格**：加载时视频区显示加载界面：74×74 转圈动画（12 帧，每帧 50 ms，`CMD_ANIMATION`），下方是加粗的 15 px 提示文字（`Logo.txt:22-38`，`State=CMD_LOADING_LOGO`，A §2.4，**确认**）。

**目标行为**：

1. 来源阶段为 `loading` 时，背景层 `effective` 为 `true`（视频区不再显示上一个来源的最后一帧）。
2. `error` 仍沿用上一次的值（不变）。
3. 背景层画什么由背景层插件决定（G-16b）；没有背景层插件时，加载中视频区为 `Theme.bg-video` 纯色。
4. 立即生效，不加延时（PotPlayer 的界面按状态切换，没有证据表明有延时；本地文件打开很快时会短暂闪一下加载界面，与 PotPlayer 一致，待 Q-13 之外另行观察，见附录 A 新增项 IA-Q-01）。

**实现要点**：`no_video` 的 `match` 增加 `Some("loading") => true`；改单元测试 `composition.rs:532-533`。参考文档 §6.3「`loading`、`error` 保持上一状态」改为「`loading` 为 `true`，`error` 保持上一状态」。

**核心 API**：不新增（行为语义变化，visualizer 插件的判断同步改，见 G-16b）。

**跨平台**：无差异。

**验收**：播放视频 A 时打开网络链接 B：在 B 出画面之前，视频区不再停在 A 的最后一帧，显示加载界面（有 visualizer 时）或纯色（没有时）。

### IA-CORE-03 窗口尺寸记忆区分「普通尺寸」与「最大化」（P2）

**审计行**：W-12、X-03。

**现状**：`main.rs:1300-1320` 的 `remember_window` 每一刻都保存窗口实际尺寸。最大化（macOS 缩放）状态下也会被记下来，下次启动就成了一个很大的普通窗口。

**PotPlayer 规格**：保存 `MainX/Y/Width2/Height2`（普通尺寸）；`cfg:RestoreLastState=0`，Dialog(457) 3131 默认不勾，即启动时不恢复最大化状态。

**目标行为**：

1. 只在窗口既不是全屏也不是最大化时保存窗口位置与尺寸（普通尺寸）。
2. 启动时总是以普通尺寸打开，不恢复最大化（`RestoreLastState=0`）。
3. 从最大化还原时回到最大化之前的普通尺寸（系统行为，不变）。

**实现要点**：`remember_window` 前加 `!window.is_maximized()` 判断（Slint `Window::is_maximized`）。`ui_state::Frame` 不需要加字段。

**核心 API**：不新增。

**跨平台**：Slint 的 `is_maximized` 在 macOS 对应 `isZoomed`、在 Windows 对应 `IsZoomed`，都由 winit 提供。

**验收**：把窗口最大化（macOS 双击标题栏或按住 Option 点绿色按钮）后退出、重启：窗口为最大化之前的尺寸和位置。

### IA-CORE-04 全屏入口：Alt+Enter、全屏+、在其它显示器上全屏（P2 / P3）

**审计行**：F-12。

**现状**：宿主「全屏」绑定 Enter、F11、双击（`main.rs:2108`）；没有 Alt+Enter，没有拉伸全屏和在其它显示器全屏。

**PotPlayer 规格**：Enter、Alt+Enter 进入全屏（M:101_0_20、M:101_0_19_14）；Ctrl+Enter「全屏+」（拉伸全屏，S:4400）；Ctrl+Shift+Enter 在其它显示器上全屏（S:4400）。

**目标行为**：

1. （P2）宿主「全屏」的默认绑定改为 `Enter`、`Alt+Enter`、`F11`、`Mouse:LeftDoubleClick`。F11 保留为 Vestra 额外的绑定（不与 PotPlayer 冲突）。
2. （P3）宿主命令「全屏+」Ctrl+Enter：进入全屏，同时让画面拉伸铺满屏幕（不保持宽高比）；再按一次回到普通全屏或退出全屏的具体语义未知，暂定为「切换拉伸全屏」：拉伸全屏中再按退出全屏（Vestra 取值，待 Q-20 一起观察）。菜单位置在「全屏」之后，group `screen`，order 701。
3. （P3）宿主命令「在其它显示器上全屏」Ctrl+Shift+Enter：把主窗口移到下一个显示器再进入全屏；只有一个显示器时禁用，原因「只有一个显示器」。

**实现要点**：`main.rs:2108` 加绑定。拉伸用 mpv 的 `keepaspect=no`（全屏结束时恢复）。换显示器用 winit 的 `available_monitors()` 与 `set_outer_position`。

**核心 API**：不新增（宿主自己的命令）。

**跨平台**：macOS 上 ⌥↩、⌃↩ 不被系统占用；Ctrl 绑定在 macOS 上 ⌘ 也触发（⌘↩ 不冲突）。macOS 全屏会新建空间，换显示器需要先退出全屏、移动窗口、再进入。

**验收**：Alt+Enter 进入全屏；Ctrl+Enter 画面铺满屏幕、无黑边；双显示器时 Ctrl+Shift+Enter 在另一块屏幕上全屏。

### IA-CORE-05 窗口模式「自动隐藏控制栏」开关（P3）

**审计行**：W-03。

**现状**：窗口模式下底栏从不隐藏，也没有开关（`window_state.rs:257-263` 只在全屏时判定为 `Idle`）。

**PotPlayer 规格**：`cfg:AutoHideSkin=0`（「视频区活动时自动隐藏主窗口皮肤」默认关，C §2.2）；菜单 M:101_0_14_7（CmdID 10369）。

**目标行为**：宿主命令「屏幕 ▶ 窗口模式下自动隐藏控制栏」（勾选，默认关；order 720，group `screen-options`）。打开后窗口模式与全屏相同：底栏改为覆盖在视频底部，空闲 3000 ms 后隐藏，活动时出现，G-04 的悬停锁同样生效。自绘标题栏（IA-CORE-07）是否一起隐藏未知，本条只隐藏底栏。选择由宿主保存在界面状态文件。

**实现要点**：`window_state::Policy` 增加 `auto_hide_windowed`，`evaluate` 中把 `self.fullscreen` 的覆盖与空闲判断改为 `self.fullscreen || self.auto_hide_windowed`；composition 的底栏 `y` 与高度条件同样替换。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：勾选后窗口模式播放，指针静止 3 秒：底栏消失、视频占满窗口；移动指针：底栏覆盖在视频底部出现。

### IA-CORE-06 宿主设置页（P2）

**审计行**：G-01、G-04、G-05、G-10 的选项部分，O-02、O-07。

**现状**：「选项」窗口（`settings_window.rs`）只显示插件用 `plugin.settings.page` 注册的页面，宿主自己没有页面。上面几条宿主行为的选项没有地方放。

**PotPlayer 规格**：这些选项都在首选项里：Dialog(456) 3102（全屏时弹出附着面板）、Dialog(434) 3034（悬停不隐藏）、Dialog(366) 3038（仅标题栏移动窗口）、Dialog(375) 3040（自动字号）、3057（显示屏幕提示，`cfg:MessageVisible=1`）、3288-3292（OSD 水平 / 垂直对齐、边距）。

**目标行为**：选项窗口的「常规」分类最前面加一页宿主自己的「界面」页（order 最小），包含：

| 选项 | 默认 | 对应 |
| --- | --- | --- |
| 仅在标题栏上移动窗口 | 关 | G-05，`MouseMoveOnlyTitle=0` |
| 全屏时即使窗口模式下关着也弹出附着面板 | 关 | G-01，`AttachOpenFullScreen=0` |
| 指针在控制栏上时不自动隐藏 | 开 | G-04，`PersistOsc=1` |
| 显示屏幕提示 | 开 | O-07，`MessageVisible=1` |
| 屏幕提示位置：水平（左 / 中 / 右）、垂直（上 / 中 / 下）、水平边距、垂直边距 | 左、上、10、10 | O-02，`MessageHoriPos/VertPos=0`、`MessageLeftMargin/TopMargin=10` |
| 屏幕提示自动字号 / 固定字号 | 自动；固定时 16 | G-10，`MessageFontSize=0` |

关闭「显示屏幕提示」时 `osd.show` 照常 resolve，只是不画（插件不用知道）。选择保存在界面状态文件。

**实现要点**：宿主页面用宿主自带的 Slint 组件实现，挂在选项窗口里与插件页面同样的位置；不经过插件 API。`settings_window.rs` 的页面集合加一类「宿主页面」。

**核心 API**：不新增（宿主内部）。按 §4.3 考虑过的另一种做法：给 preferences 插件开放一组宿主选项接口，由它画页面。代价是新增接口，而这些选项只属于宿主，所以不采用。

**跨平台**：无差异。

**验收**：选项窗口「常规」第一页是「界面」；逐项切换后对应行为改变，重启后保持。

### IA-CORE-07 自绘标题栏（P3）

**审计行**：W-06（重新判断后改为对齐，§0.3）；顺带 D4 作废。W-07（失去焦点时的颜色）无法确定，不在本条。

**现状**：使用系统标题栏，显示来源名（C9）；没有主菜单按钮、置顶状态、全屏按钮。

**PotPlayer 规格**（`TopCommon.txt`，A §2.2，**确认**）：31 px 高，`IsTitleBar='1'` 可拖动，底色 `#1e1e1e`，最后一行 `#000`。从左到右：留白 1、主菜单按钮 89×30（`CMD_MAIN_MENU`，文字「PotPlayer ∨」，悬停底 `#272727`、按下底 `#181818`）、留白 5、文件类型文字（13 px，`#777777`，窗口宽度小于 300 时隐藏，`VisParentMinWidth='300'`）、留白 8 加 3×10 竖分隔（有文件类型时才显示）、标题文字（占满剩余宽度，13 px，`#f0f0f0`，过长时每 50 ms 滚动 1 px）。从右往左：留白 10、关闭 14×13、留白 10、全屏、留白 10、最大化 / 还原（二选一）、留白 10、最小化、留白 10、置顶状态（4 种图标，`ID_ETC_NEXTONTOP` 循环切换置顶方式）、留白 10、1×31 竖分隔线 `#000`、留白 8。图标四态颜色：常态 `#777777`、悬停强调色、按下强调按下色、禁用 `#3b3b3b`（配色按 Vestra 当前主题，尺寸与交互按 PotPlayer）。

**目标行为**：

1. 主窗口改为无系统标题栏，宿主在内容顶部画 31 px 标题栏；整个标题栏（按钮以外）可拖动移动窗口，双击最大化 / 还原（Windows 系统惯例）或缩放（macOS 系统惯例）。
2. 主菜单按钮：文字「Vestra ∨」，89×30，点击在按钮左下角下方弹出主菜单（与右键菜单同一棵树，G-03 的规则）。
3. 文件类型：当前来源的容器格式简称（如 `MKV`、`MP4`；网络链接显示 `URL`），宽度不足 300 时隐藏。
4. 标题：当前来源名（即现在的窗口标题 C9）；超长时以 1 px / 50 ms 向左滚动，到头后停 1 秒再从头开始（停顿为 Vestra 取值）。
5. 右侧按钮：
   - **Windows**：置顶状态、最小化、最大化 / 还原、全屏、关闭，顺序、尺寸、间距照 PotPlayer。
   - **macOS**：左端保留系统的红黄绿按钮（系统控件，§0.3），主菜单按钮紧接其后（留出系统按钮的宽度）；右侧只画置顶状态与全屏两个按钮，不画最小化 / 最大化 / 关闭。
6. 置顶状态按钮：图标按当前置顶模式显示 4 种之一；点击触发绑定输入 `Title:OnTop`（见本条 API）。没有插件声明该绑定时按钮显示为禁用。
7. 全屏时标题栏随系统全屏处理：macOS 全屏空间里不显示（指针移到顶端时由系统显示菜单栏）；Windows 全屏时隐藏（审计 F-10 无法确定，暂按隐藏处理，待 Q-03 一起观察）。
8. 窗口最小尺寸相应增加 31 px 高（与 IA-CORE-13 一起计算）。
9. 失去焦点时的颜色不变（审计 W-07 无法确定，Q-22）。

**实现要点**：

- `host.slint` 的根布局顶部插入 `TitleBar` 组件（宿主自带，`src/host/ui/`）；标题、文件类型由宿主写属性。
- 拖动与双击：标题栏空白处的 `TouchArea` 按下后调用 `drag_window()`（同 G-05），双击调用平台的「标题栏双击动作」。
- Windows：`with_decorations(false)` 后要自己处理边框缩放（winit `drag_resize_window(direction)`，在窗口四边各留 4 px 的缩放热区）、最小化 / 最大化（`set_minimized`、`set_maximized`），以及 Windows 11 的贴靠布局（悬停最大化按钮时的布局菜单需要 `WM_NCHITTEST` 返回 `HTMAXBUTTON`，只能在平台模块里做；做不到时不提供贴靠布局菜单，其它照常）。
- macOS：不去掉系统标题栏，而是设为透明并让内容延伸到标题栏下（`titlebarAppearsTransparent`、`NSFullSizeContentViewWindowMask`、`titleVisibility = hidden`），系统按钮仍由系统画。只能在 `platform/macos.rs` 里做。
- D4 作废后，「删除 Commands 按钮」不再适用；主菜单按钮由宿主提供，不经过插件。

**核心 API**【核心新增】：

```ts
// 绑定输入（参考文档 §4.1 的「输入」语法增加一项）
"Title:OnTop"          // 点击自绘标题栏上的置顶状态按钮；不能带修饰键

// 主窗口（参考文档 §6.6）
setAlwaysOnTop(onTop: boolean, options?: { mode?: "never" | "always" | "playing" | "fullscreen" }): Promise<void>;
// mode 只用于标题栏图标；省略时按 onTop 显示「总是 / 不」两种图标
```

按 §4.3 先想的不改 API 的办法：

- 标题栏的置顶按钮只显示开 / 关两态（宿主自己知道是否置顶），点击时弹出主菜单的「屏幕 ▶ 置顶」子菜单（宿主知道菜单树）。代价：与 PotPlayer 的「点击循环切换、图标 4 态」不一致。
- 宿主自己接管「置顶方式」。代价：置顶方式的逻辑（播放时 / 全屏播放时）是 window 插件的业务，搬进核心违反「核心不承担插件业务」。

所以提出请求：`Title:OnTop` 只是绑定表里多一个输入名，沿用现有的冲突与改绑规则；`mode` 是可选参数，不影响现有调用。

**跨平台**：见实现要点。平台差异全部在平台模块：Windows 无边框窗口的缩放热区与贴靠；macOS 透明标题栏。其它系统保留系统标题栏，不画自绘标题栏（`unsupported` 不需要对插件暴露，`Title:OnTop` 只是永远不会触发）。

**验收**：

1. Windows：标题栏 31 px，左起主菜单按钮、文件类型、标题；右起关闭、全屏、最大化、最小化、置顶状态，间距 10；四边可拖动缩放；拖标题栏到屏幕顶端最大化。
2. macOS：红黄绿按钮在左端，其后是主菜单按钮；右侧是置顶状态、全屏；标题栏空白处可拖动；双击按系统设置缩放。
3. 点主菜单按钮：主菜单在按钮下方弹出。
4. 打开一个名字很长的文件：标题缓慢向左滚动。
5. 窗口拖到 299 宽以下：文件类型隐藏。
6. 点置顶状态按钮：置顶方式按 window 插件的顺序循环，图标随之变化（IA-WIN-07）。

### IA-CORE-08 绑定输入补齐：鼠标双击、按住、侧键滚轮，以及 Scroll Lock（P3）

**审计行**：M-04、M-05、M-06、M-07、M-10；O-08 用到的 Scroll Lock。

**现状**：绑定语法（参考文档 §4.1）只有 `Mouse:LeftClick`、`LeftDoubleClick[:LeftHalf|:RightHalf]`、`MiddleClick`、`X1`、`X2`、四个滚轮方向；右键保留给菜单。键名里没有 `ScrollLock`。

**PotPlayer 规格**：鼠标配置是 21 种事件 × 8 种修饰键的表（C §4.2，**确认**），包括右键双击（S:3881）、中键单击 / 双击（S:3886/3887）、X1 / X2 单击与双击（S:3882-3885）、5 种按住事件（Left / Right / X1 / X2 / Middle Push，S:5004）；Dialog(366) 3037「按住 X 键时操作滚轮」。杂项信息的开关键是 Scroll Lock（Dialog(375) 3194）。出厂时整张表为 0（内置默认动作），默认动作未知（Q-01），所以**只补能力，不加默认绑定**。

**目标行为**：

1. 新输入名：`Mouse:RightDoubleClick`、`Mouse:MiddleDoubleClick`、`Mouse:X1DoubleClick`、`Mouse:X2DoubleClick`、`Mouse:LeftPush`、`Mouse:RightPush`、`Mouse:MiddlePush`、`Mouse:X1Push`、`Mouse:X2Push`、`Mouse:X1+WheelUp` / `WheelDown`、`Mouse:X2+WheelUp` / `WheelDown`；键名 `ScrollLock`。都可以带修饰键。
2. 「按住」：按下后保持 500 ms 不松开、指针移动不超过 4 px（Vestra 取值，Win32 没有统一的长按阈值，待 Q-01、D-01）。
3. 双击判定沿用 `Mouse:LeftClick` 的规则：只有某个双击输入有生效的绑定时，对应按键的单击才延迟到双击间隔结束后判定；没有绑定时行为不变。
4. 右键：`Mouse:RightDoubleClick` 有绑定时，右键松开后要等双击间隔结束才弹出菜单；第二次按下算双击、不弹菜单。没有绑定时按 G-15 立即在松开时弹出。`Mouse:RightPush` 有绑定时，按住达到阈值即触发，这次松开不弹出菜单。
5. `X1` / `X2` 有对应的「侧键 + 滚轮」绑定时，按住侧键期间的滚轮走这些绑定，侧键松开时不再触发 `X1` / `X2` 的单击。

**实现要点**：`bindings.rs` 的输入解析与匹配；`host.slint` 视频区 `pointer-event` 分别处理各按键的 down / up；`main.rs` 的 `PendingClick` 推广为按按键区分的待定单击与按住计时。hotkeys 插件的鼠标页同步列出（IA-HK-01）。

**核心 API**【核心新增】：绑定语法新增上述输入名（参考文档 §4.1）。不改任何方法签名。按 §4.3：这是宿主独有的能力（视频区的鼠标事件只有宿主收得到），插件没有替代办法。

**跨平台**：Scroll Lock：Windows 键盘有；Mac 键盘一般没有，Apple 扩展键盘的 F14 在旧系统上对应 Scroll Lock，winit 报告为 `F14` 而非 `ScrollLock`，所以 macOS 上 `ScrollLock` 通常不会触发，info 插件的命令仍可从菜单执行（IA-INFO-02）。侧键在 macOS 上由 winit 报告为 `Back` / `Forward`，与 Windows 相同。

**验收**：在快捷键设置里把「中键双击」绑定到「全屏」：中键双击切换全屏，中键单击（如有绑定）在双击间隔后才触发；把「右键按住」绑定到「播放/暂停」：按住右键半秒暂停，松开不弹菜单；没有这些绑定时右键行为与 G-15 相同。

### IA-CORE-09 控制面板（P3）

**审计行**：B-05 的一部分（重新判断后改为对齐，§0.3）；UA D2、C12、§7.4 作废。

**现状**：没有控制面板。画面调节、音频工作台、字幕工作台是三个独立的大工具窗口。

**PotPlayer 规格**（UA §7.1 引 A §2.7、B §4.2、C §6，**确认**）：一个 301×227 的「控制面板」小窗口（F7，`ID_CONTROL_BOX_WINDOW`，默认关），无系统边框；34 px 标题（粗体 11 px，右侧透明度滑块、置顶、关闭）；31 px 页签条（每个页签 74 px，13 px 字，未选 `#777`，选中 / 悬停 `#f0f0f0`，底 `#333`）；4 个页签：音频、视频、字幕、播放；每页 295×159，底 `#333333`。改动立即生效，没有确定 / 取消，每项有自己的「默认」复位。底栏右侧有 23×22 的控制面板开关按钮（`DownFlag`，开着时显示按下态）。

**目标行为**：

1. 宿主提供「控制面板」窗口：尺寸、标题、页签、页面按上面的规格；可拖动标题移动，位置跨启动记住；默认关。
2. 宿主命令「控制面板」（勾选，F7，order 810，group `windows`，在「播放列表」之后）切换显示。
3. 页签固定为 音频 / 视频 / 字幕 / 播放 四个，顺序按 PotPlayer。插件可以往某个页签挂「区块」；同一页签有多个区块时按 `order` 从上到下排列，超出 159 高时页面内垂直滚动。一个页签没有任何区块时页签显示为禁用。
4. 区块的规则与设置页相同：一个 component 加一个 global，回调没有「先绑定再显示」的限制；插件停用时移除。
5. 标题栏右侧的透明度滑块（50 宽，调整整个窗口的不透明度 20%–100%）与置顶开关由宿主实现。
6. 控制面板里不触发快捷键（与工具窗口相同；审计 K-08 无法确定，不改）。

**实现要点**：复用 `settings_window.rs` 的页面装配方式（同一种「插件组件挂进宿主窗口」的机制），窗口外框与页签条由宿主画。

**核心 API**【核心新增】：

```ts
// 需要新能力 ui.controlPanel
type ControlTab = "audio" | "video" | "subtitles" | "playback";
plugin.controlPanel.section(file: string, options: { id: string; tab: ControlTab; order?: number }):
  View & Disposable & { readonly ready: Promise<void> };
plugin.controlPanel.state: { readonly visible: boolean; readonly tab: ControlTab };   // 不需要能力
plugin.controlPanel.onChange(listener: (state) => void): Disposable;                   // 需要 events
plugin.controlPanel.show(tab?: ControlTab): Promise<void>;
plugin.controlPanel.hide(): Promise<void>;
plugin.controlPanel.toggle(): Promise<void>;
```

按 §4.3 先想的不改 API 的办法：

- 每个插件把自己的工具窗口缩成 301×227、各自加页签外观（UA 旧方案 D2(a)）。代价：用户面对 3–4 个各自独立的小窗口，F7 只能打开其中一个，无法与 PotPlayer「一个窗口四个页签、底栏一个开关」对齐。
- 由一个插件（例如 video）提供控制面板、其它插件往里挂。代价：插件之间不能通信，做不到。

控制面板需要跨插件共享同一个窗口，满足 §4.3 第 4 步的第二条（「插件之间不能通信，而这项能力本身就要跨插件共享」），所以提出请求。

**跨平台**：窗口与透明度用 winit 的通用接口；透明度在 macOS 用 `NSWindow.alphaValue`、Windows 用分层窗口 `SetLayeredWindowAttributes`，放平台模块。

**验收**：按 F7：出现 301×227 的控制面板，四个页签；各插件的区块出现在对应页签；拖动视频页的亮度滑块，画面立即变化；按 F7 再次关闭；底栏控制面板按钮的按下态与之同步（IA-PB-06）；重启后位置保持。

### IA-CORE-10 浮动窗口的标题栏元素与吸附（P3）

**审计行**：G-17（核心部分）、L-03、L-04、W-13。

**现状**：浮动列表窗口用系统标题栏，只有关闭（`composition.rs` 的 `floating_source`）；不吸附主窗口，不吸附屏幕边缘；置顶只能用 `setAlwaysOnTop` 让所有窗口一起置顶。主窗口也不吸附屏幕边缘。

**PotPlayer 规格**：浮动列表的标题栏有透明度滑块、附着按钮、置顶钉、关闭按钮（`SubCommon.txt:549-581`，**确认**）。吸附：`cfg:UseSideWindow=1`、`AutoAlignWindow=1`、`UseMagWindow=1`，Dialog(354) 3073「Snap to edge/window」（推断；吸附距离与「跟随主窗口」的语义未知，Q-09）。

**目标行为**：

1. 浮动窗口顶部改为宿主画的标题行（与控制面板标题同规格：34 px），右侧依次为透明度滑块（20%–100%）、附着按钮（等于 `setDetached(false)`）、置顶钉（只让这个浮动窗口置顶）、关闭（等于 `setVisible(false)`）。面板自己页签条上的分离 / 附着按钮保留（L-01 已一致）。
2. 吸附：拖动浮动窗口或主窗口时，窗口边缘距屏幕工作区边缘或另一个播放器窗口的边缘在 10 px 以内时吸附对齐（Vestra 取值，待 Q-09 校准距离）。
3. 「移动主窗口时浮动窗口跟随」语义未知（Q-09），本条不做。

**实现要点**：浮动窗口改为无系统标题栏（Windows）/ 透明标题栏（macOS），标题行由 `floating_source` 生成。吸附在宿主处理窗口移动事件（winit `Moved`）时计算并 `set_outer_position`；系统拖动过程中改位置在 macOS 上可能被系统覆盖，做不到时退为「松手时吸附」。

**核心 API**：不新增（宿主窗口行为）。置顶钉只作用于浮动窗口，不经过 `setAlwaysOnTop`。

**跨平台**：Windows 上系统自带贴靠只对屏幕边缘，窗口间吸附要自己算；macOS 系统不吸附，两者都由宿主计算。窗口级别与透明度放平台模块。

**验收**：分离列表：浮动窗口顶部有透明度、附着、置顶钉、关闭；拖透明度滑块窗口变透明；点置顶钉后点别的应用，浮动窗口仍在最上而主窗口不置顶；把浮动窗口拖近主窗口右边缘：贴齐。

### IA-CORE-11 最小化时所有窗口一起最小化（P3）

**审计行**：R-06。

**现状**：浮动列表和工具窗口不跟随主窗口最小化。

**PotPlayer 规格**：`cfg:MinimizeWindowAll=1`，Dialog(354) 3094 默认开。

**目标行为**：主窗口最小化时，浮动侧栏、控制面板、工具窗口一起隐藏；主窗口还原时一起恢复（只恢复最小化前可见的那些）。

**实现要点**：宿主在主窗口 `Occluded` / 最小化事件时对自己的其它窗口调用 `set_minimized(true)`（或隐藏）。

**核心 API**：不新增。

**跨平台**：Windows 用拥有者窗口关系（owned window）最自然，最小化主窗口时系统自动隐藏被拥有的窗口；macOS 用子窗口（`addChildWindow`）会让它们随主窗口移动，不合适，改为宿主逐个最小化。都在平台模块。

**验收**：分离列表、打开画面调节，最小化主窗口：三个窗口都消失；还原：三个都回来。

### IA-CORE-12 画中画期间双击视频区（P2）

**审计行**：P-05（Vestra 自身的问题）。

**现状**：画中画时在主窗口视频区双击仍然切换主窗口全屏，全屏后视频区只有一行「正在以画中画播放」。

**目标行为**：画中画期间，视频区的双击改为退出画中画（画面回到主窗口），不切换全屏。

**实现要点**：`host.slint` 的 `double-clicked` 在 `root.picture-in-picture` 时调用新回调 `pip-return()`，宿主执行与小窗「返回」按钮相同的路径。

**核心 API**：不新增。**跨平台**：画中画目前只有 macOS（参考文档 §6.6），其它系统不出现此状态。

**验收**：macOS 上进入画中画，在主窗口视频区双击：小窗关闭，画面回到主窗口，不进入全屏。

### IA-CORE-13 窗口最小高度（P3）

**审计行**：W-10、S-01。

**现状**：最小 320×182（`host.slint:11`；UI-SHELL-04：底栏 62 加视频至少 120）。`setVideoArea` 的视频高度下限同为 120（参考文档 §6.6）。

**PotPlayer 规格**：宽 320，高度不设下限（`VideoSkin.xml:54,56`）。

**目标行为**：视频区不设高度下限，窗口最小高度 = 标题栏（IA-CORE-07 落地后 31，之前为 0）+ 底栏 62。`setVideoArea` 的视频高度下限改为 1。视频区高度为 0 时暂停渲染输出（不建零尺寸纹理）。

**实现要点**：`host.slint:11` 与 composition 的最小值；渲染尺寸计算（`main.rs` 的 `sizing.observe`、`render_target.rs`）在高度小于 1 物理像素时跳过渲染。

**核心 API**：不新增（`setVideoArea` 的下限变化写进参考文档）。

**跨平台**：Windows 系统对无边框窗口没有额外的最小高度；macOS 系统标题栏方案下另有系统最小高度，以系统为准。

**验收**：窗口可以拖到只剩底栏（加标题栏）的高度；播放不中断，放大后画面恢复。

### IA-CORE-14 嵌入式隐藏字幕轨（P3）

**审计行**：B-05 的一部分（底栏 CC 按钮的前提）。

**现状**：mpv 默认不为视频流里的 CEA-608/708 隐藏字幕建立字幕轨，所以 `subtitles.state.tracks` 里看不到它们。

**PotPlayer 规格**：底栏 CC 按钮 `ID_CLOSED_CAPTION`，`AutoVisible`（推断：有隐藏字幕时才显示），`DownFlag`（开启时按下态）（A §2.3.2）。

**目标行为**：宿主为每个来源打开 mpv 的 `sub-create-cc-track=yes`，隐藏字幕作为一条普通的内嵌字幕轨出现在 `subtitles.state.tracks` 里（`codec` 为 `eia_608`），不默认选中。

**实现要点**：`backend.rs` 初始化 mpv 选项时加一行。

**核心 API**：不新增（`SubtitleTrack` 已有 `codec` 字段）。**跨平台**：mpv 选项，无差异。

**验收**：打开带 CEA-608 字幕的 TS 文件：字幕菜单多一条隐藏字幕轨，默认不显示。

### IA-CORE-15 主窗口接口扩展：最大化、位置、宽高比锁定、活动状态（P3）

**审计行**：X-02、P-08（最大、自定义、窗口移动）、S-11、R-01、R-02。插件侧见 IA-WIN-05、IA-WIN-06、IA-PB-07。

**现状**：`plugin.window` 只有 `setVideoArea`、`setAlwaysOnTop`、`setCursorAutoHide`、`setPictureInPicture`；`WindowState` 只有 `fullscreen`、`videoArea`、`pictureInPicture`。

**PotPlayer 规格**：「最大化」（不盖住任务栏）与「最大化+」（盖住任务栏）（M:101_0_19_11/12）；窗口尺寸预设里的「最大」「自定义」「窗口 +/−」「窗口移动」（M:101_0_19_*，Ctrl+Alt+小键盘）；「调整窗口尺寸时保持比例」（M:101_0_18_20，CmdID 10341）；失去焦点时暂停（Dialog(354) 3066，默认关）、最小化时暂停（`cfg:PauseOnMin=0`，Dialog(354) 3110）。

**核心 API**【核心新增】：

```ts
interface WindowState {
  // …现有字段
  readonly maximized: boolean;
  readonly active: boolean;      // 主窗口（或本应用的任一窗口）是活动窗口
  readonly minimized: boolean;
  readonly frame: { x: number; y: number; width: number; height: number }; // 屏幕坐标，逻辑点
}
setMaximized(mode: "off" | "maximized" | "cover"): Promise<void>;  // cover = 盖住任务栏 / Dock 的最大化
moveBy(dx: number, dy: number): Promise<{ x: number; y: number }>; // 逻辑点；结果夹在屏幕可见区内
setAspectLock(ratio: number | null): Promise<void>;                 // 用户拖动缩放时保持视频区宽高比；null 关闭
```

按 §4.3 先想的不改 API 的办法：

- 「窗口 +/−」可以用现有 `setVideoArea` 按比例放大缩小实现，不需要新接口（IA-WIN-05 就这样做）。
- 「最大化」「窗口移动」「保持比例」「失去焦点 / 最小化时暂停」都需要宿主的窗口状态或窗口操作，插件没有替代办法；`active`、`minimized` 不提供时，「失去焦点时暂停」做不了。
- 可以只接受其中一部分：建议维护者先接受 `active`、`minimized`（只读，代价最小），其余按需求排期。

**跨平台**：`cover` 在 Windows 为全屏尺寸的普通窗口（不切换全屏模式），在 macOS 为铺满整个屏幕（含菜单栏区域）的普通窗口；`maximized` 在 macOS 对应「缩放」。`frame` 在多显示器时为全局坐标。放平台模块。

**验收**：见各插件条目。

### IA-CORE-16 播放统计补充下载速率（P3）

**审计行**：B-06（底栏下载速率徽标）。

**现状**：`PlaybackStats` 有 `cacheSeconds`，没有下载速率。

**PotPlayer 规格**：底栏下载速率（`CMD_DOWNLOAD_RATE_MESSAGE`，网络流，窗口宽度 ≥ 740 时显示，A §2.3.2）。

**核心 API**【核心新增】：

```ts
interface PlaybackStats {
  // …现有字段
  cacheSpeed: number | null;   // 字节/秒，取自 mpv cache-speed；本地文件为 null
}
```

按 §4.3：插件无法读 mpv 属性，没有替代办法；这是 `stats()` 现有的「原样取 mpv 属性」类别里多一个字段，属于兼容扩展。

**跨平台**：mpv 属性，无差异。

**验收**：播放网络链接时 `stats().cacheSpeed` 为正数；本地文件为 `null`。

### IA-CORE-17 视频区修饰键拖动（P3）

**审计行**：M-11（Alt / Ctrl / Shift 拖动画面帧位、Alt / Ctrl 拖动字幕）。插件侧见 IA-VID-02、IA-SUB-02。

**现状**：视频区拖动没有任何作用（G-05 之后无修饰键拖动用于移动窗口）。

**PotPlayer 规格**：`cfg:MousePanScanControl=0`（Alt / Ctrl / Shift + 拖动调整画面帧位，默认关）；`cfg:MouseCaptionMove=0`（Alt / Ctrl + 拖动移动字幕，默认关）（C §4.2）。

**核心 API**【核心新增】：

```ts
// 需要新能力 input.videoDrag
plugin.input.onVideoDrag(
  modifiers: "alt" | "ctrl" | "shift",
  listener: (e: { phase: "start" | "move" | "end"; dx: number; dy: number; videoArea: { width: number; height: number } }) => unknown,
): Disposable;   // dx/dy 为自 start 以来的位移（逻辑点）；同一修饰键先登记的生效
```

按 §4.3 先想的不改 API 的办法：用绑定表——把「Alt+拖动」做成离散的绑定输入（例如每移动 10 px 触发一次「上移帧位」命令）。代价：命令不带参数，拖动只能按固定步长离散地走，手感和精度都差，且上下左右要四条命令。两个功能默认都关，**建议维护者把这条排在最后**，必要时直接按离散绑定的办法降级。

**跨平台**：无差异。macOS 上 Ctrl+左键在系统里等于右键，所以 macOS 上 Ctrl 拖动收不到，`ctrl` 改用 ⌘（与绑定表「Ctrl 也认 ⌘」一致）。

**验收**：见 IA-VID-02、IA-SUB-02。

### 本章核心 API 请求

不改 API、只改宿主行为或语义的：G-01（`SlotState.reason: "fullscreen"` 语义）、G-04、G-03、G-15、IA-CORE-01、G-05、G-10（参考文档 §6.5）、G-13a（§5.5）、G-16a（§6.3）、IA-CORE-03、IA-CORE-04、IA-CORE-05、IA-CORE-06、IA-CORE-10、IA-CORE-11、IA-CORE-12、IA-CORE-13（§6.6 下限）、IA-CORE-14。

【核心新增】请求草案：

| 签名 | 理由（§4.3 第 4 步） | 先考虑过的替代办法 | 来源 |
| --- | --- | --- | --- |
| `CommandOptions.mnemonic?: string` | 菜单由宿主绘制与处理按键 | 宿主从标题自动取字母（中文标题取不到） | IA-CORE-02 |
| 绑定输入 `Title:OnTop` | 标题栏是宿主的界面 | 点击弹出「置顶」子菜单（与 PotPlayer 不一致） | IA-CORE-07 |
| `window.setAlwaysOnTop(onTop, options?: { mode? })` | 标题栏图标要显示 4 种置顶方式 | 只显示开 / 关两态 | IA-CORE-07 |
| 绑定输入 `Mouse:RightDoubleClick`、`Mouse:MiddleDoubleClick`、`Mouse:X1DoubleClick`、`Mouse:X2DoubleClick`、`Mouse:LeftPush`、`Mouse:RightPush`、`Mouse:MiddlePush`、`Mouse:X1Push`、`Mouse:X2Push`、`Mouse:X1+WheelUp/Down`、`Mouse:X2+WheelUp/Down`、`ScrollLock` | 视频区鼠标事件只有宿主收得到 | 无 | IA-CORE-08 |
| `plugin.controlPanel.section(file, { id, tab, order? })`、`.state`、`.onChange`、`.show(tab?)`、`.hide()`、`.toggle()`；能力 `ui.controlPanel` | 一个窗口要由多个插件共享 | 各插件各自缩小工具窗口（做不到「一个窗口四页签」） | IA-CORE-09 |
| `WindowState.maximized`、`.active`、`.minimized`、`.frame` | 宿主窗口状态 | 无 | IA-CORE-15 |
| `window.setMaximized(mode)` | 窗口操作只有宿主能做 | 无 | IA-CORE-15 |
| `window.moveBy(dx, dy)` | 同上 | 无 | IA-CORE-15 |
| `window.setAspectLock(ratio \| null)` | 用户拖动缩放由系统与宿主处理 | 无 | IA-CORE-15 |
| `PlaybackStats.cacheSpeed` | mpv 属性 | 无 | IA-CORE-16 |
| `plugin.input.onVideoDrag(modifiers, listener)`；能力 `input.videoDrag` | 视频区拖动只有宿主收得到 | 离散绑定（手感差），可降级 | IA-CORE-17 |

---

## 播放控制（`playback`）

**位置**：`src/plugins/playback/`

本章改底栏（`ui/bar.slint`、`bar.ts`）和 playback 的默认值。

### G-07 底栏按钮的悬停提示（P1）

**审计行**：G-07、M-29。

**现状**：底栏的 `BarButton`、`MuteButton`、`SidebarToggle`、时间标签都没有悬停提示（`ui/bar.slint`）。右键后退 / 前进切换到上一个 / 下一个文件，用户无从得知。

**PotPlayer 规格**：每个皮肤按钮都带 `Tooltip`，`cfg:UseTooltip=1`；后退 / 前进按钮的提示写明右键功能（`StringTable.txt:303-304`「右键：播放上一个」，A §2.3.2）。提示延时未知（M-30），沿用 `@vestra/widgets` 的 600 ms。

**目标行为**：每个按钮与可点的标签都有提示，文字带当前生效的快捷键（由插件从 `plugin.bindings` 或命令的默认绑定得到；没有 `bindings.manage` 时用默认绑定）：

| 元素 | 提示文字 |
| --- | --- |
| 播放 / 暂停 | 「播放 (Space)」/「暂停 (Space)」 |
| 停止 | 「停止 (F4)」 |
| 后退 | 「后退 5 秒 (←)\n右键：播放上一个 (PgUp)」 |
| 前进 | 「前进 5 秒 (→)\n右键：播放下一个 (PgDn)」 |
| 打开 | 「打开文件 (Ctrl+O)」 |
| 当前时间 | 「点击切换剩余时间」 |
| 静音 | 「静音 (M)」/「取消静音 (M)」 |
| 音量条 | 「音量: N%」 |
| 播放列表开关 | 「播放列表 (F6)」 |
| 新按钮（IA-PB-05、IA-PB-06） | 各自命令标题加快捷键 |

后退 / 前进的秒数跟随 B06 的第一档设置。

**实现要点**：用 `@vestra/widgets.slint` 的 `ToolTipArea` 包住各按钮；提示文字作为桥接属性由 `bar.ts` 计算（键名从命令的绑定得到）。

**核心 API**：不新增。**跨平台**：快捷键文字在 macOS 上显示 ⌘ 还是 Ctrl 沿用菜单的规则（由宿主的标签生成），插件不自己拼。

**验收**：指针停在每个按钮上 0.6 秒，出现上表的提示；在快捷键设置里把播放 / 暂停改到 K 后，提示变为「播放 (K)」。

**实施（第 2 轮，2026-10-21）**：已实现（`tips.ts`、`bar.ts`、`ui/bar.slint`）。键名取自 `plugin.bindings.list()` 的 `labels`（manifest 增加 `bindings.manage`，只读不写）：命令第一个未被别的命令占用的键盘绑定，`bindings.onChange` 时重读；读不到时用默认键。播放列表开关没有自己的命令，取顶层菜单项「播放列表」的按键，没有插件注册它时提示只写「播放列表」。当前时间的提示随状态变为「点击切换已播放时间」。静音的提示文字用「取消静音」（与上表一致）。

### G-13b 音量滑块按 0–100 映射（P1）

**审计行**：G-13（插件部分）、M-18、B-17。核心部分见 G-13a。

**现状**：滑块范围 0–200（`ui/bar.slint:372-373`），默认 100% 时滑块停在正中间；`controller.ts:181` 把拖动值夹在 0–200。

**PotPlayer 规格**：`cfg:AudioVolumeMaxVal=100`；滑块 70 px 宽，拖到最右是 100%（C §2.3）。

**目标行为**：滑块 `maximum: 100`，拖到最右为 100%；默认 100% 时滑块在最右端。当前音量 > 100（别的插件设的）时滑块停在最右，拖动后按拖动值设置（≤ 100）。

**实现要点**：`bar.slint` 的 `maximum: 100`；`controller.ts:181` 的 `Math.min(200, …)` 改为 `Math.min(100, …)`；`bar.ts` 绑定的 `volume` 显示值取 `Math.min(volume, 100)`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：首次启动（音量 100%）时音量滑块在最右；拖到中间 OSD「音量: 50%」。

### G-14 时间显示的切换要保存，点击区域只限当前时间（P1）

**审计行**：G-14、M-22。

**现状**：「已播放 / 剩余」只保存在内存（`bar.ts:56` `let remaining = false`），重启后恢复为已播放；整块时间文字都能点（`ui/bar.slint:413-436` 的 `TouchArea` 覆盖「当前 / 总长」两段）。

**PotPlayer 规格**：选择会保存：`cfg:SkinCurrentLeftTime`（默认 0 = 已播放，保存函数只写非默认值，C §4.4）；只有当前时间标签可点（`BottomVideo.txt:15/19`，当前时间 `Cursor='HAND'`，总时长没有）。剩余时间的显示形式（是否带 `-`）未知（B-10，Q-08），保持现在的 `-MM:SS / 总长`。

**目标行为**：

1. 点击当前时间在「已播放 / 剩余」之间切换，结果保存在 `settings.json` 的 `remainingTime: boolean`（默认 `false`），重启后保持。
2. 只有当前时间那段文字可点，指针在它上方显示手形；斜杠与总时长不可点。

**实现要点**：`settings.ts` 的 `Settings` 加 `remainingTime`；`bar.ts` 的 `remaining` 改读写设置。`bar.slint` 把 `TouchArea` 只套在第一个 `Text` 上，`mouse-cursor: pointer`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：点当前时间切到剩余时间，重启播放器：仍显示剩余时间；点总时长：没有反应；指针在当前时间上是手形。

**实施（第 2 轮）**：已实现；设置键 `remainingTime`。

### G-18 侧栏看不见时底栏列表按钮的状态（P2）

**审计行**：G-18（插件部分）、B-22。PotPlayer 在窄窗口下的行为未知（S-12、Q-02），本条不改「窄窗口时侧栏怎样处理」，只修 Vestra 内部的状态不一致。

**现状**：窗口宽度不足 638 时侧栏因空间不足隐藏（`window_state.rs:282-283` `Reason::Space`），菜单仍勾选，底栏按钮仍亮着（`bar.ts:93` 只看 `preferred`），但侧栏看不见。全屏时同理（G-01 之前）。

**PotPlayer 规格**：按钮 `DownFlag='1'`，开启时保持按下态（A §2.3.2）。

**目标行为**：按钮有三种外观：

| 状态 | 条件 | 外观 |
| --- | --- | --- |
| 关 | `preferred === false` | 常态色 |
| 开 | `preferred && effective` | 强调色（按下态） |
| 开但看不见 | `preferred && !effective && reason === "space"` | 强调色 50% 不透明度；提示文字加「（窗口太窄，放不下播放列表）」 |

全屏时（G-01 之后）`reason === "fullscreen"` 表示覆盖层收起，按钮按「开」显示，因为指针移到右边缘就能看到。

**实现要点**：`bar.ts` 多传一个 `sidebarHidden: slot?.preferred && !slot.effective && slot.reason === "space"`；`bar.slint` 的 `SidebarToggle` 按它降低不透明度。

**核心 API**：不新增（`SlotState.reason` 已有）。**跨平台**：无差异。

**验收**：开着列表把窗口拖窄到 600：列表消失，底栏按钮变为半亮，悬停提示说明原因；拖宽：列表回来、按钮全亮。

### IA-PB-01 进度条悬停时间默认关（P2）

**审计行**：M-15。

**现状**：`settings.ts:28` `hoverTime: true`（功能需求 I06 默认开）。

**PotPlayer 规格**：`cfg:PreviewSeekTime=0`（悬停时间提示默认关）、`cfg:PreviewThumbnail=0`（缩略图默认关），可在菜单 M:101_0_9_21/22 开启。

**目标行为**：`hoverTime` 默认 `false`。已经保存过设置的用户保持原值（只改默认值）。书签 / 章节附近的「标题  时间」提示不受此开关影响（M-16 已一致）。

**实现要点**：改 `settings.ts` 的 `defaults()`；功能需求 I06「默认开」与持久化一行同步改写。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：删除 playback 的 `settings.json` 后启动，指针停在进度条上：没有时间提示；菜单「播放 ▶ 进度条悬停显示时间」未勾选。

**实施（第 2 轮）**：已实现；功能需求 I06 已同步改写。

### IA-PB-02 窄窗口时右侧按钮的隐藏规则（P3）

**审计行**：B-07。

**现状**：只有列表开关在底栏宽度小于 320 时隐藏（`ui/bar.slint:439-440`）。底栏宽约 330、时间为 `H:MM:SS / H:MM:SS`（约 310 px）时，时间文字与列表开关重叠。

**PotPlayer 规格**：`VisMinLeft=320/350/410`：按钮左边缘的 x 小于 N 时隐藏，窗口变窄时右侧按钮从左到右依次消失（推断，A §2.3.2，属性语义见 D-04）。

**目标行为**：右侧每个按钮有自己的 `VisMinLeft`（列表开关 320、控制面板开关 320、附着面板开关 320、流选择 350、CC 410，见 IA-PB-06），按钮左边缘 x 小于该值时隐藏；另外，按钮左边缘与时间文字（含徽标，IA-PB-05）右边缘之间不足 8 px 时也隐藏，保证不重叠。

**实现要点**：`bar.slint` 中右侧按钮由右往左排，各自的 `visible` 按 `self.x >= N && self.x >= time-row-right + 8px` 计算。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：播放超过 1 小时的视频，把窗口从 800 拖窄到 320：右侧按钮从左到右依次消失，任何宽度下都不与时间文字重叠。

### IA-PB-03 不能跳转时显示禁用的滑块（P3）

**审计行**：I-03、B-14。

**现状**：不能跳转时整个滑块隐藏（`bar.slint` 中 `if PlaybackBridge.can-seek`）；已播放部分用 `icon-disabled`。

**PotPlayer 规格**：禁用时显示禁用帧（滑块 `#7d7000`，A §2.3.1）；`AltImage #444444` 可能用于缓冲或不可跳转（用途未知，A U9），不改。

**目标行为**：不能跳转（空闲、直播）时滑块仍显示在当前位置（空闲时在最左），颜色为强调色的禁用色（Vestra 配色下对应 `Theme.accent-disabled`），不响应指针。

**实现要点**：`bar.slint` 去掉 `if can-seek` 的整体隐藏，改为按 `can-seek` 换颜色并禁用 `TouchArea`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：空闲时底栏进度条最左端有一个暗色滑块，点击没有反应。

### IA-PB-04 进度条上 A-B 与书签标记的形状（P3）

**审计行**：B-15（外观差异，交互一致）。

**现状**：标记画成 1×6 的竖线，A-B 区间用色带加两端 1×8 竖线（`bar.slint`）。

**PotPlayer 规格**：A、B 为 6×10 的白色三角（带黑描边），书签为 6×10 的白色五边形旗标（A §2.3.1，**确认**）。

**目标行为**：A 点、B 点画 6×10 三角（A 尖朝右、B 尖朝左），书签画 6×10 旗标，章节保持竖线（PotPlayer 没有单独的章节标记形状）；颜色用主题的文字色加黑描边。悬停判定范围不变（3 px）。

**实现要点**：`bar.slint` 用 `Path` 画；`kind` 来自 `timeline.marks`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：设置 A-B 循环并添加书签：进度条上出现两个三角与一个旗标。

### IA-PB-05 信息徽标（P3）

**审计行**：B-06（重新判断后改为对齐，§0.3）；UA UI-BAR-10 前半句作废。

**现状**：底栏没有信息徽标。

**PotPlayer 规格**（A §2.3.2，**确认**；显示阈值的语义为推断，D-04）：时间之后依次是

| 徽标 | 命令 | 可点击 | 窗口宽度 ≥ 时显示 |
| --- | --- | --- | --- |
| HW / SW 解码 | `CMD_VIDEO_OUT_HW_SW_MESSAGE` | 是 | 460 |
| HDR | `ID_VIDEO_PS_SMPTE_2084` | （HDR 来源才有） | 700 |
| 视频 FOURCC | `CMD_VIDEO_CODEC_FOURCC_MESSAGE` | 否 | 630 |
| 音频编码 | `CMD_AUDIO_CODEC_TYPE_MESSAGE` | 否 | 670 |
| 输出声道 | `CMD_AUDIO_OUT_NUM_CHANNEL_MESSAGE` | 是 | 690 |
| 下载速率 | `CMD_DOWNLOAD_RATE_MESSAGE` | 否 | 740 |

样式：高 14，左右内边距 5，底 `#333333`（悬停 `#3b3b3b`），11 px 字 `#777777`（悬停强调色），彼此间距 5。

**目标行为**：

1. 按上表在时间文字之后显示，阈值按**主窗口宽度**判断（`VisParentMinWidth` 的父容器是主窗口区域，推断）。
2. 内容：`HW` / `SW`（`stats().video.hwdec` 不为 `"no"` 时为 HW）；`HDR`（`video.transfer` 为 `pq` 或 `hlg` 时才显示）；FOURCC 取 `video.codec` 的大写短名（如 `H264`、`HEVC`）；音频编码取 `audio.codec` 大写短名；声道显示 `audio.channels` 加「ch」（如 `2ch`、`6ch`）；下载速率按 `stats().cacheSpeed`（IA-CORE-16）显示 `1.2 MB/s`，本地文件不显示。
3. 没有对应数据时该徽标不显示。每秒刷新一次（与 info 插件相同的频率），只在有来源时轮询。
4. 可点击的两个：本轮**不接点击动作**（HW / SW 切换要求改渲染选项，那是 video 插件独占的 `render()`；声道菜单属于 audio 插件），只显示悬停态。这是按 §4.3「降低需求」的处理，不提 API 请求；以后如果要接，HW / SW 可以做成核心动作。

**实现要点**：`bar.ts` 定时调用 `player.stats()`（需要 `playback.read`，已有）；`bar.slint` 加一行徽标，宽度阈值用 `root.width` 加上底栏到主窗口宽度的差（底栏宽 = 窗口宽 − 侧栏），由 `bar.ts` 用 `window.state.videoArea` 与侧栏宽度算出窗口宽度传入。

**核心 API**：依赖 IA-CORE-16 的 `PlaybackStats.cacheSpeed`（仅下载速率徽标）；其余不新增。

**跨平台**：Windows 当前为软件渲染，`hwdec` 总是 `"no"`，徽标显示 `SW`，如实反映。

**验收**：窗口 800 宽播放 HEVC 5.1 声道 HDR 片源：依次显示 `HW` `HDR` `HEVC` `EAC3` `6ch`；拖窄到 650：只剩 `HW` `HEVC`；拖到 450：全部消失。

### IA-PB-06 底栏右侧按钮补齐：CC、流选择、控制面板、附着面板（P3）

**审计行**：B-05（重新判断后改为对齐，§0.3）；UA UI-BAR-09 后半句作废。

**现状**：右侧只有播放列表开关（23×22，距右 8 px）。

**PotPlayer 规格**（A §2.3.2，**确认**；`AutoVisible`、`VisMinLeft` 的语义为推断）：右侧从左到右：CC（23×22，`DownFlag`，`AutoVisible`，`VisMinLeft=410`）、流选择（弹出菜单，`AutoVisible`，350）、360°、3D、浏览器（不做，§0.3）、播放列表开关（`DownFlag`，320）、控制面板开关（`DownFlag`，与右侧间距 10，320）、附着面板开 / 关（41×40 贴最右，二选一图标，320）。23×22 按钮之间间距 8。

**目标行为**：

1. **CC**：当前来源的字幕轨里有隐藏字幕轨（`codec === "eia_608"` 或 `"eia_708"`，IA-CORE-14）时显示；点击选中 / 取消该轨（`plugin.subtitles.select(track)` / `select(null)`）；该轨被选中时为按下态。
2. **流选择**：当前来源有 2 条以上音轨或有任何字幕轨时显示；点击在按钮上方弹出单层菜单：音轨列表（单选）、分隔线、字幕轨列表（单选，含「关闭字幕」）。用本插件自己注册的无 `path` 命令加 `Panel.showMenu` 实现：音轨用 `player.selectAudioTrack`，字幕用 `subtitles.select`。
3. **播放列表开关**：现状，外观按 G-18。
4. **控制面板开关**：点击 `controlPanel.toggle()`；`controlPanel.state.visible` 时为按下态（IA-CORE-09）。
5. **附着面板开 / 关**（41×40，贴最右，左侧 1 px 分隔）：Vestra 的附着面板就是侧栏，按钮等同 `sidebar.toggle()`；侧栏附着且可见时显示「关闭」图标，否则「打开」图标。与 23×22 的播放列表开关作用相同（Vestra 侧栏里只有播放列表），两者都保留以对齐布局。
6. 各按钮的窄窗口隐藏按 IA-PB-02。

**实现要点**：manifest 增加 `subtitles.read`、`subtitles.control` 能力；流选择的菜单命令放在一个子作用域，轨道变化时重建（参考文档 §4 的动态菜单写法）。

**核心 API**：依赖 IA-CORE-09（控制面板）与 IA-CORE-14（隐藏字幕轨）。流选择、CC 都用现有 API，不新增。

**跨平台**：无差异。

**验收**：播放双音轨带字幕的 MKV：出现流选择按钮，点击弹出音轨与字幕两组，选第二条音轨后声音切换；播放带 CEA-608 的 TS：出现 CC 按钮，点击显示隐藏字幕、按钮变为按下态；按 F7 打开控制面板，控制面板按钮同步按下；附着面板按钮与 F6 效果相同。

### IA-PB-07 失去焦点时暂停、最小化时暂停（P3）

**审计行**：R-01、R-02（默认值已一致，缺选项）。

**PotPlayer 规格**：失去焦点时暂停（Dialog(354) 3066，默认关）；`cfg:PauseOnMin=0`（Dialog(354) 3110，默认关）。

**目标行为**：playback 的设置页（「播放」分类）加两个复选框，默认都关。打开后：主窗口（及本应用其它窗口）都不是活动窗口时暂停；最小化时暂停。恢复焦点 / 还原时**不**自动继续（PotPlayer 行为未知，取保守做法）。

**实现要点**：监听 `window.onChange` 的 `active`、`minimized`；只在播放中且由本规则暂停时记录，避免误继续。

**核心 API**：依赖 IA-CORE-15 的 `WindowState.active`、`minimized`。

**跨平台**：「活动」在两个系统上都是「本应用的任一窗口是前台窗口」。

**验收**：打开「失去焦点时暂停」，播放中点击别的应用：暂停；切回来：保持暂停。

### IA-PB-08 控制面板「播放」页的跳转与速度区块（P3）

**审计行**：B-05 的一部分（控制面板，IA-CORE-09）。

**PotPlayer 规格**（UA §7.1）：播放页：跳转 −1分 −5秒 +5秒 +1分（51×22）；速度 慢 / 快 / 默认（69×22）；AB 循环（由 marks 提供，IA-MK-01）。

**目标行为**：playback 往「播放」页签挂一个区块（order 100）：第一行四个跳转按钮（−1分、−5秒、+5秒、+1分；秒数跟随 B06 的第 1、3 档，标题随之变化），第二行速度三个按钮（慢 = `rate-down`、快 = `rate-up`、默认 = `rate-normal`）与当前速度文字。按钮执行核心动作（`host.run`），OSD 照常。

**实现要点**：新文件 `ui/control.slint`；`plugin.controlPanel.section("ui/control.slint", { id: "playback", tab: "playback", order: 100 })`。

**核心 API**：依赖 IA-CORE-09。**跨平台**：无差异。

**验收**：控制面板「播放」页点 +5秒：前进 5 秒并有 OSD；点「快」：速度 1.1x。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `PlaybackStats.cacheSpeed`（IA-CORE-16） | IA-PB-05 |
| `plugin.controlPanel.*`（IA-CORE-09） | IA-PB-06、IA-PB-08 |
| `WindowState.active`、`WindowState.minimized`（IA-CORE-15） | IA-PB-07 |

其余条目只用现有 API。

---

## 播放列表（`playlist`）

**位置**：`src/plugins/playlist/`

### G-08 选中项与正在播放项自动滚入视野；拖动时靠边自动滚动（P1）

**审计行**：G-08、K-03、L-19、L-20，M-27 的自动滚动部分。证据为推断（Win32 列表控件的默认行为），审计把它列为高优先级，Q-16 只用来校准细节。

**现状**：`ListView` 不跟随选中项滚动（`ui/sidebar.slint:155-158` 的 `select-step` 只改选中状态）：用 ↑ / ↓ / Home / End 移动后，选中项可能在视野之外；切到下一个文件时列表也不滚动；拖动排序时靠近列表边缘不会自动滚动。

**PotPlayer 规格**：标准列表控件：键盘移动选中项时自动滚动到可见（`LVM_ENSUREVISIBLE` 行为）；切换文件时正在播放的项滚入视野（推断）。

**目标行为**：

1. 键盘移动选中项（↑↓、Home / End、PgUp / PgDn 在列表焦点下、Shift 扩展）后，键盘光标所在行完全可见：在视野上方时滚到它成为第一行，在下方时滚到它成为最后一行（Win32 的最小滚动）。
2. 正在播放的项变化时（自动推进、下一个、双击播放），若它不在视野内，滚到它居中；在视野内则不动。列表有搜索筛选且该项被筛掉时不滚动。
3. 拖动排序时指针进入列表区上 / 下边缘 20 px 内，以每 50 ms 一行的速度向该方向滚动，越靠近边缘越快（最多每 50 ms 三行）（Vestra 取值，待 Q-16）。
4. 用户自己滚动过（滚轮或拖滚动条）之后的 2 秒内，正在播放项的变化不触发滚动，避免打断浏览（Vestra 取值）。

**实现要点**：插件自己的桥接加 `in-out property <float> scroll-to-y` 与一个递增的 `scroll-request`（Slint 里 `changed scroll-request => { list.viewport-y = -root.scroll-to-y * 1px; }`），由 `main.ts` 按行号乘 `ROW_HEIGHT`（`drop.ts` 的 20）算出；视野高度用已有的 `list-area` 回调得到。拖动时的自动滚动在 `sidebar.slint` 里用 `Timer` 实现，不经过插件。

**核心 API**：不新增（插件自己的 Slint 桥接）。**跨平台**：无差异。

**验收**：200 项的列表，选中第 1 项按 ↓ 直到第 40 项：视野始终跟随；按 End：滚到底；播放第 1 项，按 PgDn 到第 60 项：第 60 项滚到中间；拖动第 5 项到列表底边并停住：列表向下滚动。

**实施（第 2 轮）**：已实现（`scroll.ts`、`main.ts`、`ui/sidebar.slint`）。键盘跟随覆盖 ↑ / ↓、Home / End、Shift 扩选、搜索框 ↓ 回到列表、Ctrl+↑ / ↓ / Home / End 移动。列表焦点下的 PgUp / PgDn 本轮**未加**：它们现在是全局「上一个 / 下一个」，列表里改作翻页会改变现有按键含义，留待与 menus.md 一并决定；验收里「按 PgDn 到第 60 项」按全局「下一个」理解，走第 2 条（正在播放项居中）。拖动排序时的靠边滚动在 Slint 里实现，拖动的行改为在拖动开始时记下（列表滚动时行元素可能被复用）。

### G-11 拖放到画面：先清空列表再加入并播放（P1）

**审计行**：G-11、L-18；用户裁决按 PotPlayer（§0.2），功能需求 A15 行为第 1 条作废。

**现状**：拖到视频区时追加到列表末尾，然后播放第一个新项（`main.ts:718-766`，`dropped(paths, { play: true })` 不清空）。原有列表保留。

**PotPlayer 规格**：`cfg:AppendFilePL=0`，即 Dialog(373) 3119「Append files rather than clear playlist」（EN:926）默认不勾：拖入文件时先清空列表再加入并播放（推断，C §2.4）。

**目标行为**：

1. 拖到视频区（没有修饰键）：展开拖入内容（文件夹、列表文件规则不变）；有至少一项可加入时，先清空列表，再按拖入顺序加入，播放第一项。一项都不能加入时列表不变（OSD 同现状）。
2. 清空前不弹「移除全部前确认」（那是给「移除全部」命令的确认，拖放不经过它；PotPlayer 也不确认）。
3. 播放设置「打开或拖入时追加，不清空列表」（IA-PL-02 的设置页，默认关）打开时：回到现状（追加并播放第一个新项）。
4. 按住 Option（Windows 为 Alt）拖入：只追加、不清空、不播放（Vestra 额外的修饰键行为，PotPlayer 没有对应；保留，不与默认行为冲突）。
5. 播放器正在打开另一个来源（`busy`）时：不清空，只追加（同现状的保护规则），OSD「正在打开，已加入列表」。
6. 拖入字幕等 playlist 不认领的文件：不经过 playlist，列表不变（同现状）。

**实现要点**：`dropped` 增加 `clear: boolean`；视频区提供者 `drop: ({ paths, modifiers }) => dropped(paths, { play: !modifiers.alt, clear: !modifiers.alt && !settings.appendOnOpen })`；在 `expandDrop` 完成且有条目之后、`addEntries` 之前调用 `queue.clear()`。

**核心 API**：不新增。**跨平台**：Option / Alt 已由 `DropModifiers.alt` 统一。

**验收**：列表有 5 项，拖 2 个 mkv 到视频区：列表只剩这 2 项，播放第一项；打开「追加」设置后再拖：列表变为 7 项；按住 Option / Alt 拖：追加且不播放。

**实施（第 2 轮）**：已实现。清空时不再显示「列表已清空，当前媒体继续」（紧接着就播放新项）。设置键 `appendOnOpen`；IA-PL-01 暂缓期间它只管拖放，设置页上的文字暂写「拖到画面时追加，不清空列表」，IA-PL-01 落地时改回「打开或拖入时追加，不清空列表」。

### G-12 拖放到列表：默认追加到末尾（P1）

**审计行**：G-12、L-17；功能需求 A15 行为第 2 条、验收第 4 条作废。

**现状**：插入到落点所在行之前（`main.ts:736-742`，`drop.ts` 的 `dropRow`）。

**PotPlayer 规格**：`cfg:DropMousePos=0`，即 Dialog(373) 3118「Insert dragged files at the drop point」（EN:925）默认不勾：拖到列表上的文件追加到末尾（推断，C §2.4）。

**目标行为**：

1. 拖到列表（附着或浮动）上：追加到末尾，不播放，不清空（列表上的拖放与 `AppendFilePL` 无关，推断）。
2. 设置「插入到拖放位置」（IA-PL-02，默认关）打开时：按现状插到落点所在行之前。
3. 列表内部的拖动排序不受影响。

**实现要点**：`list.onDrop` 只在 `settings.dropAtPointer` 为真时传 `at`。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：列表 5 项，拖 2 个文件到第 2 行上：追加为第 6、7 项；打开「插入到拖放位置」后再拖到第 2 行上：插在原第 2 行之前。

**实施（第 2 轮）**：已实现；设置键 `dropAtPointer`；功能需求 A15 已同步改写。

### IA-PL-01 打开本地文件时清空列表并加入相似文件（P1）

**审计行**：G-11 的「打开」部分（审计 G-11 建议「拖入或打开」共用一个选项，并指出与 UI-PL-20 冲突）；用户裁决按 PotPlayer，UA UI-PL-20 与功能需求 A13 的默认值作废。

**现状**：别的插件（底栏「打开文件」、最近播放等）打开本地文件时，模式默认「仅打开该文件」，列表不变、不接管（`store.ts:30` `openMode: "only"`，`follow.ts`）；「加入」模式下是追加去重，不清空。「打开文件夹」（A03）默认也不清空（`store.ts` `clearOnOpenFolder: false`，`main.ts:217`）。

**PotPlayer 规格**：`cfg:AppendFilePL=0`（打开新文件时清空列表，C §2.4）；`cfg:OpenWithSameName=1`：打开文件时把相似文件一并加入，菜单 M:101_0_15_5_0..2 依次为仅打开选定文件 / 同时打开相似文件 / 同时打开全部文件，取值 1 = 相似（推断，C §2.1）。

**目标行为**：

1. 「打开文件时」三选一的默认值改为「加入相似文件」。
2. 别的插件打开一个**本地文件**（触发条件同 A13 第 1 条）时：先清空列表，再按模式加入（相似文件 / 同文件夹全部 / 仅该文件），打开的文件成为当前行（接管）。「仅打开该文件」模式下列表清空后只有这一项并接管它（PotPlayer 的「仅打开选定的文件」同样会把它放进列表，推断）。
3. 设置「打开或拖入时追加，不清空列表」（IA-PL-02，默认关，与 G-11 共用）打开时：回到 A13 现在的「追加去重」规则。
4. 「打开 ▶ 打开文件夹…」（A03）按同一设置：默认先清空再加入并播放第一项；`clearOnOpenFolder` 合并进这个设置后删除。
5. 网络链接、生成流打开时列表不变（PotPlayer 打开 URL 时的列表行为未知，见附录 A 新增项 IA-Q-02）。
6. 列表文件（`.m3u` 等）通过「打开列表文件…」打开时本来就是替换列表，不变。
7. 清空前不确认（同 G-11 第 2 条）。
8. 「记住列表」开着时，清空后的新列表照常保存。

**实现要点**：`follow.ts` 的 `handle()`：`mode() === "only"` 也不再提前返回；在 `addEntries` 之前按设置 `queue.clear()`；`existing`（已在列表中）分支在清空模式下不复用旧行。`store.ts` 默认值与校验（`MODES`）更新，`clearOnOpenFolder` 读入时迁移为新设置的取值（只在用户明确改过时保留）。菜单「播放 ▶ 播放列表 ▶ 打开文件时 ▶」三项文字不变，默认圆点移到「加入相似文件」。

**核心 API**：不新增（A13 已只依赖公开状态）。**跨平台**：无差异。

**验收**：

1. 列表有 3 项，文件夹里有 `A01 A02 A03 B`：用底栏「打开文件」打开 `A02`：列表变为 `A01 A02 A03`，`A02` 为当前行，播完自动播放 `A03`。
2. 模式改为「仅打开该文件」后用底栏打开 `B`：列表只剩 `B`，为当前行。
3. 打开「追加」设置后再打开 `A01`：按 A13 现在的规则追加去重，原有行保留。
4. 打开一个网络链接：列表不变。
5. 「打开文件夹…」选一个文件夹：列表被替换为该文件夹的内容并播放第一项。

#### 同类文件规则

来源：静态逆向 PotPlayer 260819 的 `PotPlayer64.dll`，证据（地址、常量、键名）见 [similar-files.md](research/potplayer-260819/similar-files.md)。本小节**取代**功能需求 A13「行为细节」第 2–4 条中自定的「相似」规则、「全部文件」规则、排序与上限。触发条件、接管、清空列表仍按上面的目标行为。

**规则摘要**：只在打开单个本地文件时生效。在同一文件夹（不递归）里找同组文件：视频只配视频，音频只配音频。「相似」模式下，打开的文件名里必须含空格、`-`、`_` 或 ASCII 数字，否则不加入任何文件。每个候选依次过四步宽松比较（标点无关前缀、开头数字、去掉标点和数字后相同或前缀相同、掩码前缀），任意一步通过就加入。最后连同打开的文件一起按自然顺序排序，打开的文件成为当前行。

**确证与推断的划分**

- 确证（有指令级证据）：三种模式的取值（0 仅该文件 / 1 相似 / 2 全部），默认 1；只对单个文件扩展，以 `-` 开头的不扩展；相似模式的前置条件（空格、`-`、`_`、数字，长度 ≥ 2）；不递归，不跳过隐藏文件，没有数量上限；跳过 `.zip`/`.rar` 和打开的文件本身；视频组与音频组的判定方式和默认扩展名表；四步相似判定的全部常量与分支；音频组比视频组多一条更宽松的规则；最后用 `StrCmpLogicalW` 对完整路径排序，以打开的文件为起始项；「监视正在播放的文件夹」（`AutoAddPL`，默认关）复用同一套规则。
- 推断：`AppendFilePL = 0` 时先清空列表（依据选项文字和默认值）；哪些「打开」入口会触发扩展（网络路径、压缩包内文件可能不触发）；大小为 0 的文件被跳过的条件；小写转换只作用于 A–Z；`StrCmpLogicalW` 在中文系统上的具体顺序。

**实现规格**（`media.ts` 的 `siblings` / `isSimilar`，`follow.ts` 不变）

1. 输入：打开的文件路径 P；模式 `similar | folder`（`only` 不调用本规则）。
2. 拆分：`ext` 取最后一个 `.` 之后的部分并转小写；`stem` 为去掉扩展名的文件名，保留原大小写。
3. 分组：`ext` 属于音频扩展名集合时为音频组；否则属于视频扩展名集合时为视频组；两者都不是时只返回 `[P]`。集合沿用附录 B.5（`VIDEO`、`AUDIO`），按整个扩展名精确匹配。PotPlayer 用子串查找，会有 `.wm` 被当成音频这类意外，不照搬。
4. 相似模式的前置条件：`stem` 至少含一个空格、`-`、`_` 或 `0-9`，并且长度 ≥ 2；不满足时返回 `[P]`。全角数字、中文数字都不算数字。
5. 枚举 P 所在文件夹，不递归，只看文件，不看目录。逐项判断：
   - 名字以 `.` 开头的跳过。这是 Vestra 的偏离：macOS 会生成 `._*` 元数据文件，Windows 上没有这个问题；PotPlayer 不跳过隐藏文件。
   - 跳过 P 本身。
   - 候选扩展名按第 3 条分组。音频组：P 和候选都是音频；视频组：P 和候选都是视频。其它跳过。
   - 「全部」模式：直接收入。「相似」模式：`isSimilar(a, b, audio)` 为真才收入，其中 `a = asciiLower(stem) + ".tmp"`，`b = asciiLower(候选 stem) + ".tmp"`。`asciiLower` 只把 A–Z 转成小写；`.tmp` 必须拼上，因为所有长度阈值都把它算在内。
6. `isSimilar` 逐条按 similar-files.md §5 的 S1–S4 实现，常量不得改动：`T1` = 空格 `_-.(){}[]'"`；数字只认 ASCII `0-9`；`digitDiff` 中码位 ≥ 0x81 的字符一律视为非数字；`loosePrefix` 的停止条件是「两个字符都 < 0x80 且至少一个是数字」；所有除法取整。
7. 结果加上 P 本身，按现有的 `collator`（`Intl.Collator("zh", {numeric: true, sensitivity: "base"})`）比较文件名后排序。这是对 `StrCmpLogicalW` 的近似。
8. 不设上限，与 PotPlayer 一致。删掉 `SIBLING_LIMIT` 的截断：文件夹很大时会慢，但截断会让列表与 PotPlayer 不同。如需保护，只在单个文件夹超过 5000 个文件时（同 `SCAN_LIMIT`）截断，并在 OSD 提示。
9. 打开的文件成为当前行；排在它前面的文件也在列表里。
10. 多个文件同时打开时（例如拖入多个文件）不扩展。这条只影响 A15 拖放：拖入单个文件且模式为「相似」或「全部」时，A15 是否也走本规则，见下面差异第 9 条。

**与现实现（A13，`media.ts` `similarName` / `isSimilar` / `siblings`）的差异**

1. 判定算法完全不同。现实现做法：去掉括号内容，数字串替换为 `#`，归一化后相等，或公共前缀 ≥ 4 且 ≥ 50%。PotPlayer 是上面的四步规则，**不去掉括号内容**（只删去括号字符本身），所以字幕组前缀会参与比较。
2. 现实现没有「名字里须有空格、`-`、`_` 或数字」的前置条件：`movie.mkv` 会带入 `movie2.mkv`，PotPlayer 不会。
3. 字幕组：现实现会把 `[其他组] 进击的巨人 - 03` 当成 `[字幕组] 进击的巨人 - 02` 的相似文件，PotPlayer 不会；反过来，PotPlayer 会带入同组的 `[字幕组] 鬼灭之刃 - 01`，现实现不会。
4. 音频：PotPlayer 对音频组额外放宽（只比前 3 个掩码字符）。`01 - Intro.mp3` 会带入 `02 - Song Two.mp3`、`10 - Outro.mp3`，现实现不会。
5. 视频组也更宽：`abc-01` 会带入 `abd-01`，`Show.S01E01` 会带入 `Shower.Cam`，现实现都不会。
6. 「全部」模式：PotPlayer 只收与打开文件**同组**的文件（打开视频时不收音频）。现实现视频、音频都收。
7. 上限：PotPlayer 没有上限；现实现 1000 个，并以打开的文件为中心截取。
8. 隐藏文件：PotPlayer 不跳过；现实现跳过以 `.` 开头的文件，保留这一点（第 5 条的偏离）。
9. 触发范围：现实现只对「别的插件打开的本地文件」生效；PotPlayer 的拖放入口（0x180465c84）也会扩展。A15「拖到视频区」是否在拖入单个文件时也按本规则扩展，属于 A15 的范围，这里只记录差异，不在本条目内改。
10. 排序：两边都是自然顺序、不区分大小写，现实现的 `collator` 已经满足，不改。

**测试用例**（17 组；「打开」为当前行；列表按第 7 条排序；默认模式为相似）

| # | 打开 | 文件夹内容 | 期望列表（顺序） | 不应加入 | 通过的步骤 |
| --- | --- | --- | --- | --- | --- |
| 1 | `Show.S01E02.1080p.mkv` | `Show.S01E01.1080p.mkv` `Show.S01E02.1080p.mkv` `Show.S01E10.1080p.mkv` `Show.S02E01.1080p.mkv` `Show.S01E03.720p.mp4` `Show.S01E01.1080p.srt` `Other.Movie.2019.mkv` | `Show.S01E01.1080p.mkv` `Show.S01E02.1080p.mkv` `Show.S01E03.720p.mp4` `Show.S01E10.1080p.mkv` `Show.S02E01.1080p.mkv` | `.srt`（非媒体）、`Other.Movie.2019.mkv` | E01、E03：S1；E10、S02E01：S3 相等 |
| 2 | `第02集.mp4` | `第01集.mp4` `第02集.mp4` `第10集.mp4` `第01集.mkv` `花絮.mp4` | `第01集.mkv` `第01集.mp4` `第02集.mp4` `第10集.mp4` | `花絮.mp4` | S3 相等；mkv 与 mp4 同为视频组 |
| 3 | `[字幕组] 进击的巨人 - 02 [1080p].mkv` | `[字幕组] 进击的巨人 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 02 [1080p].mkv` `[字幕组] 进击的巨人 - 13 [1080p].mkv` `[字幕组] 鬼灭之刃 - 01 [1080p].mkv` `[其他组] 进击的巨人 - 03 [720p].mkv` | `[字幕组] 鬼灭之刃 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 02 [1080p].mkv` `[字幕组] 进击的巨人 - 13 [1080p].mkv` | `[其他组] 进击的巨人 - 03 [720p].mkv` | 01：S1；13：S3 相等；鬼灭之刃：S3 前缀 |
| 4 | `A02.mkv` | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` `song.mp3` | `A01.mkv` `A02.mkv` `A03.mkv` | `B.mkv`、`song.mp3` | S3 相等 |
| 5 | `A02.mkv`（**全部**模式） | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` `song.mp3` `notes.txt` | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` | `song.mp3`（不同组）、`notes.txt` | 只看分组 |
| 6 | `movie.mkv` | `movie.mkv` `movie2.mkv` `trailer.mkv` | `movie.mkv` | `movie2.mkv`、`trailer.mkv` | 不满足前置条件 |
| 7 | `ep1.mp4` | `ep1.mp4` `ep2.mp4` `EP3.MP4` `ep4.avi` `ep10.mp4` `ep5.mp3` | `ep1.mp4` `ep2.mp4` `EP3.MP4` `ep4.avi` `ep10.mp4` | `ep5.mp3` | S3 相等（大小写无关） |
| 8 | `Ep 001.mkv` | `Ep 001.mkv` `Ep 002.mkv` `Ep 010.mkv` `Ep 100.mkv` `Ep 1000.mkv` | 原顺序全部 | 无 | 002：S1；其余：S3 相等（数字位数不同） |
| 9 | `01 - Intro.mp3` | `01 - Intro.mp3` `02 - Song Two.mp3` `03 - Another.flac` `10 - Outro.mp3` `Interview.mp3` `cover.jpg` | `01 - Intro.mp3` `02 - Song Two.mp3` `03 - Another.flac` `10 - Outro.mp3` | `Interview.mp3`、`cover.jpg` | S4 音频 |
| 10 | `Vlog_2023-05-01.mp4` | `Vlog_2023-05-01.mp4` `Vlog_2023-05-02.mp4` `Vlog_2024-12-31.mp4` `Holiday_2023-05-01.mp4` | `Vlog_2023-05-01.mp4` `Vlog_2023-05-02.mp4` `Vlog_2024-12-31.mp4` | `Holiday_2023-05-01.mp4` | S1 |
| 11 | `Lecture 1.mp4` | `Lecture 1.mp4` `Lecture 2.mp4` `lecture 3.mp4` `Lecture 12.mp4` `Lab 1.mp4` | `Lecture 1.mp4` `Lecture 2.mp4` `lecture 3.mp4` `Lecture 12.mp4` | `Lab 1.mp4` | S1 |
| 12 | `Friends.S01E01.mkv` | `Friends.S01E01.mkv` `Friends.S01E02.mkv` `Friends.Special.mkv` `Friends.S01E01.ass` `Friends.S01E03.zip` | `Friends.S01E01.mkv` `Friends.S01E02.mkv` `Friends.Special.mkv` | `.ass`、`.zip` | S1（`Special` 也算相似） |
| 13 | `西游记01.rmvb` | `西游记01.rmvb` `西游记02.rmvb` `西游记25.rmvb` `红楼梦01.rmvb` | `西游记01.rmvb` `西游记02.rmvb` `西游记25.rmvb` | `红楼梦01.rmvb` | S3 相等 |
| 14 | `abc-01.mkv` | `abc-01.mkv` `abc-02.mkv` `abd-01.mkv` `xyz-01.mkv` | `abc-01.mkv` `abc-02.mkv` `abd-01.mkv` | `xyz-01.mkv` | S1 |
| 15 | `Show.S01E01.mkv` | `Show.S01E01.mkv` `Show.S01E02.mkv` `Shower.Cam.mkv` `Showtime.S01E01.mkv` | `Show.S01E01.mkv` `Show.S01E02.mkv` `Shower.Cam.mkv` | `Showtime.S01E01.mkv` | E02：S1；Shower.Cam：S4 掩码 |
| 16 | `第０２集.mp4`（全角数字） | `第０１集.mp4` `第０２集.mp4` `第１０集.mp4` | `第０２集.mp4` | 其余两个 | 不满足前置条件 |
| 17 | `ab1.mkv` | `ab1.mkv` `ab2.mkv` `ac1.mkv` `b1.mkv` | `ab1.mkv` `ab2.mkv` | `ac1.mkv`、`b1.mkv` | S3 相等（短名字只能完全相同） |

单元测试还应直接测 `isSimilar`，与上表的「通过的步骤」逐项对应，并覆盖两个边界：长度阈值包含 `.tmp`（第 17 组：`ab1.tmp` 共 7 个字符，7/2 = 3 < 5，不进入 S1，`ac1` 在 S3、S4 也因长度不足而判为不相似）；S4 的掩码前缀中，「数字对非数字」在第 n 位上判为不相似。

### IA-PL-02 播放列表设置页与列表焦点下的 F5（P2）

**审计行**：K-05；同时为 G-11、G-12、IA-PL-01 的选项提供位置。

**现状**：列表获得焦点时按 F5 执行全局的「选项…」；拖放与打开相关的选项没有设置页（只有菜单里的「打开文件时」与几个勾选项）。

**PotPlayer 规格**：列表加速键表 118 中 F5 打开「播放列表设置」（CmdID 10315，B §3.6）；对话框 Dialog(373) 包含 3119（追加而非清空）、3118（插入到拖放位置）、3127（开始播放时转移焦点，`FocusSetPL=0`，已一致）等。

**目标行为**：

1. playlist 用 `plugin.settings.page` 注册「播放列表」页（分类「播放」）：
   - 打开文件时：仅打开该文件 / 加入相似文件（默认）/ 加入同文件夹全部文件（与菜单同步）；
   - 打开或拖入时追加，不清空列表（默认关）；
   - 拖到列表时插入到拖放位置（默认关）；
   - 循环、随机、快速推进到下一个、记住列表、移除全部前确认（与菜单同步）。
2. 列表获得焦点时 F5（`focus: "panel"`）打开该页（`settings.show("playlist")`）；列表没有焦点时 F5 仍是全局「选项…」。
3. 菜单里已有的勾选项保留，与设置页双向同步。

**实现要点**：新文件 `ui/settings.slint`；`store.ts` 的 `Settings` 加 `appendOnOpen`、`dropAtPointer`。menus.md「播放列表获得焦点时」一节加 F5。

**核心 API**：不新增（`plugin.settings` 已有）。**跨平台**：Apple 键盘上 F5 需要 fn（menus.md 已有说明）。

**验收**：列表获得焦点按 F5：选项窗口打开在「播放列表」页；在页面上改「打开文件时」，菜单圆点同步。

**实施（第 2 轮）**：已实现（`ui/settings.slint`，manifest 增加 `ui.settings`，页面 id `playlist`，分类「播放」，order 30）。「打开文件时」「循环」用下拉框，其余为复选框，与菜单双向同步。F5 是不进菜单的列表命令「播放列表设置…」。「打开或拖入时追加」的文字见 G-11 的实施说明。menus.md「播放列表获得焦点时」一节的 F5 由交互设计负责人补。

### IA-PL-03 键盘光标的焦点框（P3）

**审计行**：L-11。

**现状**：选中行有底色；没有焦点框，键盘光标与选中合在一起（Ctrl+点击取消选中后，键盘光标在哪看不出来）。

**PotPlayer 规格**：选中行底色 `#2b2b2b`、文字 `#f0f0f0`；焦点框 `FrameColor`（推断，A §2.5.2）。

**目标行为**：列表获得焦点时，键盘光标所在行画 1 px 虚线框（主题的次要文字色），与是否选中无关；列表失去焦点时不画。

**实现要点**：`sidebar.slint` 的行组件加 `is-cursor` 与 `list-focused` 条件。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：选中第 3 行后 Ctrl+点击取消选中：第 3 行仍有虚线框；按 ↓：框移到第 4 行。

### IA-PL-04 列表行的图标列（P3）

**审计行**：L-13。

**现状**：「序号. 标题」，时长右对齐，没有图标。

**PotPlayer 规格**：`cfg:DisplayIconPL=1`（显示图标）、`DisplayExtPL=1`（显示扩展名）、`DisplayIdxPL=1`、`DisplayTimePL=1`（C §2.4）。

**目标行为**：序号之后、标题之前画 12×12 的类型图标：视频、音频、网络链接三种（字形自绘，不用 PotPlayer 素材）。扩展名：标题来自文件名时保留扩展名（现状）。

**实现要点**：行模型加 `kind`（视频 / 音频由扩展名判断，链接由 `kind === "url"`）。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：列表里混放视频、音频、链接：三种图标不同。

### 本章核心 API 请求

无。本章全部用现有 API。

说明（G-15 的配合建议，不是整改项）：列表的右键菜单现在在按下时调用 `showMenu`；主菜单改为松开时弹出（G-15）后，建议列表也改为松开时弹出，保持一致。

---

## 窗口（`window`）

**位置**：`src/plugins/window/`

### G-02 开始播放时把窗口调整为 1.0 倍视频尺寸（P1）

**审计行**：G-02、S-03；功能需求 H04 第 5 条作废。

**现状**：打开文件不改变窗口尺寸；window 插件只提供手动的倍数（`main.ts`、`size.ts`）。

**PotPlayer 规格**：`cfg:PlayScreenSize=2`，对应下拉框「播放窗口尺寸」Dialog(358) 3286（EN:672）。下拉项的填充顺序（C §2.1）：不使用、0.5x、1.0x、1.5x、2.0x、全屏三种、自定义、适应比例、最大化；索引 2 = 1.0x。`cfg:PlayScreenSizeFirst=0`：Dialog(358) 3095「仅在播放视频时调整一次尺寸」默认不勾，即每个文件都调整（推断）。上下限 Dialog(490)：`cfg:2xMinSizeUse/2xMaxSizeUse=0`（默认不启用）、`ResizeMinWidth=300`、`ResizeMinHeight=200`。基准点：`cfg:MoveSizeByCenter=0`（保持左上角，已一致）；`PlayScreenMoveCenter=0`（不居中，已一致）。

**目标行为**：

1. 设置「开始播放时的窗口尺寸」：不调整 / 0.5 倍 / 1 倍（默认）/ 1.5 倍 / 2 倍 / 适应视频比例。PotPlayer 的「全屏三种」「自定义」「最大化」暂不提供（需要 IA-CORE-15 的接口，见 IA-WIN-05）。
2. 设置「只调整一次」（默认关）：打开时只在本次运行的第一个有视频的来源上调整。
3. 触发：每次新来源（`sourceId` 变化）第一次拿到视频尺寸时调整一次（用现有的 `VideoSize` 探测：`ready` 后读 `stats()`，每 500 ms 重试至多 10 次）。同一来源重播（`restart`）、暂停、跳转不触发。
4. 不调整的情况：全屏；迷你尺寸生效中；画中画中；纯音频或没有视频；来源是插件生成的流且没有尺寸。
5. 尺寸规则与手动倍数（H03）相同：目标是视频区，核心加上底栏与附着侧栏，保持左上角，放不下时按屏幕缩小、太小时按最小尺寸放大。
6. 自动调整不显示 OSD（避免每次打开文件都弹提示；PotPlayer 是否提示未知）；被屏幕缩小时同样不提示。
7. 用户在播放中手动拖动窗口后，这一来源内不再自动调整（下一个来源照常）。

**实现要点**：`main.ts` 订阅 `player.onChange`，`video-size.ts` 已有的探测结果到达时调用 `Sizer` 的倍数逻辑（复用 `scaled`、`fitted`）。设置放在 window 插件新注册的设置页（分类「播放」，标题「窗口」），同时在「屏幕 ▶ 开始播放时 ▶」下给六个单选项（order 716–721，group `screen-auto`）。持久化 `settings.json` 的 `autoSize: "off" | 0.5 | 1 | 1.5 | 2 | "fit"`，默认 `1`；`autoSizeOnce: boolean`，默认 `false`。

**核心 API**：不新增（`setVideoArea`、`player.onChange`、`stats()` 已有）。

**跨平台**：`setVideoArea` 在 Windows 上按整个显示器计算可见区（可能与任务栏重叠，参考文档 §6.6），这是核心的已知差异，不在本条处理。

**验收**：

1. 默认设置下打开 1280×720 片源（屏幕 2560×1440）：视频区变为 1280×720，窗口左上角不动。
2. 接着打开 640×480 片源：视频区变为 640×480。
3. 打开 3840×2160 片源：按屏幕缩小到能放下，无提示。
4. 打开 mp3：窗口不变。
5. 全屏中通过列表切到下一个视频：不调整；退出全屏后窗口为全屏前的尺寸。
6. 设置为「不调整」：打开文件时窗口不变；设置为「只调整一次」：只有第一个视频调整。

**实施（第 2 轮，2026-10-21）**：已实现（`window/settings.ts`、`main.ts`）。实现、菜单、默认值与目标行为相同（设置键 `autoSize`、`autoSizeOnce`）。另外：

- 「只调整一次」在菜单里也有入口：「屏幕 ▶ 开始播放时 ▶ 只调整一次」（勾选，order 722，group `screen-auto-once`）。设置页「选项 ▶ 播放 ▶ 窗口」（order 30）同时放了置顶方式与「播放时隐藏鼠标指针」，与菜单项共用设置。
- 从菜单改设置时提示「开始播放时: 1倍」「开始播放时: 不调整窗口」「开始播放时只调整一次: 开 / 关」；改设置不立即调整当前窗口，下一个来源生效。从设置页修改不提示。
- 「只调整一次」：本次运行（插件本次激活）中第一个有视频的来源用掉这一次，即使它因为全屏等原因没有调整。
- 第 7 条「播放中手动拖动」：只在来源已就绪、尺寸还没拿到的这段时间里有意义（拿到尺寸就立即调整，之后同一来源不再调整）；窗口模式下视频区变化超过 1 点即算拖动，全屏切换、画中画中不算。尺寸拿到前倍数命令是禁用的，所以不存在「用户先按了倍数键」的情况。
- 插件激活时已在播放的来源也按新来源处理（启动时带文件打开的情况）。

### G-06 播放时隐藏鼠标指针默认开（P1）

**审计行**：G-06、W-09；功能需求 B30「默认关」作废。

**现状**：`settings.ts:11` `hideCursorWhilePlaying: false`，窗口模式下指针一直显示；全屏下会隐藏。

**PotPlayer 规格**：`cfg:AutoHideMouse=1`（播放时隐藏指针默认开）；菜单 M:101_0_9_16；Dialog(358) 3089（C §4.1）。隐藏延时未知（W-09），沿用宿主 3000 ms。

**目标行为**：默认开。隐藏条件不变（参考文档 §6.6 `setCursorAutoHide("playing")`：播放中、指针在视频区、3 秒无活动、无菜单与对话框、主窗口是活动窗口）。已经保存过设置的用户保持原值。

**实现要点**：`settings.ts:11` 改默认值；`main.ts:150` 在激活时照常按设置调用（默认开时会调用一次 `setCursorAutoHide("playing")`）。功能需求 B30「默认关」「默认 `false`」同步改写。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：删除 window 的 `settings.json` 后启动，窗口模式播放，指针在视频上静止 3 秒：消失；菜单「播放 ▶ 播放时隐藏鼠标指针」已勾选。

**实施（第 2 轮，2026-10-21）**：已实现；设置键 `hideCursorWhilePlaying` 默认 `true`。已保存过 `settings.json` 的用户保持原值（3a 起插件每次改任何一项都会写入整个文件，所以改过置顶方式的老用户文件里已有 `false`，不会变成开）。功能需求 B30 已同步改写。

### IA-WIN-01 `` ` `` 在所有系统上都是「迷你尺寸」，画中画改为独立命令（P2）

**审计行**：P-01（重新判断后改为对齐，§0.3）；功能需求 H07 的「替代」关系与 menus.md 对应行作废。

**现状**：命令 `mini`（`` ` ``，order 710）在 `pictureInPicture.supported` 为真时（macOS 14+）是「画中画」，否则是「迷你尺寸」（`main.ts:39-74`）。

**PotPlayer 规格**：`` ` `` 是「迷你尺寸 Minimum size」，CmdID 10231（M:101_0_19_1），窗口尺寸预设；OSD S:1085「Window Size : Minimum Size」。

**目标行为**：

1. 「屏幕 ▶ 迷你尺寸」（勾选，`` ` ``，order 710，group `screen`）在所有系统上都存在，行为按功能需求 H07「3a 形态：迷你尺寸」：视频区缩到能放进 400×300 的最大同比例尺寸、保持左上角，再按一次恢复。尺寸「400×300」是 Vestra 取值，PotPlayer 迷你尺寸的实际尺寸待 Q-19。置顶规则见 IA-WIN-02。
2. 「屏幕 ▶ 画中画」（勾选，order 709，group `screen`，**无默认按键**）只在 `pictureInPicture.supported` 为真时注册；行为按 H07 现有的画中画规则不变。它是 Vestra 额外的命令，PotPlayer 没有对应。
3. 两项互斥：进入画中画时若迷你尺寸生效，先恢复原尺寸（现有规则）；画中画中执行迷你尺寸时先退出画中画。
4. 命令 id：迷你尺寸保留 `mini`（用户对 `` ` `` 的改绑随之保留）；画中画用新 id `pip`。已把 `` ` `` 改绑到别的键的用户，改绑仍作用于迷你尺寸。

**实现要点**：`main.ts` 拆掉 `declare(supported)` 的二选一，迷你尺寸命令常驻，画中画命令随 `window.onChange` 的 `supported` 注册 / 释放。

**核心 API**：不新增。**跨平台**：迷你尺寸只用 `setVideoArea`，两个系统相同；画中画仍只有 macOS。

**验收**：macOS 上按 `` ` ``：窗口缩为迷你尺寸（不进入画中画）；从「屏幕 ▶ 画中画」进入画中画；Windows 上菜单没有「画中画」，`` ` `` 行为与 macOS 相同。

**实施（第 2 轮，2026-10-21）**：已实现（`main.ts`）。迷你尺寸命令常驻；画中画命令 `pip` 在 `supported` 变为真时注册、变为假时注销。互斥规则按第 3 条：画中画中按 `` ` `` 先退出画中画，退出失败（提示「无法退出画中画：<原因>」）则不进入迷你尺寸。两项的切换进行中互相标为忙碌。功能需求 H07、menus.md 已同步改写。

### IA-WIN-02 迷你尺寸不再强制置顶（P2）

**审计行**：P-02（差异，无证据支持）。

**现状**：迷你尺寸会强制置顶（`on-top.ts:25-27` 的 `this.#mini() ||`）。

**PotPlayer 规格**：迷你尺寸与置顶无关，置顶只有 4 种模式（C §2.1，`OnTopMode`）。

**目标行为**：迷你尺寸只改窗口尺寸；是否置顶完全由置顶方式（IA-WIN-03）决定。迷你尺寸的进入 / 退出不再调用 `setAlwaysOnTop`。

**实现要点**：`on-top.ts` 去掉 `#mini` 依赖；`main.ts:28` 构造 `OnTop` 时不再传 `mini`；`Sizer` 的 `changed` 回调不再触发置顶刷新。功能需求 H07 3a 段「同时置顶」「置顶回到 H05 所选方式」改写。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：置顶方式为「不置顶」时按 `` ` ``：窗口变小，点 Finder / 资源管理器后播放器被盖住。

**实施（第 2 轮，2026-10-21）**：已实现（`on-top.ts` 不再依赖迷你尺寸）。功能需求 H07 3a 段已同步改写。

### IA-WIN-03 置顶方式四种，默认「全屏播放时」（P2）

**审计行**：F-13、P-07；功能需求 H05 第 6 条与默认值作废。

**现状**：三种（不 / 总是 / 播放时），默认「不置顶」（`settings.ts:3-11`）。Ctrl+T 在三种之间循环。

**PotPlayer 规格**：`cfg:OnTopMode=3`；取值 0 = 不，1 = 总是，2 = 播放时，3 = 全屏播放时（字符串 4561，CmdID 10968）（C §2.1，取值映射已确认）。菜单 M:101_0_15_3_0–3，Ctrl+T 循环切换（`ID_ETC_NEXTONTOP`）。

**目标行为**：

1. 四种方式（单选）：不置顶、总是置顶、播放时置顶、全屏播放时置顶（新增，order 729）。默认「全屏播放时置顶」。
2. 「全屏播放时」：`window.state.fullscreen && phase === "ready" && !paused` 时置顶，否则不置顶。
3. Ctrl+T 的循环顺序：不 → 总是 → 播放时 → 全屏播放时 → 不（与 PotPlayer 的取值顺序一致）。
4. OSD：「置顶: 全屏播放时」。
5. 已经保存过设置的用户保持原值。

**实现要点**：`settings.ts` 的 `OnTop` 类型与 `ON_TOP` 列表加 `"fullscreen"`，默认值改为 `"fullscreen"`；`on-top.ts` 的 `wanted` 加分支，并订阅 `window.onChange`（全屏切换时重新计算）。

**核心 API**：不新增。

**跨平台**：macOS 全屏是独立空间，主窗口在那里本来就在最前，「全屏播放时」在 macOS 上没有可见效果（功能需求 H05 第 6 条的理由）。这不是做不到，只是效果不可见，所以仍然提供并作为默认值，保证两个系统的设置一致；Windows 上全屏为置顶的无边框窗口，效果可见（其它窗口弹出时不会盖住播放器）。

**验收**：首次启动菜单「屏幕 ▶ 置顶」圆点在「全屏播放时置顶」；Windows 上全屏播放时打开一个会自动弹到前台的程序：播放器仍在最上；窗口模式下不置顶。Ctrl+T 四次回到原方式。

**实施（第 2 轮，2026-10-21）**：已实现，只用现有 `setAlwaysOnTop(boolean)`：`on-top.ts` 的 `wanted` 加「全屏 且 正在播放」分支，并在 `window.onChange` 时重新计算；仍只在值变化时发送。不需要 `setAlwaysOnTop` 的 `mode` 参数（那是 IA-WIN-07 标题栏图标用的，第 3 轮随 IA-CORE-07 再提）。设置页「选项 ▶ 播放 ▶ 窗口」也有这四种单选。功能需求 H05 已同步改写。

### IA-WIN-04 按视频尺寸调整时的上下限（P3）

**审计行**：S-04。

**现状**：只有按屏幕可用区缩小、按最小尺寸放大两条规则（参考文档 §6.6），没有用户可设的上下限。

**PotPlayer 规格**：Dialog(490)：相对于桌面的百分比上下限，`cfg:2xMinSizeUse/2xMaxSizeUse=0`（默认不启用）；`ResizeMinWidth=300`、`ResizeMinHeight=200`。

**目标行为**：window 的设置页「窗口」加两组选项（默认都不启用）：「最小不小于桌面的 N%」「最大不超过桌面的 N%」（N 为 10–100）。启用时，G-02 的自动调整与手动倍数（H03）在调用 `setVideoArea` 之前先按这两个百分比夹住视频区尺寸（按屏幕可见区计算，保持宽高比）。

**实现要点**：桌面尺寸用 `plugin.app.system()` 的显示器信息（参考文档 §3.5，已有）。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：启用「最大不超过桌面的 50%」后在 2560×1440 屏幕按 4（2 倍）播放 1080p：视频区不超过 1280×720。

### IA-WIN-05 更多窗口尺寸预设：最大化、窗口 +/−、窗口移动（P3）

**审计行**：P-08、X-02。

**现状**：只有 0.5 / 1 / 1.5 / 2 倍与适应比例。

**PotPlayer 规格**：「屏幕」菜单 M:101_0_19_*：最大（含「最大化」不盖任务栏、「最大化+」盖住任务栏，M:101_0_19_11/12）、迷你、适应比例、自定义、窗口 +/−、窗口移动（快捷键 1–9、0、Ctrl+Alt+小键盘）。各项的具体按键以 menu-tree 为准，实施时逐项核对。

**目标行为**：

1. 「屏幕 ▶ 窗口放大 / 窗口缩小」：视频区按 10% 放大 / 缩小（Vestra 取值，PotPlayer 步长未提取），用现有 `setVideoArea`，不需要新接口。
2. 「屏幕 ▶ 最大化 / 最大化+」（单选，可取消）：用 `setMaximized("maximized" | "cover")`。
3. 「屏幕 ▶ 窗口移动 ▶ 上 / 下 / 左 / 右」：每次移动 10 点（Vestra 取值），Ctrl+Alt+小键盘 8 / 2 / 4 / 6；用 `moveBy`。
4. 「自定义」尺寸：PotPlayer 的自定义对话框未提取，不做。
5. 全屏中都禁用，原因「全屏时不可用」。

**核心 API**：依赖 IA-CORE-15 的 `setMaximized`、`moveBy`、`WindowState.maximized`（第 1 条不依赖）。

**跨平台**：「最大化+」在 macOS 上盖住菜单栏与 Dock 区域，在 Windows 上盖住任务栏。

**验收**：按「窗口放大」：视频区变大 10%；Ctrl+Alt+Num6：窗口右移；「最大化+」后窗口盖住任务栏 / Dock。

### IA-WIN-06 调整窗口尺寸时保持比例（P3）

**审计行**：S-11。

**PotPlayer 规格**：「调整窗口尺寸时保持比例」M:101_0_18_20（CmdID 10341）；默认值未提取（审计只记「可选」），本文取默认关。

**目标行为**：「屏幕 ▶ 调整窗口尺寸时保持比例」（勾选，默认关）；打开时用户拖动窗口边框，视频区保持当前视频的显示宽高比；没有视频时不锁定。换来源时按新视频重新锁定。

**核心 API**：依赖 IA-CORE-15 的 `setAspectLock`。

**跨平台**：macOS 用 `NSWindow.contentAspectRatio`（需要把底栏与侧栏的固定尺寸扣掉，宿主计算），Windows 在 `WM_SIZING` 中修正矩形；都在平台模块。

**验收**：勾选后播放 16:9 视频，拖窗口右下角：视频区始终没有黑边。

### IA-WIN-07 标题栏置顶按钮（P3）

**审计行**：W-06 的一部分（IA-CORE-07 标题栏上的置顶状态按钮）。

**目标行为**：「切换置顶方式」命令（Ctrl+T）的默认绑定加 `Title:OnTop`；每次置顶方式变化时调用 `setAlwaysOnTop(wanted, { mode })`，让标题栏显示 4 种图标之一（即使实际是否置顶没变，例如「播放时」在暂停中）。

**核心 API**：依赖 IA-CORE-07 的 `Title:OnTop` 与 `setAlwaysOnTop` 的 `mode` 选项。

**跨平台**：无差异。

**验收**：点标题栏置顶图标四次：OSD 依次显示四种方式，图标随之变化。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `window.setMaximized(mode)`、`window.moveBy(dx, dy)`、`WindowState.maximized`（IA-CORE-15） | IA-WIN-05 |
| `window.setAspectLock(ratio \| null)`（IA-CORE-15） | IA-WIN-06 |
| 绑定输入 `Title:OnTop`；`setAlwaysOnTop(onTop, { mode })`（IA-CORE-07） | IA-WIN-07 |

G-02、G-06、IA-WIN-01–04 只用现有 API。

---

## 播放信息（`info`）

**位置**：`src/plugins/info/`

### IA-INFO-01 简要信息补上字幕名与章节（P2）

**审计行**：O-09。

**现状**：Shift+Tab 显示文件名、时间、视频与音频编码，3 秒（`main.ts:103-118` 的 `brief`）；没有字幕名和章节。

**PotPlayer 规格**：简要信息包括文件名、字幕名、时间、章节、编码（`cfg:ShortMessage*=1`，C §5）。

**目标行为**：第二行在时间之后插入当前章节标题（`timeline.marks` 中 `kind === "chapter"` 且时间不晚于当前位置的最后一项；无标题时显示「第 N 章」），之后是编码；第三行显示当前字幕轨名（`subtitles.state.selected` 的 `title`，空则显示语言或「字幕」；没有选中字幕时不显示这一行）。每行仍按视频区宽度截断。

**实现要点**：manifest 加 `subtitles.read`；`timeline.marks` 已有（需要 `playback.read`）。

**核心 API**：不新增。**跨平台**：无差异。

**验收**：播放带章节和字幕的 MKV，按 Shift+Tab：显示文件名；`12:34 / 45:00 · 第二章 标题 · H.264 1920×1080 · AAC 2 声道`；字幕名。

**实施（第 2 轮，2026-10-21）**：已实现（`info/main.ts`；manifest 增加 `subtitles.read`）。与上文的差异：

- 章节取自 `player.chapters()`（核心的章节表，每个来源读一次），不取 `timeline.marks`：时间轴上的章节标记由 marks 插件发布，用它会让简要信息依赖另一个插件是否加载（工作流 §1「插件独立」）。规则相同：开始时间不晚于当前位置的最后一章，没有标题时「第 N 章」。
- 字幕选择没有推送事件，简要信息显示期间每秒随 `stats()` 一起 `subtitles.refresh()` 一次。
- 「按视频区宽度截断」：列数按 `window.state.videoArea.width` 估算（`(宽 − 32) / 6.5`，32 = 信息块左右边距与内边距），不少于原来的 44 列；所以 1280 宽的窗口能完整显示验收里那一行，最窄窗口（318）仍按 44 列截断。

### IA-INFO-02 杂项信息（Scroll Lock）与「仅全屏时显示」（P3）

**审计行**：O-08、F-11、O-12。

**现状**：没有杂项信息。

**PotPlayer 规格**：杂项信息：播放时间、百分比、序号、文件名、章节；可设「仅全屏时显示」（Dialog(375) 3194 / 3045 / 3047）；开关键 Scroll Lock。

**目标行为**：

1. 命令「杂项信息」（勾选，绑定 `ScrollLock`，菜单「视图」类位置按 menus.md 的 info 分组）：开时在视频区持续显示一行：`12:34 / 45:00 (27%) · 文件名 · 章节标题`。
2. 「序号」（列表里的第几项）：播放序列由 playlist 提供，info 插件读不到，**本轮不显示**（按 §4.3 降低需求）。以后如果要做，需要核心在 `PlayerState` 里给出序列位置，届时单独提请求。
3. 设置「仅全屏时显示」（默认关，info 的设置页）。
4. 显示位置：用 info 插件已有的信息块（`osd.panel()`），与 Tab 的完整信息共用一块时排在最上。

**实现要点**：复用 `main.ts` 的刷新定时器（每秒）。

**核心 API**：依赖 IA-CORE-08 的 `ScrollLock` 键名（仅快捷键；菜单入口不依赖）。

**跨平台**：Mac 键盘一般没有 Scroll Lock，macOS 上从菜单开关（见 IA-CORE-08）。

**验收**：Windows 上按 Scroll Lock：视频区出现杂项信息行，再按消失；打开「仅全屏时显示」后窗口模式下不显示，全屏时显示。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| 绑定键名 `ScrollLock`（IA-CORE-08） | IA-INFO-02 |

---

## 专辑封面与空闲画面（`visualizer`）

**位置**：`src/plugins/visualizer/`

### G-16b 加载中显示加载动画（P1）

**审计行**：G-16（插件部分）、I-07。核心部分见 G-16a。

**现状**：`main.ts:109` 的注释「loading and error keep what is shown」：加载中保持原样；背景层在加载中也不生效（核心规则）。

**PotPlayer 规格**：`State=CMD_LOADING_LOGO`（`Logo.txt:22-38`，A §2.4，**确认**）：74×74 转圈动画，12 帧、每帧 50 ms（`CMD_ANIMATION`），画在视频区中央向上偏移 30 px；下方是加粗 15 px 的提示文字（`#777777`），向下偏移 30 px；底色 `#101010`。

**目标行为**：

1. `player.state.phase === "loading"` 且背景层生效时，画加载界面：中央 74×74 的转圈（12 个刻度，每 50 ms 转一格；字形自绘，不用 PotPlayer 素材），下方一行粗体 15 px 文字，颜色为主题的次要文字色：「正在打开：<来源名>」（来源名取文件名或链接的主机名加路径末段，超长截断）。
2. 进入 `ready` 后按现有规则判断（有画面 → 背景层不生效；纯音频 → 封面）。进入 `error` 时保持加载界面之前的内容（核心规则不变），由出错提示（OSD）说明。
3. 加载界面与空闲徽标、封面互斥。

**实现要点**：`ui/backdrop.slint` 加 `loading` 状态与 `Timer` 驱动的帧序号；`main.ts` 在 `phase` 为 `loading` 时设置 `mode: "loading"` 与文字。

**核心 API**：不新增；依赖 G-16a 的语义变化（`loading` 时 `effective` 为 `true`）。

**跨平台**：无差异。

**验收**：打开一个网络链接：在出画面之前，视频区中央有转圈和「正在打开：…」；出画面后消失。

### 本章核心 API 请求

无（依赖 G-16a 的行为变化）。

---

## 快捷键设置（`hotkeys`）

**位置**：`src/plugins/hotkeys/`

### IA-HK-01 鼠标页列出新增的鼠标输入（P3）

**审计行**：M-04、M-05、M-06、M-07、M-10（随 IA-CORE-08）。

**现状**：`keys.ts:110-120` 列出 11 种鼠标输入。

**PotPlayer 规格**：鼠标配置表的 21 种事件（C §4.2）。

**目标行为**：鼠标页增加：右键双击、中键双击、侧键后退双击、侧键前进双击、左键按住、右键按住、中键按住、侧键后退按住、侧键前进按住、按住侧键后退时滚轮上 / 下、按住侧键前进时滚轮上 / 下。右键双击、右键按住的说明里注明「绑定后右键菜单在松开并等待双击间隔后弹出」（IA-CORE-08 第 4 条）。

**实现要点**：`keys.ts` 的鼠标输入表加项。

**核心 API**：依赖 IA-CORE-08。**跨平台**：无差异。

**验收**：鼠标页能选到「中键双击」并绑定到「全屏」，绑定生效。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| 新的鼠标绑定输入（IA-CORE-08） | IA-HK-01 |

---

## 画面调节（`video`）

**位置**：`src/plugins/video/`

### IA-VID-01 控制面板「视频」页（P3）

**审计行**：B-05 的一部分（控制面板，IA-CORE-09）；UA UI-TOOL-06 的「独立窗口」前提作废，样式规格仍有效。

**PotPlayer 规格**（UA §7.1）：视频页 295×159：每行 = 标签 65 px（右对齐，11 px）+ 滑块 154×12（2 px 线，填充 `#2282f2`，thumb 14×12）+「默认」41×18，行距 24；4 行：亮度、对比度、饱和度、色调；下方 51×13 复选：柔化 / 锐化 / 去块；右下「截图」41×22。改动立即生效，每项独立复位。

**目标行为**：video 往「视频」页签挂区块（order 100）：亮度、对比度、饱和度、色调四行（与现有画面调节同一套滤镜与状态，Q/W/E/R/T/Y/U 快捷键照常）；下方三个复选：柔化（高斯模糊）、锐化（E04）、去块（E06），与菜单同步。「截图」按钮：截图属于 capture 插件，video 不能调用（插件不通信），**本区块不放截图按钮**（按 §4.3 降低需求）。现有「画面调节」工具窗口保留为完整设置（菜单入口不变）。

**实现要点**：新文件 `ui/control.slint`，状态与现有窗口共用同一个模型。

**核心 API**：依赖 IA-CORE-09。**跨平台**：无差异。

**验收**：F7 打开控制面板切到「视频」页，拖亮度滑块：画面立即变亮，画面调节窗口里同一滑块同步；点「默认」：亮度复位。

### IA-VID-02 Alt / Ctrl / Shift + 拖动调整画面帧位（P3，默认关）

**审计行**：M-11（帧位部分）。

**PotPlayer 规格**：`cfg:MousePanScanControl=0`（Alt / Ctrl / Shift + 拖动调整画面帧位，默认关）（C §4.2）。

**目标行为**：设置「用 Alt / Ctrl / Shift + 拖动调整画面」（默认关）。打开后：Alt + 拖动平移画面（E13 的平移），Ctrl + 拖动缩放画面（上拖放大），Shift + 拖动（PotPlayer 语义未知，不绑定）。

**核心 API**：依赖 IA-CORE-17（`plugin.input.onVideoDrag`）；维护者拒绝时降级为离散的绑定命令（每 10 px 一步）。

**跨平台**：macOS 上 Ctrl 改用 ⌘（见 IA-CORE-17）。

**验收**：打开设置后按住 Alt 在视频上拖动：画面跟随平移；关闭设置后同样操作：不平移（无修饰键拖动移动窗口不受影响）。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `plugin.controlPanel.section`（IA-CORE-09） | IA-VID-01 |
| `plugin.input.onVideoDrag`（IA-CORE-17） | IA-VID-02 |

---

## 音频工作台（`audio`）

**位置**：`src/plugins/audio/`

### IA-AUD-01 控制面板「音频」页（P3）

**审计行**：B-05 的一部分（控制面板，IA-CORE-09）；UA UI-TOOL-07 的「独立窗口」前提作废，样式规格仍有效。

**PotPlayer 规格**（UA §7.1）：音频页：均衡器开关 82×14 + 预设名 97×22 +「预置」41×22；10 条竖滑块 14×70，间距 23；主音量、波形音量两条竖滑块。

**目标行为**：audio 往「音频」页签挂区块（order 100）：均衡器开关、预设名、「预置」按钮（点击弹出预设列表；区块不是面板，不能用 `Panel.showMenu`，所以在区块里用 Slint 的 `PopupWindow` 自绘）；竖向均衡器滑块（段数按现有实现，左对齐排列）；右侧一条主音量竖滑块（0–100，用 `player.setOutput`，与底栏同步，G-13b）。「波形音量」是 Windows 混音器的程序音量，Vestra 没有对应，不放。现有音频工作台窗口保留为完整设置。

**实现要点**：新文件 `ui/control.slint`。

**核心 API**：依赖 IA-CORE-09。

**跨平台**：「波形音量」只有 Windows 有系统概念，两个系统都不做，保持一致。

**验收**：控制面板「音频」页开均衡器、选预设：声音变化，与菜单同步；拖主音量：底栏音量条同步。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `plugin.controlPanel.section`（IA-CORE-09） | IA-AUD-01 |

---

## 字幕（`subtitles`）

**位置**：`src/plugins/subtitles/`

### IA-SUB-01 控制面板「字幕」页（P3）

**审计行**：B-05 的一部分（控制面板，IA-CORE-09）；UA UI-TOOL-08 的「独立窗口」前提作废。

**PotPlayer 规格**（UA §7.1）：字幕页：字体 160×22；位置 上 / 下 / 左 / 右 51×22（间距 3）；同步 慢 / 快 / 默认 69×22。

**目标行为**：subtitles 往「字幕」页签挂区块（order 100）：字幕轨下拉（160×22，含「关闭」）；位置 上 / 下（51×22，对应 D07 的字幕位置快捷键；左 / 右 Vestra 不支持水平移动，不放）；同步 慢 / 快 / 默认（69×22，对应 `<` `>` `/`）。现有字幕工作台保留为完整设置。

**实现要点**：新文件 `ui/control.slint`，与现有状态共用。

**核心 API**：依赖 IA-CORE-09。**跨平台**：无差异。

**验收**：控制面板「字幕」页点「快」：字幕提前 0.5 秒，OSD 照常；切换字幕轨：字幕菜单同步。

### IA-SUB-02 Alt / Ctrl + 拖动移动字幕（P3，默认关）

**审计行**：M-11（字幕部分）。

**PotPlayer 规格**：`cfg:MouseCaptionMove=0`（Alt / Ctrl + 拖动移动字幕，默认关）（C §4.2）。

**目标行为**：设置「用 Alt / Ctrl + 拖动移动字幕」（默认关）。打开后 Ctrl（macOS ⌘）+ 竖向拖动改变字幕的垂直位置（`marginPercent`），松开时 OSD「字幕位置: N%」。与 IA-VID-02 的修饰键冲突时（两者都打开、都用 Alt），以先登记的为准（IA-CORE-17 规则），所以字幕默认只用 Ctrl，画面只用 Alt。

**核心 API**：依赖 IA-CORE-17；降级办法同 IA-VID-02。

**跨平台**：同 IA-CORE-17。

**验收**：打开设置后 Ctrl / ⌘ + 向上拖动：字幕上移。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `plugin.controlPanel.section`（IA-CORE-09） | IA-SUB-01 |
| `plugin.input.onVideoDrag`（IA-CORE-17） | IA-SUB-02 |

说明：底栏 CC 按钮（IA-PB-06）由 playback 直接用 `plugin.subtitles.select` 选择隐藏字幕轨，不需要 subtitles 插件配合；subtitles 的「显示字幕」（Alt+H）记住的「上次显示的轨」可能因此与 CC 按钮的选择不同步，这是插件独立带来的已知差异，用户可见的结果是 Alt+H 恢复的是 Alt+H 自己记住的轨。

---

## 章节、书签与区段（`marks`）

**位置**：`src/plugins/marks/`

### IA-MK-01 控制面板「播放」页的 A-B 循环区块（P3）

**审计行**：B-05 的一部分（控制面板，IA-CORE-09）。

**PotPlayer 规格**（UA §7.1）：播放页含 AB 循环。

**目标行为**：marks 往「播放」页签挂区块（order 200，在 playback 的区块之下）：「设置 A」「设置 B」「清除」三个按钮（69×22）与当前区段文字（`01:23 – 02:45`），与 B23 的命令同一套逻辑。

**核心 API**：依赖 IA-CORE-09。**跨平台**：无差异。

**验收**：控制面板「播放」页依次点「设置 A」「设置 B」：进度条出现 A-B 区段并循环；点「清除」：区段消失。

### 本章核心 API 请求

| 签名 | 来源 |
| --- | --- |
| `plugin.controlPanel.section`（IA-CORE-09） | IA-MK-01 |

---

## 附录 A：待 Windows 实机观察（审计 Q-xx）

这些项的 PotPlayer 规格静态资料无法确定，本文**不据此改动**；部分整改条目里先用了 Vestra 取值，观察后校准。观察条件同审计：PotPlayer 260819 全新安装、默认配置、100% DPI。

| 编号 | 观察什么（摘要，全文见审计 §11） | 观察后影响本文哪些条目 / 现状 |
| --- | --- | --- |
| Q-01 | 鼠标页默认动作；单击是否立即暂停 | IA-CORE-08 的默认绑定、按住阈值；M-01/02/06/08 现状不改 |
| Q-02 | 打开附着列表时窗口变宽还是挤压视频；窄窗口时怎样 | G-18 只修状态显示；W-05、L-09、S-07、S-12 现状不改 |
| Q-03 | 全屏控制栏的唤出区域、隐藏延时、动画 | UI-SHELL-09 的 3000 ms 与「任意移动唤出」不改；IA-CORE-07 第 7 条（全屏时标题栏） |
| Q-04 | 全屏暂停时控制栏与指针是否隐藏 | 现状（暂停也隐藏底栏）不改 |
| Q-05 | 控制栏、进度条、音量条上的滚轮 | 现状（没有反应）不改 |
| Q-06 | 控制栏空白处、标题栏、页签条上右键弹出哪个菜单 | 现状不改；IA-CORE-07 的标题栏右键暂不弹菜单 |
| Q-07 | 空闲时点播放、双击 / 单击 Logo | 现状不改 |
| Q-08 | 时间格式、剩余时间的显示形式 | G-14 只改保存与点击区域 |
| Q-09 | 浮动列表是否跟随主窗口、吸附距离、透明度与置顶钉的效果 | IA-CORE-10 的 10 px 吸附距离；「跟随」不做 |
| Q-10 | 控制面板、非模态对话框里主窗口快捷键是否生效 | IA-CORE-09 第 6 条；K-08 现状不改 |
| Q-11 | 拖动过程中是否出现「播放 / 加入列表」落点区域 | R-08 不做；G-11、G-12 不受影响 |
| Q-12 | 纯音频界面 | G-09、A-01、A-02 不做 |
| Q-13 | 全屏右边缘弹出的速度、宽度、收起延时、F6 效果 | G-01 的 4 px、150 ms、500 ms、底栏宽度 |
| Q-14 | Tab 完整信息的字段 | O-10 现状不改 |
| Q-15 | OSD 停留时长、连续按键时替换还是叠加、与 Tab 信息的上下位置 | O-05、O-11 现状不改；G-10 的信息块位置 |
| Q-16 | 列表键盘移动与切换文件时是否自动滚动、拖动时靠边滚动 | G-08 的滚动方式与速度 |
| Q-17 | Esc 在各处的效果 | K-12、K-13、K-15 现状不改（K-15 的 Vestra 内部不一致也等观察后一并处理） |
| Q-18 | 右键菜单按下还是松开弹出；子菜单展开延时；按住右键 | G-15 已按 Win32 惯例取松开与系统延时，观察用于确认 |
| Q-19 | 迷你尺寸的尺寸、是否隐藏标题栏与控制栏、是否置顶 | IA-WIN-01 的 400×300；IA-WIN-02 |
| Q-20 | 全屏时按窗口倍数键 | F-14 现状（禁用）不改；IA-CORE-04「全屏+」的再次按下语义 |
| Q-21 | 窗口从 800 拖到 320 时各按钮消失的宽度 | IA-PB-02 的阈值 |
| Q-22 | 失去焦点时标题栏与控制栏颜色 | W-07 不做；IA-CORE-07 第 9 条 |

本文新增的观察点（审计没有列，执行本文时发现需要确认）：

| 编号 | 观察什么 | 影响 |
| --- | --- | --- |
| IA-Q-01 | 打开本地小文件时，加载界面是否会闪一下；是否有显示延时 | G-16a 第 4 条 |
| IA-Q-02 | 已有列表时打开一个网络链接（Ctrl+U），列表是否被清空 | IA-PL-01 第 5 条 |
| IA-Q-03 | 音量 100% 时按 ↑，是否能超过 100%；首选项里有没有「音量放大」 | G-13a |
| IA-Q-04 | 开始播放时自动调整窗口时是否有 OSD；窗口最大化时打开文件是否还原再调整 | G-02 第 6 条 |
| IA-Q-05 | 「全屏播放时置顶」在全屏暂停时是否取消置顶 | IA-WIN-03 第 2 条 |

## 附录 B：逆向资料补充提取建议（审计 D-xx）

| 编号 | 缺口（摘要，方法见审计 §12） | 补上后校准本文哪些条目 |
| --- | --- | --- |
| D-01 | 鼠标内置默认动作 | IA-CORE-08 的默认绑定与按住阈值 |
| D-02 | 自动隐藏的计时与区域常量（`SetTimer` 毫秒数、右边缘像素阈值） | G-01 的 4 px 与收起延时；G-04；全屏控制栏 3000 ms |
| D-03 | `AudioSkin.xml`、`BottomAudio.txt`、OSC Inside 层 | G-09（纯音频布局） |
| D-04 | 皮肤属性语义：`VisMinLeft`、`VisParentMinWidth`、`AutoVisible`、`DownFlag`、`AltImage` | IA-PB-02、IA-PB-05、IA-PB-06、IA-PB-03 |
| D-05 | 对话框控件 ↔ 配置键 ↔ 默认值对照表 | IA-CORE-06 各选项、IA-PL-02、G-02 的设置项与默认值 |
| D-06 | 加速键表 157 / 118 全量导出 | IA-CORE-04、IA-WIN-05 的按键；Esc（Q-17） |
| D-07 | 命令的窗口状态前置条件（`ON_UPDATE_COMMAND_UI`） | IA-WIN-05「全屏时禁用」等可用条件 |
| D-08 | OSD 的时长、队列、位置取值、字号公式 | G-10 的字号公式；IA-CORE-06 的位置选项 |
| D-09 | 列表行高、选中与焦点框的绘制 | IA-PL-03、IA-PL-04 |
| D-10 | 状态切换类字符串的索引 | IA-CORE-04「全屏+」语义 |
| D-11 | `reg?` 与含义不明的键（`SkinTransparent=180` 等） | 全屏底栏是否半透明（B-20） |
| D-12 | 窗口所有关系与按键转发 | IA-CORE-09 第 6 条、IA-CORE-11 |

本文新增的提取建议：

| 编号 | 缺口 | 建议怎么提取 | 影响 |
| --- | --- | --- | --- |
| IA-D-01 | `AudioVolumeMax=1000`（0x2274）的含义与放大开关 | 交叉引用 0x2274 的读取，看是否作为 ↑ 的上限、在哪个对话框控件里设置 | G-13a 的「音量放大」 |
| IA-D-02 | `OpenWithSameName` 与「打开 URL」时的列表处理 | 找打开命令的处理函数中对 `AppendFilePL`（0x18057f4bc 写入的字段）的读取分支，看 URL 是否走同一分支 | IA-PL-01 第 5 条 |
| IA-D-03 | 迷你尺寸（CmdID 10231）的目标尺寸 | 反汇编 10231 的处理函数，找传给 `SetWindowPos` 的宽高来源 | IA-WIN-01 |
| IA-D-04 | 标题栏文件类型文字（`CMD_FILE_TYPE_MESSAGE`）的取值 | 找该消息的格式化函数 | IA-CORE-07 第 3 条 |

## 附录 C：优先级与轮次划分

原则：先核心后插件；同一轮内，核心的行为改动（不改 API）先于需要 API 协商的改动；插件条目在它依赖的核心条目验收后开工，不依赖核心的插件条目可以与核心轮并行（工作流 §3「开工门槛只看实际依赖」）。

### 第 1 轮：核心行为（P1，无新 API）

| 条目 | 说明 |
| --- | --- |
| G-04 | 底栏悬停锁（G-01 复用同一机制，先做） |
| G-01 | 全屏右边缘弹出侧栏 |
| G-03 第一步 | 菜单窗口内滚动（保底） |
| G-15 | 子菜单延时、右键松开弹出 |
| IA-CORE-01 | Esc 逐级关闭（与 G-15 同改 `menu-view.slint`） |
| G-05 | 拖动画面移动窗口 |
| G-10 | OSD 字号 |
| G-13a | 音量键上限 |
| G-16a | 加载中背景层 |
| IA-CORE-03 | 尺寸记忆区分最大化 |
| IA-CORE-12 | 画中画期间双击 |
| IA-CORE-04 第 1 条 | Alt+Enter |

维护者同轮更新参考文档 §5.5、§6.3、§6.5 的语义说明，并通知 playback、playlist、visualizer。

### 第 2 轮：插件对齐（P1 / P2，只用现有 API）

可以与第 1 轮并行开工的：G-07、G-14、IA-PB-01、G-08、G-11、G-12、IA-PL-01、IA-PL-02、G-02、G-06、IA-WIN-01、IA-WIN-02、IA-WIN-03、IA-INFO-01。

依赖第 1 轮的：G-13b（G-13a）、G-18（G-01 的语义）、G-16b（G-16a）。

### 第 3 轮：核心第二批（P2 / P3）

| 条目 | 是否需要 API 协商 |
| --- | --- |
| G-03 第二步（菜单覆盖层窗口） | 否 |
| IA-CORE-06 宿主设置页 | 否 |
| IA-CORE-05 自动隐藏控制栏 | 否 |
| IA-CORE-13 最小高度 | 否 |
| IA-CORE-14 隐藏字幕轨 | 否 |
| IA-CORE-10 浮动窗口标题栏与吸附 | 否 |
| IA-CORE-11 一起最小化 | 否 |
| IA-CORE-04 第 2、3 条 | 否 |
| IA-CORE-07 自绘标题栏 | 是（`Title:OnTop`、`setAlwaysOnTop` 的 `mode`） |
| IA-CORE-08 绑定输入补齐 | 是 |
| IA-CORE-09 控制面板 | 是 |
| IA-CORE-15 主窗口接口扩展 | 是（建议先接受 `active`、`minimized`） |
| IA-CORE-16 `cacheSpeed` | 是（兼容扩展） |
| IA-CORE-02 助记符 | 是（最低） |
| IA-CORE-17 视频区拖动 | 是（最低，可降级） |

需要协商的条目按工作流 §4.3 逐条提交；协商期间先做不需要协商的条目。

### 第 4 轮：插件第二批（P3）

依赖第 3 轮各自的核心条目：IA-PB-02、IA-PB-03、IA-PB-04（不依赖核心，可提前）、IA-PB-05、IA-PB-06、IA-PB-07、IA-PB-08、IA-PL-03、IA-PL-04（不依赖核心，可提前）、IA-WIN-04（不依赖核心，可提前）、IA-WIN-05、IA-WIN-06、IA-WIN-07、IA-INFO-02、IA-HK-01、IA-VID-01、IA-VID-02、IA-AUD-01、IA-SUB-01、IA-SUB-02、IA-MK-01。

### 每轮结束时

- 协调者按各条「验收」做真实 GUI 测试（工作流 §6.2），macOS 实测；Windows 只做编译与可移植性检查（工作流 §6.1），等有 Windows 机器时补测「跨平台」一栏的 Windows 行为。
- 交互设计负责人改写 §0.2 表中被取代的功能需求与界面整改条目，更新 menus.md。

### 条目统计

| 章 | 条目数 | P1 | P2 | P3 |
| --- | --- | --- | --- | --- |
| 核心 | 25 | 8 | 5 | 12 |
| playback | 12 | 3 | 2 | 7 |
| playlist | 7 | 4 | 1 | 2 |
| window | 9 | 2 | 3 | 4 |
| info | 2 | 0 | 1 | 1 |
| visualizer | 1 | 1 | 0 | 0 |
| hotkeys | 1 | 0 | 0 | 1 |
| video | 2 | 0 | 0 | 2 |
| audio | 1 | 0 | 0 | 1 |
| subtitles | 2 | 0 | 0 | 2 |
| marks | 1 | 0 | 0 | 1 |
| 合计 | 63 | 18 | 12 | 33 |

IA-CORE-04 的第 1 条按 P2 计，第 2、3 条为 P3。
