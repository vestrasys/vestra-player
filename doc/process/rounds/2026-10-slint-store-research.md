# 任务 1 步骤 1.1：Slint 1.16.1 → 1.18.1 调研（维护者）

日期 2026-10-22。基线 `rebuild/slint-118` = main `0349e36`。本文只做调研，没有改任何仓库文件。

**试验环境**（一次性，可删）：
- 试验 worktree `scratchpad/wt-slint-trial`（分支 `trial/slint-118-research`）：两处版本钉改成 `=1.18.1`、`cargo update -p slint -p slint-interpreter --precise 1.18.1`，`ui_markdown.rs` 的两个用 `as_ref()` 的测试用 `#[cfg(any())]` 临时关掉；`src/node_modules` 是指向 wt-core 的软链接。清理：`git worktree remove --force …/wt-slint-trial && git branch -D trial/slint-118-research`。
- 对照探针 `scratchpad/slint118/probe-{116,118}`（同一份 `probe-common/main.rs`，分别链接 1.16.1、1.18.1，软件渲染器无窗口）：渲染对比、ComboBox 复现、条件元素布局复现、逐文件编译诊断。原始输出：`slint118/diag-116.txt`、`diag-118.txt`、`test-118.log`、`subp-*.log`。
- `CARGO_TARGET_DIR=~/.cache/vestra-target/slint118`。上游变更日志取自 `https://raw.githubusercontent.com/slint-ui/slint/v1.18.1/CHANGELOG.md`（crate 包里没有 CHANGELOG），存于 `slint118/CHANGELOG.md`。

**1.18 下的实测结果一览**

| 检查 | 结果 |
| --- | --- |
| `cargo build`（正式代码） | 无错误无警告 |
| `cargo clippy --locked --all-targets -D warnings`（关掉两个 as_ref 测试后） | 通过，无弃用警告 |
| `cargo fmt --check` | 通过 |
| `cargo test --locked` | 289 通过、2 失败（见下）、2 个测试临时编译掉 |
| 失败 1 `ui_probe::a_combo_box_two_way_bound_to_a_global_is_refused` | 预期内：上游已修，探针不再拒绝 |
| 失败 2 `ui::popped_sidebar_narrows_the_fullscreen_bar_from_the_left_edge` | **1.18 行为变化**：`if` 条件变化后，在下一次渲染前读到的布局几何是旧的（§1.3） |
| 54 个 .slint 逐个编译 | 0 个新错误；新增 15 条警告（14 条 `viewport-*` 弃用、1 条 `hide` 遮蔽内置函数），§3 |
| subtitles 插件测试 | 单独跑 3/3 通过；4 路并发跑 12 次失败 3 次，已定位（§5.4），与 Slint 无关 |

---

## 1. 1.16.1 → 1.18.x 与我们相关的变化

### 1.1 破坏性 / 编译层面

