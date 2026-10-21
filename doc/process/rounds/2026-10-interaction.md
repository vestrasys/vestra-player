# 交互对齐轮

分支：`rebuild/interaction`（自 `main`）。依据：[交互对齐整改文档](../../product/interaction-alignment.md)（下称「整改文档」），以[细粒度审计](../../product/interaction-gap-audit.md)为准。流程：先按社区工作流 §5 自上而下改核心，再按 §4 自下而上（§4.3 保守协商）；跨平台遵守 §1.5。

## 1. 本轮范围（整改文档附录 C 第 1、2 轮）

- **A 线，维护者**：附录 C 第 1 轮的全部核心条目：G-04、G-01、G-03 第一步、G-15、IA-CORE-01、G-05、G-10、G-13a、G-16a、IA-CORE-03、IA-CORE-12、IA-CORE-04 第 1 条。同步更新参考文档 §5.5、§6.3、§6.5，并在 §4 给出插件迁移通知。
- **B 线，作者**：附录 C 第 2 轮中不依赖核心的条目，与 A 线并行开工：
  - playback：G-07、G-14、IA-PB-01
  - playlist：G-08、G-11、G-12、IA-PL-02
  - window：G-02、G-06、IA-WIN-01、IA-WIN-02、IA-WIN-03
  - info：IA-INFO-01

  依赖 A 线的条目等 A 线交付后再做：G-13b、G-18、G-16b。
- **暂缓，待用户确认**：IA-PL-01（打开文件时先清空列表）、IA-CORE-07（自绘标题栏与主菜单按钮）、IA-CORE-09 系列（F7 控制面板）。这三项与用户此前明确的决定或使用习惯冲突，影响面大。

## 2. 规则

- 作者：只改自己的插件目录，以及需求文档 / 整改文档里对应本插件的章节；需要核心改动时按 §4.1 提请求。
- 维护者：只改核心、SDK、参考文档，`unsafe` 只放在 `src/host/unsafe/` 下。
- 都不 commit，不操作 GUI。GUI 实测由协调者做。

## 3. 记录

（各阶段完成后补充）

### 3.1 A 线交付（维护者，2026-10-21）

**结果**：附录 C 第 1 轮核心条目全部实现，未新增插件 API。三个提交（分支 `rebuild/interaction` 之上，未 push）：

| 提交 | 条目 |
| --- | --- |
| Interaction core: bottom bar hover lock and fullscreen sidebar overlay | G-04、G-01 |
| Interaction core: context menu scrolling, submenu delay, right-button release, Esc | G-03 第一步、G-15、IA-CORE-01；另修 `ToolTipArea`（作者报告） |
| Interaction core: video area, OSD, volume, loading backdrop, window size, PiP, Alt+Enter | G-05、G-10、G-13a、G-16a、IA-CORE-03、IA-CORE-12、IA-CORE-04 第 1 条 |

**实现说明与偏差**（其余按整改文档各条「目标行为」「实现要点」）：

- G-04 / G-01：底栏和全屏侧栏弹出层外包一层只做悬停检测的 `TouchArea`（作为内容的父元素，Slint 的悬停过滤先于子元素运行，不吃子元素的事件），输出 `bottom-hover` / `sidebar-hover`；视频区输出 `video-hover`。悬停锁存在 `Policy.hover`，插件的 `setInteraction` 写不掉。`Policy` 新增 `set_hover`、`pointer`、`tick`，`set_preferred` 多了 `now`。弹出层是全屏时单独的一份面板组件实例（进入 / 退出全屏时重建，与以前全屏时销毁侧栏一致；组件里的本地状态如滚动位置不跨全屏保留，插件属性在 global 上不受影响）。弹出期间底栏宽度立即收窄（不随 150ms 滑动做动画）。全屏时拖放到弹出层上交给侧栏面板（`drops.panel`）。指针停在底栏上时也算「在弹出层左边」，开始 500ms 离开计时。
- G-04 第 4 条、G-05 第 5 条、G-10 第 4 条的设置页开关属于 IA-CORE-06（第 3 轮），本轮未做，行为固定为默认值。
- G-03 第一步：滚轮按 Slint 报告的像素位移滚动，没有严格「按行」（触控板逐行会太快）；箭头条 12px、每 50ms 一行。滚动偏移由宿主保存（`menu_scroll`），所以展开子菜单不会丢失父级的滚动位置；子菜单定位按父行滚动后的位置。
- G-15：延时状态机是 `menu::HoverDelay`（纯数据，有单元测试），宿主每 16ms tick 提交到期的路径；`menu-hover` 回调多了一个 `bool`（点击 / 键盘为立即）。平台函数 `platform::menu_show_delay_ms()`：Windows 读 `SPI_GETMENUSHOWDELAY`（`unsafe/win32` 新增一个调用），macOS 与其它系统 400ms。
- G-05：「无修饰键」包括 ⌘（Slint 的 meta）。拖动后系统可能吞掉松开事件，下一次按下若与拖动的按下凑成双击，这次双击被忽略（`drag-swallow`）。
- G-13a：整改文档「实现要点」的代码会让 150% 时按 ↓ 直接变成 100%；本轮按「目标行为」第 2 条「减 5（不变）」实现：`volume-up` 停在 100 / 已高于 100 时不变，`volume-down` 照旧减 5、下限 0（150 → 145）。
- G-16a：画中画用另一份「加载中沿用上一值」的判断（`composition::video_absent`），否则换来源时小窗会因加载中「没有视频」而关闭。
- `ToolTipArea`（作者按 §4.1 报告，已核实属实）：显示后执行 `self.running = false` 解除了 `running` 的绑定，每个提示只出现一次。改为私有标志 `done`，指针离开时复位并关闭提示；新增只读 `showing`；加了真实指针事件的无界面测试。