| 变化 | 出处 | 对我们 |
| --- | --- | --- |
| `StyledText` 改为公开类型 `slint::StyledText`（`from_plain_text`、`from_markdown`），去掉了 `AsRef<[StyledTextParagraph]>`，段落字段变成 `pub(crate)` | 1.17.0 Rust「Added `StyledText` struct …」；源码 `i-slint-core-1.18.1/styled_text.rs` | `ui_markdown.rs:760,803` 的测试编译失败。正式代码仍可用 `private_unstable_api::re_exports::{parse_markdown, string_to_styled_text}`（1.18.1 仍在，`private_unstable_api.rs:159`）；带占位参数的插值只有这个私有函数能做，公开的 `from_markdown` 不接受参数。解释器的 `Value::StyledText` 仍是 `#[doc(hidden)]`（`slint-interpreter-1.18.1/api.rs:133`）。**结论：私有接口依赖仍在，升级 Slint 时须复测这一处的规则不变** |
| `parse_markdown` 不再 `unwrap()` 解析错误，改为 `debug_log` 后返回部分结果 | 对比 1.16.1 `styled_text.rs:90` 与 1.18.1 `styled_text.rs:148` | `Inline::styled` 的 `catch_unwind` 退回纯文本的分支在 1.18 下不会再触发（无害，可删） |
| `Flickable`/`ScrollView` 的 `viewport-x/y/width/height` 改名 `content-*`，旧名弃用（仍可用） | 1.18.0 Slint language「Renamed the `viewport-x`…」(#1443) | 14 条弃用警告：`@vestra/markdown.slint`（宿主 + SDK 副本）、playlist、audio、video 插件 |
| Window 新增内置函数 `close()`、`hide()`（1.17），组件自己声明同名回调时出警告「shadows the builtin function」 | 1.17.0「`Window`: Added … `close()`, and `hide()` functions」；1.18.0「A component can now declare … shadows a private member」 | `playlist/ui/link.slint:11 callback hide()` 警告；宿主生成的选项窗口源码 `settings_window.rs:144 callback close()` 同样（生成源码，不在扫描里）。行为不变：调用仍落到自己声明的回调 |
| `unstable-wgpu-*`、`unstable-fontique-*` 特性改名 | 1.17/1.18 Rust | 我们不用，无影响。`unstable-winit-030` 仍在（`slint-1.18.1/Cargo.toml:53`、`lib.rs:664`），宿主用法编译通过 |
| `Window::try_dispatch_event` 弃用 | 1.18.0 Rust | 我们只用 `dispatch_event`，无影响 |
| 依赖树：femtovg 0.23.2 → 0.27.0、parley/fontique 0.8 → 0.11.1；femtovg 改用 glow 0.18.0 | `Cargo.lock` 差异（+56/−44 个包） | `unsafe/render` 自己钉 `glow =0.17.0`，与 femtovg 的 0.18.0 并存。两者不共享类型（render 用自己的 `glow::Context::from_loader_function_cstr`），可编译；建议顺手升到 `=0.18.0` 去重（需编译验证）。**femtovg 大版本跨度是 GPU 渲染的主要风险**，只能真机验证 |

### 1.2 修掉了我们绕开的问题（上游证据 + 实测）

| 上游条目 | 我们的绕路 | 实测 |
| --- | --- | --- |
| 1.18.0「Fixed `changed` handlers and animations on properties that are two-way bound to a global property. (#12155, #4529)」；1.18.1「Interpreter: Fixed two-way bindings when several properties link to the same one. (#13446)」 | `ui_probe.rs` 预检、reference.md §736 规则 | 探针：`ComboBox { current-index <=> Bridge.index }` 改 global 后，1.16.1 panic「accessing deleted parent (issue #6426)」，1.18.1 正常返回 `current-index = 2` |
| 字体回退（parley/fontique 0.11；`shaping.rs:71` 按请求字体的行高比统一行高） | 强制 Hiragino Sans GB（`ui.rs:33`）；`font-mono` 不能显示中文（reference §737）；Markdown 代码按 ASCII/非 ASCII 分字体 | 不指定字体时 `Text "中文…"`：1.16.1 墨迹 0 像素（空白，与 2026-10-20 真机现象一致），1.18.1 948 像素；`font-family: "Menlo"` 写中文：1.16.1 为 0，1.18.1 为 948；Markdown `` `中文` ``：1.16.1 为 0，1.18.1 正常 |
| 1.18.0「`StyledText`: Inline code from `@markdown` now renders in a monospace font with a capsule background」（字号 ×0.85，`shaping.rs:18 INLINE_CODE_FONT_SCALE`） | — | **外观变化**：每个 Code 段都有半透明胶囊底。我们现在把代码块逐行、按 ASCII 段套 Code，1.18 下代码块会变成一段段胶囊、字号变小、中文段没有胶囊，需改写（§2） |

### 1.3 行为变化（需要真机关注）

| 变化 | 出处 | 对我们的影响 |
| --- | --- | --- |
| **`if` 条件变化后，布局在下一次遍历/渲染前不更新**。读到的 `width`/`absolute-position` 等是旧值 | 未在变更日志里单独写出；可能与 1.17.0「`for` and `if` elements are now instantiated eagerly」有关。探针 `layout` 复现：已显示的窗口里 `HorizontalLayout { a, if side: Rectangle{317px} }`，把 `side` 设为 false，1.16.1 立即读到 a=1470，1.18.1 仍为 1150，`update_timers_and_animations()` 也不更新，渲染一帧后才变 1470；未显示的窗口在 `show()` 后更新 | 宿主测试 `ui.rs:395` 因此失败。正式代码里宿主在同一轮里先设 `sidebar-visible`/`bottom-visible`（`main.rs:1300-1314`），再读 `video-x/y/width/height`、`logical-*`（`main.rs:1017`、`1216`、`2687`，用于画中画位置、窗口按视频尺寸调整、`sizing.observe`），1.18 下可能读到上一帧的值，下一轮才对。需真机看「显示/隐藏播放列表后立即按视频尺寸调整窗口」「切全屏」等。可考虑向上游报告 |
| 焦点元素变为不可见时清除焦点 | 1.18.0「Fixed the focus not being cleared when the focused item becomes invisible. (#11079)」 | 宿主快捷键靠 `keys := FocusScope` 收冒泡（`host.slint:82-92`）。全屏时底栏 `visible: root.bottom-visible`：若焦点在底栏控件上、底栏隐藏，1.18 会清焦点，之后按键可能到不了绑定表，直到点击画面。侧栏收起时宿主已调用 `focus-video`（`main.rs:1315`），底栏没有 |
| std-widgets 默认字号与 `Text` 一致：fluent 的 `body` 从 `14 * 0.0769rem`（≈1.077rem）变为 1rem | 1.17.1「Widgets: Made the default font size consistent with plain `Text`. (#12237)」；对比 `widgets/fluent/styling.slint` | 插件里 std 的 `LineEdit`、`CheckBox`、`ComboBox`、`Button` 文字约小 7%。Windows/Linux 上默认字号还会读系统设置（1.17.0），Windows 留到 S6 |
| fluent `ComboBox` 最小宽度改为 `max(160px, layout.min-width)`（原来误用 `min-height`） | 1.18.1「Widgets: Fixed the minimum width of `ComboBox`…(#13447)」 | 选项较长时 ComboBox 会更宽，可能撑宽 capture/subtitles/playlist 设置页。160px 下限仍在，hotkeys 自制下拉框（`hotkeys/ui/parts.slint:2`）仍需要 |
| 换行文本的最小宽度 = 最长单词宽度；带 padding 的布局里换行文本按正确宽度测量；Vertical/GridLayout 里换行文本单元格按分到的宽度测高 | 1.18.0 `Text` 条目、(#12776) | 含长 URL/路径的换行文字会抬高窗口最小宽度；对话框高度可能变化 |
| 滚轮滚动有动画；无可平移内容时按下立即转发 | 1.17.0「`Flickable`: Animate wheel scrolling. (#11312)」、1.18.0 (#13117) | 播放列表（`ListView`）的滚轮滚动与 `viewport-y` 回报（拖放落点计算）；动画期间会多次回报。右键菜单的滚动是自己实现的，不受影响 |
| `PopupWindow` 显示后响应几何变化；尺寸变化时偏移修正 | 1.17.0 (#6000)、1.18.0 | ToolTipArea、audio/hotkeys/video 的自制下拉框位置 |
| `float` ↔ `string` 转换按区域小数点 | 1.17.0 (#10857) | 扫描所有 .slint：拼接到字符串的数值都经过 `round()` 或是整数，无影响 |
| `@markdown`/Code 胶囊、`max-lines`、`line-height-factor` | 1.18.0 | 见 §2 Markdown |
| FemtoVG：`take_snapshot()` 返回当前状态而不是上次呈现的帧；分数像素位置图片不再模糊；阴影不再每帧重绘；`Path` 在非 1 缩放下位置修正 | 1.18.0 General | `--snapshot`（`main.rs:4260`）在 GPU 模式下可能多渲染一帧；Path 图标位置可能挪动（播放栏图标、`@vestra/widgets` 图标按钮） |
| macOS：只有背景半透明或无边框的窗口才透明 | 1.18.0 | 宿主没有无边框/透明窗口（已 grep），无影响 |
| 未设 `title` 的窗口以应用名命名 | 1.18.0 | 外观细节 |
| winit：显示前设的窗口宽度不再被首选宽度覆盖 (#13245)；稍后显示的窗口也先渲染一帧 | 1.18.0 | 宿主 `set_size`/`set_position` 后再 `show` 的路径（浮动侧栏、选项窗口、工具窗口），尺寸可能与以前不同（应更符合预期） |
| `KeyEvent.repeat` 对按住的键置位 | 1.18.0「winit: The `repeat` flag…(#12719)」 | 宿主自己从 winit 事件算 repeat，可不改 |
| 修饰键仍只跟随修饰键自身的事件；死键（无文本）仍被丢弃 | 对比 `i-slint-core window.rs`、`i-slint-backend-winit-1.18.1/winitwindowadapter.rs:1385-1395` | 宿主的修饰键补发、死键、⌥+命名键注入仍需要（§2） |

### 1.4 可以替代自有代码的新功能（本轮不必做，供参考）

- 公开 `slint::StyledText`：`ui_markdown` 的类型导入可改用公开类型（构造仍需私有 `parse_markdown`）。
- `Tooltip` 元素（1.17）：可替代 `@vestra/widgets` 的 `ToolTipArea`，但会改变 600ms 延迟等既定行为，不建议本轮动。
- `DragArea`/`DropArea`（1.17）：窗口内拖放（播放列表重排）有了原生插入位置；与 R-U2（宿主无 `onDragOver`）有关，属于以后的插件改进。
- `FlexboxLayout`、`max-lines`、`line-height-factor`、`string.starts-with/ends-with/replace-all`、数组 `push/remove/insert`、结构体字段默认值、`Keys::to_parts()`、`Image::load_from_data()`、`Window` 的 `minimized/maximized`、`WindowMoveArea`。都是新增，不影响现有代码。

---

## 2. 1.16 绕路方案清单与结论

| # | 位置 | 绕开的问题 | 结论 | 依据 |
| --- | --- | --- | --- | --- |
| W1 | `src/host/src/ui_probe.rs`（整个模块、`HAZARD`）；调用点 `ui.rs:67-68`、`settings_window.rs:314-323`、`main.rs:2093-2100`；测试 `ui_probe.rs:139-172`、`settings_window.rs:731-800` | ComboBox `<=>` global 的编译器 bug 导致事件循环 panic | **需要改写**：删掉「独立线程 + 逐个改 global」的预检，但保留注册时单独编译一次（编译失败仍以 `ui-prepare` 拒绝，见 reference §833、core-ui-design §142）。现在探针是选项页注册时唯一的单页编译，直接删会丢掉「坏页面只让自己失败」。测试：把 `a_combo_box_two_way_bound_to_a_global_is_refused` 反过来改成「双向绑定到 global 的 ComboBox 改 global 不崩」（防止退回旧版 Slint）；`options_window_…survives_global_changes` 去掉探针，直接在组合窗口里改 global | §1.2 探针实测；1.18.0 #12155/#4529 |
| W2 | `doc/plugins/reference.md:736`（ComboBox 例外规则） | 同 W1 | **可以拆**：删除该条；`ui-prepare` 只保留「编译失败」的含义 | 同上 |
| W3 | 插件里的单向写法：`playlist/ui/settings.slint:49-50,71-72`（8bc172b）、`audio/ui/workbench.slint:352-357`、`subtitles/ui/tune.slint:32-36` | 同 W1 | **保留**（两版都正确，改回 `<=>` 无收益，属各插件作者，不要求改） | — |
| W4 | `src/host/src/ui.rs:33-44`、`platform::ui_font`（强制 Hiragino Sans GB / Microsoft YaHei 并注入 `default-font-family`，`VESTRA_FONT_FALLBACK`） | 1.16 插件窗口中文空白（不按文字回退） | **可以拆，但要真机 A/B 后由协调者决定**：探针显示 1.18 不指定字体也能回退显示中文。拆掉会改变全部界面的西文字形、行高与宽度（系统 UI 字体 vs Hiragino 的西文），属外观变化；Windows 的 YaHei 回退要到 S6 才能验。建议 1.3 先不改默认，真机用 `VESTRA_FONT_FALLBACK=system` 逐窗口对比；若决定拆，`ui.rs`、`platform/{macos,windows}.rs ui_font`、`theme.slint:40` 注释、architecture README:46、core-ui-design:136 一起改 | 探针 E3（§1.2） |
| W5 | `ui_markdown.rs:26-32` 斜体琥珀色 `EMPHASIS_COLOR` | 强制字体没有斜体，渲染器不合成斜体 | **必须保留** | 1.18.1 的 femtovg（`itemrenderer.rs:874 _synthesis` 未使用）与软件渲染器（`lib.rs:3327 _synthesis`）仍忽略合成；只有 Skia 用。探针：Hiragino 下 `*abc 斜体*` 与正文逐像素相同（1.16/1.18 都是 0 像素差异）。即使拆了 W4，中文也没有斜体字形 |
| W6 | `ui_markdown.rs` 删除线灰色 `STRIKE_COLOR` | 删除线细于 1 像素看不见 | **必须保留** | 探针：1.18 在 11px、12px（`Theme.font-body`/`font-tab`，Markdown 正文字号）删除线 0 像素，14px 才有 1 行；1.16 各字号都是 0 |
| W7 | `ui_markdown.rs:113-152 Inline::code / code_block`、`markdown.slint:120` 注释（代码按 ASCII/非 ASCII 分字体） | 等宽字体在 1.16 不回退，代码里的中文不显示 | **需要改写**：回退已修（W4 的探针 E2/E3b），而且 1.18 的 Code 样式加了胶囊底和 0.85 字号，现在的分段会画成断开的胶囊。建议：行内代码整段用一个 `` `…` ``（中文也在里面，靠回退显示）；代码块不再用 Code 样式，`markdown.slint` 代码块的 `StyledText` 设 `default-font-family: Theme.font-mono`，文字作为纯占位参数（仍逐行硬换行、保留空行与缩进）。改 `markdown.slint` 要走 SDK 同步发版 | 1.18.0 StyledText 条目；`shaping.rs:192-210` |
| W8 | `ui_markdown.rs:169-172` `catch_unwind` 包 `parse_markdown` | 1.16 解析失败 panic | **可以拆**（可选）：1.18 不再 panic；格式串由我们生成且有测试覆盖 | §1.1 |
| W9 | `ui_markdown.rs:757-829` 测试里的 `styled.as_ref()` | — | **需要改写**：段落不再可读。改为断言我们自己的 `Inline.format`/`args`（这才是我们控制的部分，例如格式串里有 `<font color="#e8a33d">*\u{e541}*</font>`），再加 `styled() != StyledText::from_plain_text(plain)` 证明 Slint 接受了格式；像素级验证留给 `markdown_view_renders_the_model` | 编译错误 E0599 |
| W10 | 任务方框图形绘制（`MarkdownBlock.task`，`markdown.slint:92`） | ☐ 不在强制字体里 | **保留**（图形方框在任何字体下一致，属于外观选择；若拆 W4 也不需要改） | — |
| W11 | reference.md:636 Markdown「显示细节」段 | 描述 W5/W6/W7 | **需要改写**：保留斜体/删除线着色说明；代码段落改为新行为 | — |
| W12 | reference.md:737 与 theme.slint 注释「`font-mono`（Menlo）只含西文，中文不要用它」；architecture README:46「信息层改为默认字体」 | Menlo 无回退 | **可以拆**（文档）：1.18 Menlo 写中文可回退显示（探针 E3b）。信息层不必改回 | 探针 |
| W13 | `visualizer/ui/backdrop.slint:56-58,98-103`（文字框高于一行，否则省略的 Text 不绘制；「CJK 回退字体很高」） | 回退字体抬高行盒 | **保留**（无害；1.18 已按请求字体行高统一行盒，`shaping.rs:71`，原因不再成立，但高度富余不影响外观。属 visualizer 作者，可在注释里更新原因） | 源码 |
| W14 | `hotkeys/ui/parts.slint:2-3` 自制下拉框 | fluent ComboBox 最小宽 160px | **必须保留** | 1.18.1 `fluent/combobox.slint:22 min-width: max(160px, layout.min-width)` |
| W15 | `main.rs:3678-3720 observe_key`、`bindings.rs:155-160 Physical.mods`、`main.rs:3818 release_modifiers` | Slint 修饰键只跟随修饰键事件，合成输入时漏按下/释放 | **必须保留** | `i-slint-core window.rs` 修饰键逻辑未变 |
| W16 | `main.rs:1576-1588` 死键 + Option | Slint 丢弃无文本的死键 | **必须保留** | 1.18.1 `winitwindowadapter.rs` 仍 `if text.is_empty() return` |
| W17 | `main.rs:3768-3816 inject_option_key`（⌥⌦、⌥←） | macOS 下 Slint 不把它交给焦点元素，输入卡住 | **保留**；真机可顺带复核（MEMORY 里「⌥⌦ check」待办）。1.18 没有相关条目 | — |
| W18 | `playback/ui/bar.slint:177`「Slint has no loops」（13 步展开） | 语言没有循环 | **保留**（1.18 仍无循环语句） | — |
| W19 | `composition.rs:1`、`ui_values.rs:1` 注释写「1.16.1」；`reference.md:213` 示例 `"1.16.1"`；SDK `src/plugin/testing.ts:1075`、`types.ts:1180`、`test/round3a.test.ts:84,91` | 版本号文字 | **需要改写**（纯文字，改成 1.18.1 或去掉具体版本）。`info` 插件测试里的 `"1.16.1"` 是假数据，不必改 | — |
| W20 | store-api-proposal.md:338-346、511、553，core-ui-design.md:133、136、222、964 | 历史设计记录里写的 1.16 | **保留原文**（是当时的记录），只在 architecture README 与 reference 里更新现状 | — |
| W21 | `settings_window.rs:144` 生成源码 `callback close()`；`playlist/ui/link.slint:11 callback hide()` | （不是绕路）1.17 起与 Window 内置函数同名 | **建议改名**（宿主：`close-window` 一类；playlist：例如 `cancel` 并同步 `link.ts:16`）。不改只是警告，行为不变 | §3 |

---

## 3. 所有 .slint 在 1.18 下的诊断

方法：用宿主同样的编译配置（库路径 `vestra` → `host/ui/vestra`，风格 `fluent-dark`）逐个 `build_from_path`，1.16.1 与 1.18.1 各跑一遍对比。共 54 个文件：宿主 `host/ui` 6、SDK `slint/` 4、`src/plugins/*` 39、`src/examples/*` 4（含 `core-ui/theme.slint`）、插件仓库 2（不含 `dist/`、`node_modules/`）。

**错误**：两版相同，都是单独编译库文件造成的假阳性：`palette.slint`（宿主与 SDK 副本）只导出 global，报「No component found」。实际使用时它被别的文件导入，不是错误。没有任何文件在 1.18 下新出错。

**两版共有的警告**（31 条，都是单独编译非窗口组件/库文件的假阳性：「Exported component 'X' doesn't inherit Window」「No component is exported…」）：与 1.16 完全相同，不处理。

**1.18 新增的警告（15 条，全部需要处理）**：

| 文件 | 行 | 警告 | 归属 |
| --- | --- | --- | --- |
| `src/host/ui/vestra/markdown.slint`（及 SDK `slint/vestra/markdown.slint`，示例 `examples/markdown/ui/document.slint` 经导入也报） | 199、200 | `viewport-width`/`viewport-height` 弃用 → `content-width`/`content-height` | 宿主 + SDK 发版 |
| `src/plugins/playlist/ui/sidebar.slint` | 135、138、145×2、233、234、248、339 | `viewport-y` 弃用 → `content-y` | playlist 作者 |
| `src/plugins/playlist/ui/link.slint` | 11 | `'hide' shadows the builtin function of the same name` | playlist 作者 |
| `src/plugins/audio/ui/workbench.slint` | 230 | `viewport-height` 弃用 | audio 作者 |
| `src/plugins/video/ui/shaders.slint` | 28 | `viewport-height` 弃用 | video 作者 |

插件仓库 `youtube/youtube.slint`、`bilibili/dialog.slint`：1.18 下无错误无警告，不需要改。

宿主内部生成的源码（`composition.rs` 组合窗口、`settings_window.rs` 选项窗口）不在逐文件扫描里，但都被 `cargo test` 编译过；已知会有 `callback close()` 的遮蔽警告（W21）。另外改名 `viewport-*` 时注意：`menu-view.slint` 的 `MenuLevel.viewport-y` 是我们自己的结构体字段，不是 Flickable 属性，没有警告，不用改。

---

## 4. 步骤 1.3 实施计划

### 4.1 顺序与归属

| 步骤 | 内容 | 文件 | 归属 |
| --- | --- | --- | --- |
| 1 | 版本钉 `=1.18.1`（两处）+ `cargo update -p slint -p slint-interpreter --precise 1.18.1`；可选：`unsafe/render` 的 `glow` 升 `=0.18.0` 去重 | `src/host/Cargo.toml`、`src/host/unsafe/render/Cargo.toml`、`src/host/Cargo.lock` | 维护者 |
| 2 | W9 测试改写；W7 代码段改写；W8 去 `catch_unwind`（可选）；类型改用公开 `slint::StyledText`；W5/W6 注释更新原因（1.18 实测） | `src/host/src/ui_markdown.rs` | 维护者 |
| 3 | `markdown.slint`：`viewport-*` → `content-*`；代码块 `default-font-family: Theme.font-mono`、更新注释 | `src/host/ui/vestra/markdown.slint` | 维护者（随后 SDK 同步） |
| 4 | W1：删 `ui_probe.rs`，三处调用改为只编译（选项页注册仍单页编译、失败 `ui-prepare`；面板挂载与工具窗口本来就会编译，可直接删预检）；测试改写 | `ui_probe.rs`（删）、`ui.rs`、`settings_window.rs`、`main.rs` | 维护者 |
| 5 | `ui.rs:395` 测试：断言前 `show()` 并渲染一帧（或改用已显示窗口）；评估正式代码读 `video-*` 的时机（§1.3），至少在注释里写明 1.18 的「下一帧才更新」并在真机确认；可向上游报 issue | `ui.rs`，可能 `main.rs` | 维护者 |
| 6 | W21 宿主侧：生成源码的 `callback close()` 改名 | `settings_window.rs` | 维护者 |
| 7 | W19 注释与文档；W2、W11、W12 文档；reference §213 示例版本；W4 若协调者决定拆则一并改 | `composition.rs`、`ui_values.rs`、`doc/plugins/reference.md`、`doc/architecture/README.md` | 维护者 |
| 8 | 小问题（§5）：操作回执淘汰、portability 目录、钥匙串清理 | `package_store.rs`、`package_manager.rs`、`src/scripts/check-portability.mjs`、`system.rs`/`plugins.rs` 等 | 维护者 |
| 9 | `node scripts/sync-sdk-slint.mjs --write <SDK 仓库>` | — | 维护者 |
| 10 | SDK 1.1.2（§1.6）：同步 `slint/vestra/markdown.slint`；`testing.ts` 默认 `slint: "1.18.1"`、`types.ts` 注释、`test/round3a.test.ts`；`TestHost.settle()` 文档写明「只等宿主调用，插件自己的文件读写要轮询可观察结果」；CHANGELOG；打标签；播放器 `npm install @vestra-player/sdk@1.1.2` | SDK 仓库、`src/package.json`、`package-lock.json` | 维护者写，协调者提交/打标签 |
| 11 | 插件 .slint 适配（步骤 1.4）：playlist（`sidebar.slint` 8 处 `viewport-y`、`link.slint` 的 `hide` 改名 + `link.ts:16`）；audio（`workbench.slint:230`）；video（`ui/shaders.slint:28`）；visualizer（W13 注释，可选） | `src/plugins/<名字>/` | 各插件作者 |
| 12 | subtitles 测试去抖动（§5.4） | `src/plugins/subtitles/tests/` | subtitles 作者 |
| 13 | nes 版本对齐：`package.json`、`package-lock.json`（第 3、9 行）`0.1.0` → `0.2.0` | `src/plugins/nes/` | nes 作者（本轮「顺手」，可由协调者直接改） |
| 14 | 插件仓库 | 不需要改（无诊断；不依赖宿主 Slint 版本）。可选：用新播放器跑一次 youtube/bilibili 真机 | — |
| 15 | §6.1 全部检查 + 真机回归（下节） | — | 协调者 |

步骤 2–8 互不依赖，可一个 agent 顺序做；10 依赖 3；11、12、13 可与 2–8 并行。

### 4.2 风险

1. **GPU 渲染**（femtovg 0.23 → 0.27，glow 双版本）：画面黑屏/错位/闪烁只能真机发现；软件渲染回退路径（`VESTRA_SOFTWARE_RENDER=1`）也要过一遍。
2. **条件元素布局延后一帧**（§1.3）：窗口按视频尺寸调整、画中画定位、全屏侧栏弹出时读到旧几何。测试只改了一处，正式代码要真机看。
3. **焦点清除**（#11079）：全屏底栏自动隐藏后快捷键失效的可能。
4. **外观整体微变**：std-widgets 字号 −7%、ComboBox 变宽、换行文本最小宽度、Path 图标位置、Code 胶囊。
5. **私有接口**：Markdown 仍依赖 `private_unstable_api::parse_markdown` 与隐藏的 `Value::StyledText`，下次升级还要复测。
6. **SDK 发版耦合**：改 `markdown.slint` 就必须发 SDK 1.1.2，否则 `test:sdk` 的逐字节一致检查失败。
7. W4（去掉强制字体）若决定做，是全局外观变化，建议与本次升级分开提交，方便回退。

### 4.3 协调者真机回归清单（侧重 1.18 可能改变的地方）

在 §6.2 全量场景之外，特别看：

| # | 场景 | 看什么 |
| --- | --- | --- |
| 1 | 默认 GPU 渲染播放视频；拖动窗口大小、全屏/退出、画中画开关；再用 `VESTRA_SOFTWARE_RENDER=1` 重复 | 画面正常、无黑帧/撕裂/错位；画中画位置正确 |
| 2 | 窗口模式下显示/隐藏播放列表、拖动分隔条；随后立即执行「窗口按视频尺寸」类命令 | 视频区域和窗口尺寸一次就对（§1.3） |
| 3 | 全屏：鼠标移到右侧弹出侧栏、移开收回；底栏出现/隐藏 | 底栏宽度跟侧栏同步（G-01），动画正常 |
| 4 | 全屏：点底栏上的按钮（焦点进底栏）→ 等底栏自动隐藏 → 按空格/方向键/其它快捷键 | 快捷键仍生效（#11079） |
| 5 | 播放列表搜索框有焦点时收起播放列表 → 按快捷键 | 快捷键生效 |
| 6 | 右键菜单：多级、长菜单滚动（滚轮）、子菜单位置 | 每项可达、位置正确（菜单滚动是自己实现的 `viewport-y` 字段，不受 Flickable 动画影响，属快速复核） |
| 7 | 选项窗口逐页打开：外观、窗口、播放列表、字幕、截图、快捷键、插件管理、标志页；改 ComboBox，插件侧回推值 | 不崩溃；ComboBox 宽度、std 控件字号可接受，布局不溢出 |
| 8 | 悬停提示（播放栏图标按钮）、音频工作台的下拉与预设弹窗、快捷键窗口自制下拉、视频画面的列表弹窗 | 弹出位置与关闭正常 |
| 9 | 播放列表：滚轮滚动、拖动重排（插入位置）、拖到边缘自动滚动、「添加链接…」窗口按 Esc/取消 | 落点正确；窗口能关闭（`hide` 遮蔽） |
| 10 | Markdown 示例窗口 | 斜体琥珀色、删除线灰色可见；行内代码胶囊（含中文的代码整段显示）；代码块等宽、中文可见、缩进与空行保留；任务方框、表格对齐、链接点击、图片 |
| 11 | 所有插件窗口与面板中的中文（默认强制字体）；再用 `VESTRA_FONT_FALLBACK=system` 启动对比几个窗口 | 默认无空白；A/B 结果供 W4 决定 |
| 12 | 键盘：按住方向键连发；死键（⌥E 等）在画面焦点与输入框中；⌥⌦ 在播放列表；系统事件合成的修饰键组合 | 与 1.16 一致 |
| 13 | 含长路径/长 URL 的对话框（打开链接、属性、关于、错误提示） | 窗口最小宽度没有被撑大 |
| 14 | 关于/系统信息页 | 显示「Slint：1.18.1」 |
| 15 | NES 画面、无视频时的标志背景层与封面文字、播放栏图标（Path 位置修正） | 位置与清晰度正常 |

---

## 5. 分到本轮的小问题

### 5.1 操作回执 512 条上限从不清理

**现状**：`package_store.rs:21 MAX_OPERATIONS = 512`；`insert_receipt`（:766-774）满了就拒绝新 id，`validate_catalog`（:256-263）也按 512 校验。内存中的进行中操作（`package_manager.rs:330`，64 个）在终态时会移除，没问题；落盘的回执只增不减。每次内置插件升级播种也写一条（`seed-<sha 前 32 位>`，`package_manager.rs:585`），所以长期使用、多次更新后安装会失败。

**建议**：插入前淘汰最旧的**终态**回执。
- 回执写入时加 `at`（unix 毫秒，可复用 `unix_ms()`）；旧回执没有 `at` 的视为最旧。
- 满 512 时按 `at` 从旧到新删除状态为 `committed`/`failed`/`aborted`/`cancelled`/`failedDegraded`/`completed` 的回执，直到有空位；**不删** `preparing` 以及 `catalog.pending` 里 `recovery.operation_id` 引用的回执。若全是非终态（理论上不可能，pending ≤ 64），仍按现在的错误拒绝。
- 可选再加保留期（例如 30 天）在打开目录时顺带清理。
- 语义影响：被淘汰的 `requestId` 再提交时按新请求处理，`packages.operation` 查询返回 `missing`。在 reference 的 `packages.*` 幂等说明里写明「回执至少保留最近 N 条」。
- 测试：把 `full_receipt_history_rejects_new_requests_and_preserves_existing` 改为「满了淘汰最旧终态、保留 pending」。

### 5.2 `npm run portability` 写进 `src/host/target/portability`

**现状**：`src/scripts/check-portability.mjs:90-91` 固定 `--target-dir join(host, "target", "portability")`，违反 §1「不在仓库里产生 `target/`」。

**建议**：`const targetDir = process.env.CARGO_TARGET_DIR ? join(process.env.CARGO_TARGET_DIR, "portability") : join(tmpdir(), "vestra-target", "portability");`（`node:os` 的 `tmpdir()`，跨平台）。在脚本头注释和 §6.1 写明。`.gitignore` 的 `host/target/` 可以保留防误提交。

### 5.3 带数据卸载后钥匙串条目残留

**现状**：`secrets.*` 的服务名按实例 `vestra.plugin.<guid>`（`plugins.rs:38-44`），`system.rs:135-157` 只按名字读写删；`keyring` 不能按服务枚举，所以 `uninstall(removeData)`（`package_manager.rs:778` 起）只删数据目录，钥匙串留着（store-api-proposal「卸载时…密钥库条目不删」）。

**建议**（跨平台、无需新系统调用）：宿主记录每个实例用过的密钥名。
- 位置：宿主自己的安装库目录（插件不可写），例如 `<安装库>/secrets/<guid>.json`（只存名字，不存值）；或放进 catalog 的实例记录。
- `secrets.set` 成功后加入名字，`secrets.delete` 成功后移除；文件写失败不影响 `set` 的结果但要记日志。
- `uninstall(removeData: true)`、以及休眠实例因 `MAX_DORMANT` 被丢弃时：逐个 `platform::secret_delete(service, name)`（不存在忽略），再删索引。
- 开发加载（`vestra.dev.<id>`）与试载（`<id>.trial.<owner>`）不在卸载范围，不处理。
- 名字索引需要上限，否则插件可以让索引无限增长：建议每实例 256 个名字，超出时 `set` 返回 `limit`。**这是新增的可见限制，需协调者决定**（替代：不设上限，只限制文件大小）。
- 已经残留的旧条目无法枚举清理，在 CHANGELOG 里说明。
- 备选：macOS 用 `security-framework` 按服务枚举（安全 Rust），Windows 需要 `CredEnumerateW`（要进 `unsafe/win32`），两套平台代码，不推荐。

### 5.4 subtitles 插件测试偶发失败

**复现**：`npm run plugin -- test subtitles` 单独跑 3 次全过；4 路并发、共 12 次，失败 3 次（`slint118/subp-1-3.log`、`subp-2-4.log`、`subp-3-1.log`）。两个失败点：
1. `behavior.test.ts:276-291`「a host refusal of the upload is shown as one Chinese sentence」：`window.emit("load-file")` 后只 `await host.settle()` 一次就断言，实际 `status-text` 还是「请选择字幕文件…」（3/3 次失败都有它）。
2. `round2.test.ts:48`「D18: VTT, ASS and two-language SAMI…」：辅助函数 `loadFile`（:26-31）的循环条件 `!host.osd.at(-1)?.startsWith("打开字幕")` 在第二次载入时被上一个文件的「打开字幕：a.vtt」直接满足，于是只 settle 一次就断言（1 次）。

**根因**：SDK 的 `TestHost.settle()`（SDK `src/plugin/testing.ts:1908-1915`）只等宿主调用清空、连续 3 个 1ms 空闲轮；插件自己的文件读取（选字幕后 `readFile`/`stat`/解析）不是宿主调用，负载高时 3ms 内没完成，settle 就提前返回。不是插件逻辑错误。

**修法**（subtitles 作者，只改测试）：
- 用例 1：照搬它上一个用例（:265-267）的轮询：`for (let i = 0; i < 100 && window.props["status-text"] !== text; i++) await host.settle(20);`。
- `loadFile`：先记下 `const before = host.osd.length` 和 `host.sent("subtitles.commit").length`，循环到 OSD 条数增加且最后一条以「打开字幕」开头，或 commit 数增加，再 settle。
- SDK 侧（维护者，随 1.1.2）：`settle()` 的文档写明只等待宿主调用；可选提供 `host.until(predicate, timeoutMs)` 辅助——这是新增 API，按 §4.3 默认不做，文档说明即可。
- 其它 `settle(170)` 等待 100ms 样式计时器的用例在并发下没失败，暂不动。