**检查**（每个提交都跑过，第 3 个提交的结果）：`src/` 下 `npm run check`、`format:check`、`boundaries`、`test:sdk`、`plugins:build` 通过；19 个插件 `npm run plugin -- test` 全部通过（playback 的 `r3b.test.ts`「I07 a resting pointer makes a picture once」在一次全量运行中失败一次，单独重跑 4 次全过，判断为计时相关的偶发失败，与本轮改动无关，记录待查）（workshop 的 `vendor/sdk` 已重同步，本轮只改了 `testing.ts`，副本无差异）；`npm run portability` 通过（Windows 目标完整检查）；`src/host/` 下 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`（262 项）通过；`grep -rnw unsafe src/host/src` 为空。没有操作 GUI。

新增测试：`window_state`（底栏悬停锁不被插件写掉；右边缘弹出、离开 500ms 收起、回来取消；3 秒空闲收起、锁住不收起、偏好隐藏与浮动时不弹出；全屏中 F6 立即弹出 / 收起）；`menu`（25 项在 182 高窗口里每项可滚到；偏移夹取；面板菜单滚动；子菜单按滚动后的父行定位；延时展开、离开取消、斜穿不切换、停留切换、命令行关闭子级）；`ui`（菜单 Esc 逐级关闭、`ToolTipArea` 重复显示、OSD 字号随视频区）；`pip`（双击放回不暂停）；`core_actions`（音量上限）；`composition`（加载中背景层、悬停与弹出层生成）；SDK（`TestHost` 全屏侧栏、音量上限、加载中背景层）。

### 3.2 插件迁移通知（语义变化，签名不变；见参考文档 §17「交互对齐轮 A 线」）

| 插件 | 变化 | 要改什么 |
| --- | --- | --- |
| playback | 全屏时侧栏 `reason: "fullscreen"` 改为「覆盖层收起」，弹出时 `effective: true` | G-18：列表按钮在 `reason === "fullscreen"` 时按「开」显示；只有 `reason === "space"` 才是半亮 |
| playback | `volume-up` 上限 100 | G-13b：音量滑块改 0–100，显示值取 `min(volume, 100)` |
| playback | 指针停在底栏上由宿主锁住 | 不用改；不需要为悬停自己调 `setInteraction` |
| playback | `ToolTipArea` 每次悬停都会显示 | G-07 可以直接用 |
| visualizer | `loading` 时背景层 `effective: true` | G-16b：`phase === "loading"` 时画加载界面；不改的话加载中会显示空闲徽标或上一个封面（`main.ts:109` 的注释已过时） |
| playlist | 全屏时侧栏是右边缘的弹出层；`setInteraction` 的锁在全屏下锁住弹出层 | 现在 `focused: focus.list \|\| focus.search`：列表本身有焦点时弹出层也不会收起。整改文档 G-01 验收第 4 条要求「焦点交回列表后再移开：列表收起」，所以 `focused` 应只在搜索框有焦点时为真。`showMenu` 建议改在右键**松开**时调用（与主菜单一致）；菜单太高时宿主自动滚动，不用改 |
| 全部 | 「全屏」默认多了 `Alt+Enter`；视频区左键拖动移动窗口 | 自己绑 `Alt+Enter` 的插件会被宿主压下（目前没有）；`Mouse:LeftClick` 在拖动时不再触发 |

### 3.3 给协调者的 GUI 实测要点（逐条对应整改文档验收）

前提：全部插件加载；macOS。Windows 只做了编译检查。

- **G-04**：1 全屏播放，指针停在底栏上 10 秒 → 底栏和指针都不消失；2 在底栏上左右移动 10 秒 → 不消失；3 移到视频中部静止 → 3 秒后底栏和指针消失。（第 4 条设置页开关本轮没有。）另看：底栏的悬停提示、按钮、进度条拖动照常可用（外层多了一层悬停检测）。
- **G-01**：1 窗口模式开着列表进全屏 → 列表不显示、视频占满；2 指针移到屏幕最右 → 列表从右滑出，盖住视频、视频不缩小，底栏右端对齐列表左边；3 指针移回中部 → 约半秒后滑回；4 弹出时点搜索框输入、移开 → 不收起；Esc 清空搜索后移开 → 收起（**依赖 playlist 把 `focused` 只给搜索框**，见 §3.2，否则列表有焦点时不收起）；5 窗口模式 F6 关列表、进全屏、指针到右边缘 → 不弹出；全屏中 F6 → 立即弹出，再 F6 → 立即收起；6 第 5 条各步底栏列表按钮的亮暗（**依赖 playback G-18**）；7 列表浮动时进全屏 → 浮动窗口不变、右边缘不触发。另看：弹出层里拖放文件进列表；弹出层里右键菜单；收起后按键回到视频（空格暂停）。
- **G-03 第一步**：1 主窗口拖到 320×182 右键 → 顶层菜单高度等于窗口高，上下有 ▲▼ 条，停在条上滚动、滚轮滚动、↓ 键到底自动滚动，每一项都能点到；5 浮动列表窗口（300×250）里右键 → 列表菜单可滚动、每项可点。（第 2–4、6、7 条属于第二步「菜单覆盖层窗口」，第 3 轮。）
- **G-15**：1 右键按下不松开 → 无菜单，松开 → 在松开处出现；右键在视频上按下、拖到底栏 / 窗口外松开 → 不弹出；2 「视频 ▶」停 0.2 秒移开 → 不展开，停 0.5 秒 → 展开；3 展开后从「视频」行斜穿「声音」进入子菜单 → 不切换；4 停在「声音」约 0.4 秒 → 切换；5 点击「视频 ▶」→ 立即展开。（第 6 条 Windows 系统延时，等 Windows 机器。）
- **IA-CORE-01**：打开「视频 ▶ 比例 ▶」，按 Esc 三次 → 依次关「比例」、「视频」、整个菜单；用鼠标展开（键盘光标仍在顶层）时 Esc 同样先关最深一级。
- **G-05**：1 窗口模式在视频上按住左键拖 → 窗口跟随；2 给 `Mouse:LeftClick` 绑播放/暂停后拖动 → 松开后不暂停；3 拖 2px 松开 → 按单击；4 全屏中拖 → 不动；拖完马上单击一次 → 不进全屏。（第 5 条 Windows 贴靠、第 6 条设置页开关本轮不测。）
- **G-10**：默认窗口按 ↑ → OSD 约 18px；1080p 全屏按 ↑ → 约 32px；拖到 320×182 → 约 12px 且不超出视频区；Tab（信息块）→ 紧贴 OSD 行下方，字号同比例。
- **G-13a**：音量 95% 按 ↑ 两次 → 100%、100%；滚轮上推同样停在 100%。（底栏滑块 0–100 是 playback 的 G-13b。）
- **G-16a**：播放视频 A 时打开网络链接 B → B 出画面前视频区不再停在 A 的最后一帧（visualizer 改 G-16b 前显示它的空闲画面或纯色）；画中画中换来源 → 小窗保持。
- **IA-CORE-03**：最大化（双击标题栏或 ⌥ 点绿色按钮）后退出、重启 → 窗口是最大化之前的尺寸与位置。
- **IA-CORE-12**：进入画中画，在主窗口视频区双击 → 小窗关闭、画面回到主窗口、不进全屏、不暂停。
- **IA-CORE-04 第 1 条**：⌥↩ 进入全屏（菜单「全屏」行的快捷键文字仍显示第一个键 `Enter`）。
- **ToolTipArea**：底栏任一按钮悬停 → 提示出现；移开再悬停同一按钮 → 再次出现。
