# 核心界面与输入能力设计（C1–C11、C14）

日期：2026-10-20。阶段：P1a 设计，P2 审查已通过（裁决见[本轮任务](../process/rounds/2026-10-core-ui.md) §8「P2 审查」，已并入本文），P1b 按本文实现。
依据：[本轮任务](../process/rounds/2026-10-core-ui.md) §2、§5，[界面整改文档](../../product/ui-alignment.md) §2、§3、§6、§8、§9、§10。
现状代码：`src/host/src/{main,commands,menu,composition,window_state,ui,plugins}.rs`、`src/host/ui/*.slint`、`src/sdk/plugin/*.ts`、`src/runtime/runner.ts`。

读法：每项先给公共接口，再给宿主怎么做、生命周期与错误、用法示例、取代并在 P4 删除的旧东西、现有插件的迁移写法。§14 说明迁移后菜单树为何不变，§15 是迁移者照抄的改写对照，§16 是删除总清单，§18 是需要协调者决定的问题。

## 0. 总则

- **新增公共面一览**（只列插件能看到的）：

| 位置 | 新增 / 改变 | 对应 |
| --- | --- | --- |
| `CommandOptions` | 新增 `order`、`group`、`radio`、`bindings`、`focus`、`repeat`；删除 `shortcut`；`path` 改为 1–8 段任意内容，可省略（不进主菜单） | C3、C4 |
| `plugin.panels` | 新增 `panels.bottom`、`panels.sidebar`（位置状态，任何插件可读可切换）；`mount` 选项加 `title`；`Panel` 删除 `visibility`/`onVisibility`/`setVisible`/`toggle`，新增 `showMenu` | C7、C11、C14 |
| `plugin.player` | 新增 `provideSequence`、`next`、`previous`、`stats`；`PlayerState.can` 加 `next`、`previous` | C6、C10 |
| `plugin.osd`（新模块） | `show(text)`、`panel()` | C5、C10 |
| manifest 能力 | 新增 `ui.osd`（`plugin.osd.*`）。其它新接口沿用现有能力 | C5、C10 |
| Slint | `import { Theme } from "@vestra/theme.slint"`、`import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint"`；所有插件 Slint 默认用 `fluent-dark` 风格编译 | C2 |

- C1、C8、C9 没有插件接口，只是宿主行为。
- wire 方法只服务于 SDK（[架构总览](README.md) §3）。新增：`layout.state`、`layout.set`、`ui.menu`、`osd.show`、`osd.panel`、`playback.stats`、`sequence.set`、`sequence.step`；宿主推给插件的新事件：`sequence.step`（只发给序列提供者，不需订阅）。删除：`ui.mountState`、`ui.showMount`。改变：`commands.register` 的 `declaration`、`ui.mount` 的参数、`source.state`（数据加 `sequence` 字段）。
- 迁移期间（P1b–P3，已于 P4 结束，见 §19）旧字段与新字段**并存**：宿主同时接受 `declaration.shortcut` 和 `bindings`（`shortcut` 等于多一条绑定），SDK 同时保留 `Panel` 的旧可见性方法（`ui.showMount`/`ui.mountState` 仍可用）。SDK 对**没用任何新字段**的命令仍发旧形状 `{ title, path, shortcut }`，未迁移插件的测试不受影响；用了任一新字段（迁移后的插件都写 `order`）就发完整形状。P4 一次删除（§16），届时 SDK 总发完整形状。

## 1. C1 主窗口外壳

### 1.1 宿主行为（无插件接口）

`host.slint` 改成下面的结构（数值来自整改文档 §3）：

```text
HostWindow (默认 780×671，最小 320×182，背景 Theme.bg-video #101010，默认字体见 C2)
└─ keys := FocusScope              // 全局绑定入口（C3），包住全部内容
   └─ HorizontalLayout { spacing: 0 }
      ├─ VerticalLayout（视频列，horizontal-stretch: 1）
      │  ├─ canvas := FocusScope + 视频 Image + 鼠标 TouchArea（C3） + 屏幕提示层（C5、C10）
      │  └─ 底栏槽（普通模式在视频下方；全屏时覆盖在视频底部）
      ├─ 分隔条 3px #000（侧栏附着且显示时）
      └─ 侧栏槽（全高，宽度 = sidebar-width）
   └─ 菜单层（现有 MenuView，样式改为 C4）
```

- 去掉 Commands 按钮行、说明文字、诊断行、8px 外边距（UI-SHELL-01–03）。视频贴边。
- 侧栏占视频右侧全高，底栏只在视频列下方（UI-SHELL-05）。
- **分隔条**（UI-SHELL-06/07）：拖动时由 Slint 内部改 `sidebar-width`，范围 `[317, 可用宽 − 3 − 318]`；松开时调用 `sidebar-resized(width)`，宿主记下宽度并持久化（C8）。默认 317。
- **空间不足**（UI-SHELL-08）：宿主 `window_state::Policy` 的规则改为：
  - 侧栏宽 = `min(用户宽度, 内容宽 − 3 − 318)`，且不小于 317；
  - 只有 `内容宽 < 318 + 3 + 317 = 638` 时才隐藏，`reason: "space"`，`preferred` 不变（菜单勾选仍是「显示」）；
  - 不再有高度条件（侧栏全高），删除 640×360、240、280 这组常量。
- **全屏**（UI-SHELL-09）：底栏覆盖在视频底部，3000 ms 无操作后隐藏（沿用现有 `Policy` 的 idle 逻辑）；同时若正在播放（`!paused`），视频区 `mouse-cursor: none`。侧栏在全屏时照旧隐藏（`reason: "fullscreen"`）。
- 显示侧栏时窗口太窄就自动放大窗口的逻辑（`sidebar_resize_requested`）删除，由上面的压缩规则代替（见 §18 Q7）。
- 窗口初始尺寸与位置从持久化恢复（C8），没有记录时用 780×671、系统默认位置。

### 1.2 取代与删除（P4）

`host.slint` 的 Commands 按钮行、说明文字、`diagnostic` 属性与 `Text`、`padding: 8px`、1100×720 / 720×480 尺寸、窗口标题常量；`main.rs` 中写 `diagnostic` 的两处（帧刷新、「Preparing plugin UI…」）和 `sidebar_resize*`；`window_state.rs` 的 `CANVAS_MIN_*`、`SIDEBAR_MIN_WIDTH = 240`、`SIDEBAR_TARGET_WIDTH = 280` 和「空间不足直接隐藏」的判断。

迁移：插件无需改动。

## 2. C2 主题

### 2.1 Slint 接口

```slint
import { Theme } from "@vestra/theme.slint";      // 令牌，全部是常量 out 属性
import { ToolTipArea } from "@vestra/widgets.slint"; // 悬停提示（std-widgets 没有）
```

`@vestra/theme.slint`（宿主随附，路径 `src/host/ui/vestra/theme.slint`）：

```slint
export global Theme {
    // §2.1 颜色
    out property <color> bg-bar: #1e1e1e;      out property <color> bg-video: #101010;
    out property <color> bg-list: #3b3b3b;     out property <color> bg-panel: #333333;
    out property <color> line: #000000;        out property <color> hover-bg: #272727;
    out property <color> press-bg: #181818;    out property <color> text: #f0f0f0;
    out property <color> text-dim: #777777;    out property <color> text-list: #aeaeae;
    out property <color> text-played: #888888; out property <color> accent: #fae100;
    out property <color> accent-press: #c8b400; out property <color> accent-disabled: #7d7000;
    out property <color> icon-disabled: #3b3b3b; out property <color> playing: #2282f2;
    out property <color> button-bg: #555555;   out property <color> button-hover: #5c5c5c;
    out property <color> button-press: #444444; out property <color> button-disabled: #2a2a2a;
    // §2.2 字体与字号（只用这几档）
    out property <string> font-mono: "Menlo";  // 默认字体不在这里：宿主统一设置（§2.2）
    out property <length> font-xs: 9px;  out property <length> font-body: 11px;
    out property <length> font-tab: 12px; out property <length> font-title: 13px;
    // §2.3 控件尺寸
    out property <length> button-height: 21px;  out property <length> button-min-width: 66px;
    out property <length> button-padding: 8px;  out property <length> button-primary-height: 24px;
    out property <length> button-small-height: 18px; out property <length> button-small-min-width: 41px;
    out property <length> input-height: 21px;   out property <length> checkbox-row-height: 18px;
    out property <length> label-height: 14px;   out property <length> form-row-gap: 26px;
    out property <length> dialog-padding: 14px;
}
```

`@vestra/widgets.slint` 本轮提供三个组件（P2 裁决 Q13：第二步所有工具窗口与底栏都要用；其它控件不做，用 `fluent-dark` 的 std-widgets）：

```slint
export component ToolTipArea {           // 包住任意内容；悬停 600 ms 后在指针旁显示 text
    in property <string> text;
    @children
}
export component Button {                // PotPlayer 对话框按钮
    in property <string> text;
    in property <image> icon;            // 有图标且 text 为空 = 图标按钮
    in property <bool> enabled: true;
    in property <bool> small;            // false：21px 高、最小 66px 宽；true：18px 高、最小 41px 宽
    in property <bool> checked;          // 图标按钮的开启态（图标变 Theme.accent）
    callback clicked();
}
export component Slider {                // 2px 线滑块
    in-out property <float> value;
    in property <float> minimum: 0;
    in property <float> maximum: 1;
    in property <bool> enabled: true;
    in property <bool> panel;            // false：控制栏样式，8px 圆点 thumb，填充 Theme.accent；
                                         // true：面板样式，14×12 thumb，填充 Theme.playing，轨道 Theme.bg-bar
    callback changed(float);             // 拖动中连续回调
    callback released(float);            // 松开
}
```

样式：ToolTipArea 底 `Theme.bg-panel`、1px `Theme.line` 边框、`Theme.font-body`、`Theme.text`，用 `PopupWindow` 显示，指针移出或按下即消失。Button 文字按钮底 `Theme.button-bg`（悬停 `button-hover`、按下 `button-press`、禁用 `button-disabled`），文字 `Theme.text-list` 11px；图标按钮无底色，图标 `colorize` 常态 `Theme.text-dim`、悬停与开启 `Theme.accent`、禁用 `Theme.icon-disabled`。Slider 的 thumb 常态 `Theme.text`，控制栏样式悬停 `Theme.accent`、按下 `Theme.accent-press`；未填充部分控制栏样式为 `Theme.line`。

### 2.2 宿主怎么做

- **导入机制**：slint-interpreter 1.16 的 `Compiler::set_library_paths` 支持 `@库名/文件` 形式的导入（`i-slint-compiler` `typeloader.rs` 的 `find_file_in_library_path`）。宿主把库名 `vestra` 映射到随附目录（开发时 `src/host/ui/vestra/`，打包后 `Resources/ui/vestra/`，与现在找 `host.slint` 的方式相同）。不需要虚拟文件或改写插件源码。
- 宿主现在有三处编译 Slint：`ui.rs`（工具窗口）、`composition.rs`（主窗口与面板）、`package_manager.rs`（包校验）。合并成一个 `slint_compiler()`：设库路径，`set_style("fluent-dark")`。因此**不改代码的插件，std-widgets 控件自动变成深色**（Button、Slider、LineEdit、CheckBox、ComboBox、ScrollView），这是预期变化。已核对：现有浅色工具窗口（video、subtitles、capture、nes、youtube、bilibili、workshop、playlist/link、bookmark）的 `.slint` 都没有写死文字颜色，深色下不会出现看不清的文字；audio、playback、playlist 侧栏本来就是自定深色。
- `Theme` 是只读常量，同一文件被多个插件导入、在主窗口里共享不会互相影响（`composition.rs` 注释里禁止的是**有状态**的共享 global）。
- **默认字体**：删除强制的 `Arial Unicode MS` 与按文件导入它的代码。P1b 查明 Slint 1.16 用 parley 排版、按字符经 fontique 回退（macOS 上走 CoreText 的系统回退链，`i-slint-core` `textlayout/sharedparley.rs`），所以默认**不指定字体**：系统 UI 字体 + 系统 CJK 回退。备用方案已实现但默认关闭：环境变量 `VESTRA_FONT_FALLBACK=hiragino` 时导入系统自带的 `/System/Library/Fonts/Hiragino Sans GB.ttc` 并设为主窗口、浮动窗口、工具窗口的默认字体（工具窗口沿用「插件没写 `default-font-family` 才注入」）。**P5 实测（2026-10-20）：不指定字体时插件窗口与面板的中文全部显示为空白，因此改为默认启用 Hiragino**（`VESTRA_FONT_FALLBACK=system` 可关闭）；信息层原用 `font-mono`（Menlo）同样缺中文，改为默认字体（§18 Q12）。
- 宿主自己的界面（`host.slint`、`menu-view.slint`）也改用 `Theme` 令牌；菜单专用颜色（§6.3）留在 `menu-view.slint` 内，不进 `Theme`。
- 编辑器支持：在 `doc/plugins/tutorial.md` 说明 Slint 语言服务的 `libraryPaths` 设置（`"vestra": "<仓库>/src/host/ui/vestra"`）。

### 2.3 错误

导入不存在的令牌或组件是 Slint 编译错误，沿用现有报告：工具窗口 `show()` 失败、面板 `ready` 失败（`ui-prepare`）、包校验失败。

### 2.4 示例

```slint
import { Button } from "std-widgets.slint";
import { Theme } from "@vestra/theme.slint";
import { ToolTipArea } from "@vestra/widgets.slint";

export component Picture inherits Window {
    width: 301px; background: Theme.bg-panel;
    in property <string> path;
    callback reset();
    VerticalLayout {
        padding: Theme.dialog-padding; spacing: 8px;
        ToolTipArea { text: root.path;
            Text { text: "亮度"; color: Theme.text; font-size: Theme.font-body; } }
        Button { text: "默认"; height: Theme.button-small-height; clicked => { root.reset(); } }
    }
}
```

### 2.5 取代与删除、迁移

- 删除：`ui.rs` 与 `composition.rs` 中的 `Arial Unicode.ttf` 导入和 `"Arial Unicode MS"`；`host.slint` 的 `default-font-family: "Arial Unicode MS"`；架构总览 §4「插件界面默认使用宿主内置的 CJK 字体」一句改写。
- 迁移：本轮插件**不需要**改 `.slint`（行为不变，深色是宿主的预期变化）。改用 `Theme` 令牌、删除硬编码颜色是第二步（UI-THEME-01 的验收在第二步）。

## 3. C3 输入绑定表

### 3.1 SDK 接口

```ts
interface CommandOptions {
  // …其余字段见 C4
  /** 默认绑定，例如 ["Space"]、["Ctrl+O", "F3"]、["Mouse:WheelUp"]。 */
  bindings?: string[];
  /** "panel"：这些绑定只在本插件的面板（附着或浮动）获得焦点时生效。默认全局。 */
  focus?: "panel";
  /** true：按住不放时随系统自动重复触发（方向键跳转、音量）。默认 false。 */
  repeat?: boolean;
}
```

`Command` 句柄不变。用户覆盖层没有 SDK 接口（改绑界面不做）。

### 3.2 绑定语法

一条绑定 = 若干修饰键 + 一个输入，用 `+` 连接，大小写敏感、写法固定：

```text
binding   = { modifier "+" } input
modifier  = "Ctrl" | "Alt" | "Shift" | "Cmd"          （书写顺序固定为 Ctrl、Alt、Shift、Cmd）
input     = key | "Mouse:" mouse
key       = "A".."Z" | "0".."9" | "F1".."F24"
          | "Space" | "Enter" | "Tab" | "Escape" | "Backspace" | "Delete" | "Insert"
          | "Home" | "End" | "PageUp" | "PageDown"
          | "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
          | "Plus" | 其它单个可打印符号（> < / , . ; ' [ ] \ - = ` 等）
mouse     = "LeftDoubleClick" [ ":LeftHalf" | ":RightHalf" ]
          | "MiddleClick" | "X1" | "X2"
          | "WheelUp" | "WheelDown" | "WheelLeft" | "WheelRight"
```

规则：

- **字母**按键名匹配，与大小写锁无关：`E` = 按 E；`Shift+E` = Shift 加 E。
- **符号键**按实际打出的字符匹配，不能再写 `Shift`：PotPlayer 的字幕同步 `>` 写成 `">"`，不是 `"Shift+."`。`+` 写成 `Plus`。数字同理：`"Shift+1"` 非法，要写 `"!"`。
- **macOS 的 Ctrl 与 Cmd**：`Ctrl+X` 在 macOS 上按 ⌃X 或 ⌘X 都触发（整改文档 §0）；`Cmd+X` 只认 ⌘X。同一事件先找完全匹配（⌘X 先找 `Cmd+X`），找不到再按 `Ctrl+X` 找。其它平台 `Cmd` 是 Win/Super 键。（Slint 在 macOS 上把 ⌘ 报成 `control`、⌃ 报成 `meta`，宿主在 Rust 里换回来。）
- **鼠标**只在视频区生效，可带修饰键。`LeftDoubleClick:LeftHalf`/`:RightHalf` 按双击位置在视频左/右半边区分；同时有带区与不带区的绑定时，带区的优先，不算冲突。滚轮按累计位移计步（阈值在 P1b 按触控板和鼠标实测确定），每步触发一次。`X1`/`X2` 是鼠标侧键（Slint `back`/`forward`）。
- 右键保留给菜单，不能绑定。左键单击、长按、右键双击本轮不做（§18 Q3）。宿主默认把 `Mouse:LeftDoubleClick` 绑到全屏。
- 默认绑定**只在按下时触发，忽略按住不放的自动重复**（与现在 Space/←/→ 相同）；命令声明 `repeat: true` 时，按键的系统自动重复也触发（§18 Q6）。鼠标事件没有重复。
- 菜单打开或系统对话框打开时，所有绑定暂停（与现在相同）。
- 被绑定的 `Tab` 会被宿主接住，不再用于主窗口内的焦点切换。

格式不合法（未知键名、修饰键顺序错、符号键带 Shift、重复）时，`commands.add` 立即抛出 `code: "invalid"` 的错误（SDK 先校验，宿主注册时再校验一次，以宿主为准）。

### 3.3 作用范围与焦点

| 范围 | 何时生效 | 实现 |
| --- | --- | --- |
| 全局（默认） | 主窗口或浮动面板窗口获得焦点，且按键没被焦点控件吃掉（输入框打字、列表自己处理的方向键等） | `host.slint` 最外层 `keys := FocusScope` 收到冒泡上来的按键。Slint 1.16 的按键先给焦点控件，未接受的再逐级交给上层 FocusScope（`i-slint-core` `window.rs` `process_key_input`），所以输入框里的空格、方向键不会触发绑定 |
| `focus: "panel"` | 焦点在该插件面板之内时，**先于**全局查找 | 宿主在面板外包一层 FocusScope（`focus-on-click` 默认为真，点面板空白处即获得焦点），未被面板内部接受的按键先在这里按「面板 P」范围查找，找不到再冒泡到全局 |

- 工具窗口（插件 `windows.create`）里不触发任何绑定，与现在相同；工具窗口范围的绑定本轮不做（§18 Q5）。
- 与现在相比，全局绑定在焦点位于侧栏、底栏空白处时也生效（现在只在视频获得焦点时）。快捷键原有的生效场景全部保留。

### 3.4 宿主数据结构

新文件 `src/host/src/bindings.rs`（纯数据，无 UI）：

```rust
pub struct Mods { ctrl: bool, alt: bool, shift: bool, cmd: bool }
pub enum Input { Key { key: Key, mods: Mods }, Mouse { event: MouseEvent, zone: Option<Zone>, mods: Mods } }
pub enum Scope { Global, Panel(OwnerGeneration) }
pub struct Claim { owner: OwnerGeneration /* 0 = 宿主 */, command: String, user: bool }
pub struct Table {
    defaults: BTreeMap<(OwnerGeneration, String), (Vec<Input>, Scope)>, // 来自命令声明
    overlay: Overlay,               // 用户覆盖层："<manifest id>/<命令 id>" → 绑定列表（[] = 解绑）
    active: BTreeMap<(Input, Scope), Claim>,   // 由上面两项重算
    conflicts: Vec<Conflict>,                  // 被压下的声明
}
impl Table {
    fn parse(text: &str) -> Result<Input, String>;
    fn resolve(&self, input: &Input, focused_panel: Option<OwnerGeneration>) -> Option<&Claim>;
    fn label(&self, owner: OwnerGeneration, command: &str) -> Option<String>; // 菜单里显示的文字
}
```

- 表是**派生数据**：命令注册、移除（插件停用）、覆盖层载入时整表重算，不保存中间状态。
- 宿主自己的命令（owner 0）也在表里：

| 命令 | 默认绑定 | 菜单 | 状态 |
| --- | --- | --- | --- |
| `host/fullscreen` 全屏 | `Enter`、`F11`、`Mouse:LeftDoubleClick` | 顶层，`group: "screen"`，`order: 700`，勾选 = 是否全屏 | 总是可用 |
| `host/exit-fullscreen` 退出全屏 | `Escape` | 不在菜单 | 仅全屏时可用 |
| `host/quit` 退出 | `Cmd+Q` | 顶层，`group: "exit"`，`order: 990` | 总是可用 |

  菜单中是否本轮就出现「全屏」「退出」见 §18 Q2。
- **主窗口事件流**：Slint 回调 `key-input(scope, text, ctrl, alt, shift, meta, repeat) -> bool` 和 `mouse-input(kind, zone, ctrl, alt, shift, meta) -> bool` 在 UI 线程上**同步**查询表的只读副本（`Rc<RefCell<Table>>`，重算时整体替换），绑定了就返回真（`accept`），并发 `Event::Binding(owner, id)`；主循环里再按现有 `resolve_execution` 检查启用/忙碌，再发 `commands.execute`。绑定了但命令禁用：按键被接住、不执行（与现在 Space 的行为相同）。
- **模拟器输入捕获**（`producer.declareInput`，SDK 未公开）：winit 钩子里写死的 `F11 | Escape` 放行改为「绑定在宿主命令上的键放行」，其余被捕获的键仍优先给生成端。

### 3.5 冲突规则

- 冲突 = 同一 `(输入, 范围)` 有两个以上命令声明。不让注册失败（一个插件选了某键不应让另一个插件激活失败），而是**确定性地只保留一个**，其余记入 `conflicts`：
  1. 用户覆盖层的绑定优先于任何默认绑定（被覆盖的默认绑定不算冲突，只是被取代）；
  2. 宿主自己的命令优先于插件；
  3. 插件之间，先加载的（owner 代次小）优先。代次在 `--plugin` 顺序 / 包存储顺序下按序分配，同一组插件每次启动结果相同；新安装或试载的插件代次更大，不会抢走已有插件的键。
- 报告：每次重算后，新出现的冲突写一行宿主日志（`binding conflict: Space → vestra.playback/toggle-pause; ignored: x.y/z`）；测试宿主提供 `host.conflicts` 供插件测试断言。没有给插件的冲突查询接口（无使用者）。备选规则见 §18 Q4。
- 赢家停用后，表重算，原先被压下的绑定自动生效。
- 不同范围不冲突：面板范围的 `Enter` 与宿主全局 `Enter` 共存，面板获得焦点时面板优先。
- 试载中的插件：绑定一律忽略（与现在清空 `shortcut` 相同）。

### 3.6 持久化

用户覆盖层存在 C8 的状态文件 `bindings` 字段，键是 `"<manifest id>/<命令 id>"`（宿主命令为 `"host/<id>"`），值是绑定字符串数组，`[]` 表示解绑。本轮只读不写（没有改绑界面）；保存其它状态时原样保留。非法条目忽略并写日志。

### 3.7 示例

```ts
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  plugin.commands.add({
    id: "toggle", path: ["播放", "播放/暂停"], order: 263,
    bindings: ["Space"],
    run: () => player.setPaused(!player.state.paused),
  });
  plugin.commands.add({
    id: "volume-up", path: ["声音", "音量 +"],
    bindings: ["ArrowUp", "Mouse:WheelUp"],
    run: () => player.setOutput({ volume: Math.min(200, player.state.volume + 5) }),
  });
  plugin.commands.add({
    id: "remove", path: ["播放列表", "移除所选"], focus: "panel",
    bindings: ["Delete"],     // 只在本插件面板获得焦点时
    run: () => removeSelected(),
  });
}
```

### 3.8 取代与删除、迁移

- 删除：`CommandOptions.shortcut`、`CommandDeclaration.shortcut`（SDK 两处）、`commands.rs` 的 `Declaration.shortcut`、`canvas_shortcut()`、`resolve_shortcut()`、「main-canvas shortcut already registered」冲突；`host.slint` canvas `key-pressed` 里写死的 F11、Escape、Space、←、→ 与 `window-input("shortcut:…")`；`main.rs` 的 `Host::canvas_shortcut`、`WindowInput` 中 `shortcut:` / `toggleFullscreen` / `escape` 分支；试载时清空 `shortcut` 的代码；producer 钩子里的 `"F11" | "Escape"`。
- 迁移（行为不变）：`shortcut: "Space"` → `bindings: ["Space"]`；`"ArrowLeft"`、`"ArrowRight"` 同名。现有插件只有 playback 的这三处。

## 4. C4 菜单结构

### 4.1 SDK 接口

```ts
interface CommandOptions {
  id: string;
  /** 菜单路径，1–8 段，最后一段是菜单项。1 段 = 顶层直接命令。省略 = 不进主菜单。 */
  path?: string[];
  title?: string;               // 默认取 path 最后一段；省略 path 时必填
  /** 同级排序，小的在前；默认 0。整数，范围 ±1 000 000。 */
  order?: number;
  /** 同级分组名；相邻两项分组不同时，中间画分隔线。默认 "domain"。 */
  group?: string;
  /** checked 画成单选圆点而不是 ✓。互斥由插件自己维护 checked。 */
  radio?: boolean;
  bindings?: string[]; focus?: "panel";   // C3
  enabled?; checked?; busy?; reason?; run();  // 不变
}
```

### 4.2 排序与分组规则

1. 每一级菜单里，每个节点有一个键 `(order, group)`：
   - 菜单项：取自己的 `order`、`group`；
   - 子菜单（某个路径段）：取它下面**排序最前的那个命令**的 `order` 和 `group`（递归）。子菜单不需要单独声明。
2. 同级按 `order` 升序；`order` 相同时，先按插件 manifest id，再按同一插件内的注册先后。结果与注册到达的时间无关（现有承诺不变）。**不再按标题字母排序。**
3. `group` 只决定分隔线：排好序后，相邻两项 `group` 不同就画一条线。`group` 不参与排序，所以一个分组要连在一起，它的 `order` 也要连续。
4. 路径规则：1–8 段，每段 1–80 字节、无控制字符和首尾空白；同一路径不能被注册两次，也不能既是菜单项又是子菜单（`conflict`，与现在相同）。顶层不再限定领域名。省略 `path` 的命令（P2 裁决 Q9）不进主菜单，`title` 必填，照常参与绑定表并可放进面板右键菜单（C11）；第二步播放列表的列表菜单（全选、排序）与列表内 Del/Enter 用它。
5. 推荐骨架（宿主文档约定，宿主不强制）：

| 分组 `group` | 顶层 `order` 段 | 内容 |
| --- | --- | --- |
| `open` | 100–199 | 打开文件、打开 ▶、关闭 |
| `domain` | 200–699 | 播放、字幕、视频、声音、模拟器等领域子菜单 |
| `screen` | 700–799 | 全屏（宿主 700） |
| `windows` | 800–899 | 播放列表、播放信息、插件工坊等窗口入口 |
| `exit` | 900–999 | 退出（宿主 990） |

   顶层编号表放在 [doc/product/menus.md](../../product/menus.md)（交互负责人维护）；子菜单内部的编号由所在子菜单的插件自定。
6. 试载插件：菜单项**最后一段**加前缀 `试载 <owner> · `（现在是第 2 段；现有命令都是 2 段时结果相同），绑定忽略。

### 4.3 宿主数据结构

`commands.rs`：

```rust
pub struct Declaration {
    pub title: String,
    pub path: Vec<String>,          // 0..=8；空 = 不进主菜单
    pub order: i32,                 // 默认 0
    pub group: String,              // 默认 "domain"
    pub radio: bool,
    pub bindings: Vec<String>,      // 原文；解析与生效在 bindings.rs
    pub focus: Option<Focus>,       // Some(Panel)
    pub repeat: bool,
}
pub struct Registry { commands: BTreeMap<CommandKey, CommandSnapshot>, sequence: BTreeMap<CommandKey, u64>, revision: u64 }
```

- `Registry::new` 不再接收领域列表；`snapshot()` 按 §4.2 建树。`MenuNode::Group` 增加 `order`、`group`（取自最前子项），`MenuNode::Command` 不变；菜单投影（`menu.rs`）在分组变化处插入分隔行。
- `snapshot()` 输出给菜单投影，`bindings::Table::label()` 提供每行右侧的快捷键文字。

### 4.4 菜单样式（宿主，§6.3）

`menu-view.slint`：底 `#444444`、1px `#000000` 边框、无圆角；文字 `#bbbbbb` 12px；悬停底 `#181818` 文字 `#fae100`；禁用 `#777777`；分隔线 1px `#27282f`（行高 7px，`menu.rs` 的几何计算同步）；勾选 14×14 圆角 3 底 `#34353f` 白勾；单选 12×12 外环 `#34353f` 中心点 `#2282f2`；快捷键文字右对齐 `#777777`。项高 24px。

快捷键文字取该命令第一个生效的**键盘**绑定（鼠标绑定不显示），显示形式：修饰键 `Ctrl+`、`Alt+`、`Shift+`、`Cmd+`；`ArrowLeft/Right/Up/Down` → `← → ↑ ↓`；`Escape` → `Esc`；`PageUp/PageDown` → `PgUp/PgDn`；`Delete` → `Del`；其余原样（§18 Q11）。

### 4.5 错误

| 情况 | 结果 |
| --- | --- |
| 路径段数、文字、`order` 不合法，`group` 不合法（规则同路径段） | `commands.add` 的注册以 `invalid` 失败（激活期间 = 激活失败） |
| 路径重复、叶子与子菜单冲突 | `conflict`（不变） |
| 每插件超过 64 条、总数超过 512 条 | `limit`（不变） |

### 4.6 示例

```ts
// 顶层直接命令、分组、单选组
plugin.commands.add({ id: "open", path: ["打开文件…"], group: "open", order: 100,
  bindings: ["Ctrl+O", "F3"], run: () => openFile() });
for (const [i, rate] of [0.5, 1, 1.5, 2].entries())
  plugin.commands.add({
    id: `rate-${rate}`, path: ["播放", "速度", `${rate}倍`], order: 230 + i, radio: true,
    checked: () => player.state.rate === rate,
    run: () => player.setRate(rate),
  });
plugin.commands.add({ id: "list", path: ["播放列表"], group: "windows", order: 800,
  checked: () => plugin.panels.sidebar.state?.preferred ?? false,
  bindings: ["F6"], run: () => plugin.panels.sidebar.toggle() });
```

### 4.7 取代与删除、迁移

- 删除：`Registry::new(allowed_domains)` 与 `domains`/`domain_order`、`main.rs` 里的 8 个领域名、「2..=6 segments」与「domain is not allowed」检查、`snapshot()` 的领域排名 + `path[1..]` 字母排序、试载改写 `path[1]` 的代码、参考文档 §4「顶层分组惯例」一句。
- 迁移：每条命令加 `order`，值见 §14 的表；`path`、`title` 不变；不写 `group`（全部为默认 `"domain"`，没有分隔线，与现在相同）。

## 5. C5 屏幕提示

### 5.1 接口

```ts
interface Osd {
  /** 在视频左上角显示一行提示，2 秒后淡出；新提示立即替换旧提示。 */
  show(text: string): Promise<void>;
  panel(): OsdPanel;   // C10
}
interface Scope { readonly osd: Osd }
```

需要新能力 `ui.osd`。

### 5.2 宿主行为

- 视频区左上角，左/上边距 10px，粗体 16px，白字、1px 黑色描边（Slint `Text` 的 `stroke`/`stroke-width`）；显示 2000 ms 后在 300 ms 内淡出；任何插件的新提示替换当前提示（只有一行）。全屏与普通模式都显示。
- 文字 1–200 字符，换行替换为空格。
- 提示不属于任何资源：插件停用时，已显示的提示照常到时消失。
- wire：`osd.show { text }`，宿主显示后答复。

### 5.3 错误

`invalid`（空或过长）、`unavailable`（无界面模式）、`capability`。

### 5.4 示例

```ts
plugin.commands.add({
  id: "volume-up", path: ["声音", "音量 +"], bindings: ["ArrowUp"],
  run: async () => {
    const volume = Math.min(200, plugin.player.state.volume + 5);
    await plugin.player.setOutput({ volume });
    await plugin.osd.show(`音量: ${volume}%`);
  },
});
```

### 5.5 取代与删除、迁移

无旧接口。本轮插件不使用（屏幕提示文案是第二步 UI-OSD-02）。

## 6. C6 播放序列

### 6.1 接口

```ts
interface SequenceProvider {
  next(): unknown;                 // 由 SDK 在提供者的作用域里调用
  previous(): unknown;
  canNext: Bindable<boolean>;      // 每轮更新重新求值，变化才发送
  canPrevious: Bindable<boolean>;
}
interface Player {
  /** 登记为播放序列提供者；同时只能有一个。dispose 撤销。 */
  provideSequence(provider: SequenceProvider): Disposable;
  next(): Promise<void>;           // 交给提供者；不等提供者执行完
  previous(): Promise<void>;
}
interface PlayerState { readonly can: { /* 现有字段 */ next: boolean; previous: boolean } }
```

能力：`provideSequence`、`next`、`previous` 需要 `playback.control`；`can.next/previous` 随 `player.state` 读取（`playback.read`，实时更新需 `events`）。

### 6.2 宿主行为

- 宿主保存 `sequence: Option<(owner, can_next, can_previous)>`。wire：`sequence.set { canNext, canPrevious }`（首次即登记；已有别的提供者 → `conflict`），`sequence.set null` 撤销；`sequence.step { direction: "next" | "previous" }`。
- `sequence.step`：没有提供者，或对应标志为假 → `unavailable`；否则向提供者推事件 `sequence.step { direction }` 后立即答复调用者。提供者的 SDK 在作用域里调用 `next()` / `previous()`，抛错交给提供者自己的 `onError`。
- 标志并入来源状态：宿主在每个 `source.state` 事件和 `source.state` 答复的数据中加 `sequence: { next, previous }`；标志变化时，宿主用当前来源状态（`eventSequence` 不变）重发一次 `source.state`。SDK 已接受「序号相同、内容不同」的状态（`player.ts` `accept`），所以不需要新事件；来源戳不变，`ifCurrent` 条件不受影响。
- 提供者停用、`dispose` 或插件退出时，宿主撤销提供者，标志变为假并重发状态。

### 6.3 错误

`conflict`（已有其它提供者，激活期间 = 激活失败）、`unavailable`（无提供者或不能前进/后退）、`capability`。

### 6.4 示例

```ts
// 播放列表：登记提供者
plugin.player.provideSequence({
  canNext: () => !!queue.neighbor(1) && !busy(),
  canPrevious: () => !!queue.neighbor(-1) && !busy(),
  next: () => action("next"),
  previous: () => action("previous"),
});

// 任何插件：调用
plugin.commands.add({
  id: "next", path: ["播放", "下一个"], bindings: ["PageDown", "Mouse:X2"],
  enabled: () => plugin.player.state.can.next,
  run: () => plugin.player.next(),
});
```

### 6.5 取代与删除、迁移

无旧接口（以前插件之间没有办法互相调用）。参考迁移中 playlist 登记为提供者（用户看不到变化；菜单「上一项/下一项」仍是它自己的命令）。其它插件本轮不使用。

## 7. C7 面板位置状态（侧栏与底栏）

### 7.1 接口

```ts
interface Panels {
  mount(slot: "bottom" | "sidebar", file: string,
        options?: { component?: string; global?: string; title?: string }): Panel;
  /** 底栏位置的状态；任何插件可读可切换，不需要知道是谁挂的。 */
  readonly bottom: PanelSlot;
  readonly sidebar: SidebarSlot;
}
interface PanelSlot {
  readonly state: SlotState | null;          // null：该位置没有面板
  onChange(listener: (state: SlotState | null) => void): Disposable;
  setVisible(visible: boolean): Promise<void>;  // 设置用户偏好
  toggle(): Promise<void>;                      // 按宿主当前值翻转偏好
}
interface SidebarSlot extends PanelSlot {
  setDetached(detached: boolean): Promise<void>;  // C14
}
interface SlotState {
  preferred: boolean;      // 用户的选择（菜单勾选）
  effective: boolean;      // 现在是否显示（附着在主窗口或浮动窗口中）
  reason: "userHidden" | "space" | "fullscreen" | "idle" | null;
  detached: boolean;       // 侧栏是否为浮动窗口；底栏总为 false
}
interface Panel extends View, Disposable {
  readonly ready: Promise<void>;
  setInteraction(locks: Partial<{ focused; dragging; popupOpen: boolean }>): void; // 不变
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>; // C11
}
```

能力：`panels.bottom/sidebar` 需要 `ui.mount`；`state` 的实时更新需要 `events`（没有时只在自己的 `setVisible`/`toggle`/`setDetached` 之后刷新）。

### 7.2 宿主行为

- wire：`layout.state` → `{ bottom: SlotState | null, sidebar: SlotState | null }`，在布局未确认时答复 `pending`（沿用现在 `ui.mountState` 的规则，SDK 保留上一次值）；`layout.set { slot, visible?, detached? }`。不再要求调用者就是挂载者。
- SDK 在收到 `window.state` 事件后重新读 `layout.state`（现在 `PanelHandle.#read` 的逻辑移到模块级，每个插件一份），值变化才回调 `onChange` 并触发一轮更新。
- 偏好由宿主持久化（C8，按挂载该位置的插件的 manifest id 记录侧栏；底栏不持久化，与现在相同）。

### 7.3 错误

`missing`（该位置没有面板）、`invalid`（对底栏设 `detached`）、`unavailable`（无界面模式）、`capability`。

### 7.4 示例

```ts
// playback 的底栏按钮切换播放列表侧栏（第二步 UI-BAR-09 的写法）
const sidebar = plugin.panels.sidebar;
bar.bind(() => ({
  listShown: sidebar.state?.preferred ?? false,
  listAvailable: sidebar.state !== null,
}));
bar.on("toggleList", () => sidebar.toggle());
```

### 7.5 取代与删除、迁移

- 删除：`Panel.visibility`、`onVisibility`、`setVisible`、`toggle`、类型 `PanelVisibility`（改名 `SlotState`，加 `detached`）；wire `ui.mountState`、`ui.showMount`（含 SDK 从不调用的 `scope: "presentation"` 分支与 `Policy::reveal_sidebar_in_fullscreen`、`sidebar_override`）；`MountVisibility` 类型；测试宿主中对应的假实现。
- 迁移（§15）：`panel.toggle()` → `plugin.panels.<位置>.toggle()`；`panel.visibility?.x` → `plugin.panels.<位置>.state?.x`；`panel.setVisible(v)` → `plugin.panels.<位置>.setVisible(v)`；`onVisibility` → `onChange`。只有 playback（底栏）和 playlist（侧栏）用到。

## 8. C8 界面状态持久化

### 8.1 宿主行为（无插件接口）

文件：`$VESTRA_UI_STATE`，未设置时 `~/Library/Application Support/Vestra/ui-state.json`（与包存储同一目录）。无界面模式（`--headless`）不读不写。

```json
{
  "version": 1,
  "window": { "x": 102, "y": 143, "width": 780, "height": 671 },
  "sidebar": { "width": 317 },
  "panels": {
    "vestra.playlist": { "visible": true, "detached": false,
                         "floating": { "x": 438, "y": 100, "width": 300, "height": 408 } }
  },
  "bindings": { "vestra.playback/toggle-pause": ["Space"] }
}
```

| 字段 | 何时读 | 何时写 |
| --- | --- | --- |
| `window` | 创建主窗口前（逻辑像素；恢复前检查是否在某个屏幕可见范围内，不在就用默认位置） | 普通模式下尺寸或位置变化（全屏不记录） |
| `sidebar.width` | 建主窗口时 | 分隔条松开 |
| `panels.<id>` | 该插件挂载侧栏时作为初始 `preferred`、`detached`、浮动窗口位置 | 偏好、分离状态、浮动窗口位置变化 |
| `bindings` | 启动时载入覆盖层（C3） | 本轮不写，原样保留 |

- 写入：变化后 1 秒合并写一次，退出前再写一次；先写临时文件再改名，避免写坏。
- 读失败（不是 JSON、`version` 不认识）：写日志，把原文件改名为 `ui-state.json.bad`，用默认值启动。单个字段非法只忽略该字段。
- 模块：新文件 `src/host/src/ui_state.rs`（`load() -> UiState`、`UiState::changed()`、`save_if_due(now)`），宿主主循环的 16 ms 定时器里调用 `save_if_due`。

### 8.2 取代与删除、迁移

无旧接口。插件不需要改。注意：侧栏显示状态从「每次启动都显示」变为「恢复上次」，这是 C8 的预期变化（UI-PL-03）。

## 9. C9 窗口标题

宿主行为（无插件接口）：没有来源时标题为 `Vestra`；有来源时为 `<名称> - Vestra`。名称：本地文件取文件名；网络来源取解析器给的 `title`，没有就用 URL；生成流（模拟器）取 `Vestra`。在主循环收到 `source.state` 变化时更新 `shell.window()` 的标题（`HostWindow` 加 `in property <string> window-title`）。

删除：`host.slint` 的固定标题 `"Vestra · experimental host"`。迁移：无。

## 10. C10 视频信息层与播放统计

### 10.1 接口

```ts
interface OsdPanel extends Disposable {
  /** 在视频左上角显示一块只读多行文字；null 隐藏。最后写入生效。 */
  set(text: string | null): void;
}
interface Osd { panel(): OsdPanel }        // 每个插件最多一个，第二次调用抛错

interface Player { stats(): Promise<PlaybackStats> }
interface PlaybackStats {
  backend: string;                 // 例如 "libmpv · 软件渲染"
  session: number;                 // 原诊断行的 session
  framesShown: number;             // 宿主已显示的帧数（原诊断行的 embedded frame）
  framesDropped: number | null;
  video: { codec: string | null; width: number | null; height: number | null; fps: number | null } | null;
  audio: { codec: string | null; sampleRate: number | null; channels: number | null } | null;
}
```

能力：`osd.panel` 需要 `ui.osd`；`stats` 需要 `playback.read`。

### 10.2 宿主行为

- 信息层：视频区左上角，位于屏幕提示一行之下（y = 36px，x = 10px），半透明黑底 `#000000b0`、内边距 6px、`Theme.font-mono` 11px、`Theme.text`；不接收输入（不放 TouchArea），跟随视频区，全屏时也显示。多个插件各有一块时按 owner 代次从上到下排列。
- 文字最多 32 行、4096 字节；SDK 用「最后写入生效」泵发送（同一时间一个请求在途），每秒刷新一次只产生一个请求。wire：`osd.panel { text: string | null }`。
- 插件停用或句柄 `dispose` 时宿主移除该块。
- `playback.stats`：转给后端工作线程，从 libmpv 读 `video-codec`、`video-params/w,h`、`estimated-vf-fps`（无则 `container-fps`）、`audio-codec-name`、`audio-params/samplerate`、`audio-params/channel-count`、`frame-drop-count`，加上宿主自己的 session 与已显示帧序号。没有来源时 `video`、`audio` 为 `null`。

### 10.3 错误

`osd.panel` 第二次调用：SDK 同步抛错（同「一个插件只能挂一个面板」）。`set` 失败（`invalid` 超长、`unavailable` 无界面）交给作用域 `onError`。`stats`：`capability`、`backend`（工作线程停止）。

### 10.4 播放信息插件（新建 `src/plugins/info/`）

manifest 能力：`commands`、`playback.read`、`events`、`ui.osd`。一条命令：

```ts
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();
  let shown = false, timer: ReturnType<typeof setInterval> | undefined;
  const render = async () => {
    const s = await plugin.player.stats(), p = plugin.player.state;
    if (!shown) return;               // 等待期间被关掉了
    panel.set([p.source ?? "未打开媒体", `${clock(p.position)} / ${clock(p.duration)}`,
      `视频 ${s.video?.codec ?? "-"} ${s.video?.width ?? "-"}×${s.video?.height ?? "-"} ${s.video?.fps ?? "-"} fps`,
      `音频 ${s.audio?.codec ?? "-"} ${s.audio?.sampleRate ?? "-"} Hz ${s.audio?.channels ?? "-"} 声道`,
      `${s.backend} · session ${s.session} · 帧 ${s.framesShown} · 丢帧 ${s.framesDropped ?? "-"}`].join("\n"));
  };
  plugin.commands.add({
    id: "info.toggle", path: ["视图", "播放信息"], order: 503, bindings: ["Tab"],
    checked: () => shown,
    run: () => {
      shown = !shown; clearInterval(timer);
      if (shown) { void render(); timer = setInterval(() => void render(), 1000); } else panel.set(null);
    },
  });
  plugin.onDispose(() => clearInterval(timer));
}
```

（`clock` 是插件内的时间格式化函数。菜单位置见 §18 Q10。）

### 10.5 取代与删除、迁移

删除宿主诊断文字（见 C1 删除项）。现有插件无需改动。

## 11. C11 面板右键菜单

### 11.1 接口

```ts
interface Panel {
  /**
   * 在本面板所在窗口弹出一组本插件的命令（null = 分隔线），样式与主菜单相同。
   * at 是窗口坐标（逻辑像素），在 Slint 里取 self.absolute-position + 指针位置。
   */
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>;
}
```

### 11.2 宿主行为

- wire：`ui.menu { items: (string | null)[], x, y }`，`items` 是本插件命令 id。宿主用注册表里的标题、状态、勾选/单选和绑定表的快捷键文字生成**单层**菜单，复用 `MenuView`（菜单打开期间绑定暂停，Esc 关闭，点外面关闭）。选择一项 = 与主菜单相同的执行路径（`resolve_execution` → `commands.execute`）。
- 弹在面板当前所在的窗口：附着时在主窗口的菜单层；分离时在浮动窗口自带的菜单层（C14 生成的窗口源码包含同一个 `MenuView`）。同一时刻只有一个菜单，新的替换旧的（包括主右键菜单）。
- 子菜单、工具窗口里的右键菜单本轮不做（§18 Q8）。

### 11.3 错误

`missing`（某个 id 不是本插件的命令或已释放）、`unavailable`（面板当前未显示）、`invalid`（空列表、超过 32 项、坐标非有限数）。

### 11.4 示例

```ts
// sidebar.slint：TouchArea { pointer-event(e) => { if e.button == PointerEventButton.right && e.kind == PointerEventKind.down {
//   Bridge.context-menu(self.absolute-position.x / 1px + self.mouse-x / 1px, self.absolute-position.y / 1px + self.mouse-y / 1px); } } }
const play = plugin.commands.add({ id: "play", path: ["播放列表", "播放"], run: playSelected });
const remove = plugin.commands.add({ id: "remove", path: ["播放列表", "移除所选"], run: removeSelected });
const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "播放列表" });
list.on("contextMenu", (x: number, y: number) => list.showMenu([play, null, remove], { x, y }));
```

### 11.5 取代与删除、迁移

无旧接口。本轮插件不使用（播放列表右键菜单是第二步 UI-PL-18）。

## 12. C14 面板分离

### 12.1 接口

`plugin.panels.sidebar.setDetached(detached)` 与 `SlotState.detached`（见 C7）；`panels.mount` 的 `title` 选项作为浮动窗口标题（默认空，宿主显示 `Vestra`）。插件的面板句柄、属性、回调**完全不变**。

### 12.2 宿主行为：同一份面板内容

- 主窗口的源码不变（面板组件仍被导入主窗口，`global` 桥接对象在主窗口实例里）。分离时，宿主为该面板**另外编译一个浮动窗口**，源码由宿主生成：

```slint
import { <component> as Owner7View, <global> as Owner7Bridge } from "<面板文件>";
import { MenuView, MenuLevel } from "<宿主 menu-view.slint>";
import { Theme } from "@vestra/theme.slint";
export { Owner7Bridge }
export component FloatingPanel inherits Window {
    title: "<title>"; min-width: 300px; min-height: 250px;
    default-font-family: "<系统 UI 字体>"; background: Theme.bg-list;
    // key-input / mouse-input / 菜单层：与 HostWindow 相同的宿主属性和回调
    keys := FocusScope { /* 全局 + 面板范围绑定 */  Owner7View { width: parent.width; height: parent.height; } }
}
```

- Slint 的 global 是每个窗口实例各一份，所以宿主把同一份属性写入两边：浮动窗口创建时用 `Mount.properties`（宿主已为重建主窗口而保存的全部已设属性）初始化；之后每次 `ui.setMount` 同时写主窗口和浮动窗口。两边的回调都以 `$mount` 转发给同一插件，插件感觉不到窗口换了。附着时关闭浮动窗口，主窗口里的属性本来就是最新的，不需要重建主窗口。
- 只有用户在 Slint 里改过、但插件没有回写的 in-out 属性（例如滚动位置、输入框里没提交的文字）在分离/附着时不保留——与现在其它插件挂载导致主窗口重建时的行为相同。插件状态（列表行、选中项）在插件里，不丢。
- 浮动窗口：默认位置 (438, 100)、默认尺寸 300×408，最小 300×250；位置与尺寸变化时记入 C8。点窗口的关闭按钮 = `setVisible(false)`，仍保持分离；之后再显示时仍是浮动窗口。
- 分离时 `effective` 只看 `preferred`（不受主窗口空间、全屏影响）；主窗口里的侧栏位置空出，视频占满。
- 启动时若持久化记录为分离且显示，面板挂载完成后直接打开浮动窗口。
- 插件停用：两个实例一起撤销。主窗口重建（别的插件挂载）时浮动窗口不受影响，宿主把它的回调重新指向新的主窗口代次。

### 12.3 错误

`missing`（没有侧栏面板）、`unavailable`（无界面模式）、`ui-prepare`（浮动窗口编译失败；状态保持附着）。

### 12.4 示例

```ts
// src/examples 中的演示：两条命令触发分离与附着（按钮由 playlist 在第二步加）
const sidebar = plugin.panels.sidebar;
plugin.commands.add({ id: "detach", path: ["视图", "分离侧栏"], order: 510,
  enabled: () => sidebar.state !== null && !sidebar.state.detached,
  run: () => sidebar.setDetached(true) });
plugin.commands.add({ id: "attach", path: ["视图", "附着侧栏"], order: 511,
  enabled: () => sidebar.state?.detached ?? false,
  run: () => sidebar.setDetached(false) });
```

### 12.5 取代与删除、迁移

无旧接口。参考迁移中 playlist 的 `mount` 加 `title: "播放列表"`（附着时看不到变化）。

## 13. 生命周期汇总

| 资源 | 归属 | 插件停用 / dispose 时 |
| --- | --- | --- |
| 命令的绑定 | 随命令 | 宿主 `remove_owner` 后整表重算；SDK 释放命令时命令变为禁用（绑定仍占着键但不执行，与现在相同） |
| 侧栏偏好、分离状态 | 宿主（按 manifest id 持久化） | 面板卸载，状态留在文件里供下次挂载 |
| 浮动窗口 | 宿主 | 关闭 |
| 序列提供者 | 提供者插件 | 撤销，`can.next/previous` 变为假 |
| 信息层 | 插件 | 移除 |
| 屏幕提示 | 无 | 到时消失 |
| 面板右键菜单 | 宿主 | 若属于该插件则关闭 |

## 14. 迁移后菜单树为什么不变

现在的顺序（`commands.rs` `snapshot()`）：顶层按宿主领域表（文件、播放、视频、音频、字幕、视图、模拟器、插件），其下按 `path[1..]` 的 UTF-8 字节字典序，子菜单与菜单项交错排列，没有分隔线。新模型只看 `order`，子菜单取其最前子项的 `order`，所以只要给每条命令一个**等于它现在显示位置**的 `order` 就能逐项重现：

- 每个旧领域一段：文件 200、播放 260、视频 320、音频 380、字幕 440、视图 500、模拟器 560、插件 620（都在推荐骨架的 `domain` 段 200–699 内，第二步改版时不必挪段）；
- 段内按现在的显示顺序（展开所有子菜单后的先序）从 +1 起编号；
- 不写 `group`：全部是默认 `"domain"`，不出现分隔线；
- 顶层：文件(201) < 播放(261) < 视频(321) < 音频(381) < 字幕(441) < 视图(501) < 模拟器(561) < 插件(621)，与领域表顺序相同；各子菜单内的相对顺序与现在相同，跨插件交错（音频下的 audio 与 playback）也由编号保证。

| 插件 | 命令 id | 路径（不变） | `order` | `bindings` |
| --- | --- | --- | --- | --- |
| playback | `close-source` | 文件 / 关闭当前来源 | 201 | |
| playback | `open` | 文件 / 打开媒体… | 202 | |
| playlist | `playlist.previous` | 文件 / 播放列表 / 上一项 | 203 | |
| playlist | `playlist.move-up` | 文件 / 播放列表 / 上移 | 204 | |
| playlist | `playlist.next` | 文件 / 播放列表 / 下一项 | 205 | |
| playlist | `playlist.move-down` | 文件 / 播放列表 / 下移 | 206 | |
| playlist | `playlist.play` | 文件 / 播放列表 / 播放选中项 | 207 | |
| playlist | `playlist.add` | 文件 / 播放列表 / 添加文件… | 208 | |
| playlist | `playlist.link` | 文件 / 播放列表 / 添加链接… | 209 | |
| playlist | `playlist.clear` | 文件 / 播放列表 / 清空列表 | 210 | |
| playlist | `playlist.remove` | 文件 / 播放列表 / 移除选中项 | 211 | |
| bilibili | `bilibili.open` | 文件 / 网络媒体 / Bilibili 视频… | 212 | |
| youtube | `youtube.open` | 文件 / 网络媒体 / YouTube 视频… | 213 | |
| playback | `forward-5` | 播放 / 前进5秒 | 261 | `["ArrowRight"]` |
| playback | `backward-5` | 播放 / 后退5秒 | 262 | `["ArrowLeft"]` |
| playback | `toggle-pause` | 播放 / 播放/暂停 | 263 | `["Space"]` |
| playback | `jump` | 播放 / 跳转到… | 264 | |
| playback | `rate-05` | 播放 / 速度 / 0.5倍 | 265 | |
| playback | `rate-15` | 播放 / 速度 / 1.5倍 | 266 | |
| playback | `rate-20` | 播放 / 速度 / 2倍 | 267 | |
| playback | `rate-10` | 播放 / 速度 / 正常速度 | 268 | |
| playback | `rate-custom` | 播放 / 速度 / 自定义… | 269 | |
| capture | `capture.still` | 视频 / 截图与导出 / 保存当前画面… | 321 | |
| capture | `capture.clip` | 视频 / 截图与导出 / 导出片段… | 322 | |
| capture | `capture.tasks` | 视频 / 截图与导出 / 查看导出任务… | 323 | |
| capture | `capture.burst` | 视频 / 截图与导出 / 连拍… | 324 | |
| video | `video.picture` | 视频 / 画面调节… | 325 | |
| video | `video.gaussian` | 视频 / 高斯模糊… | 326 | |
| audio | `audio.preset.1` | 音频 / 均衡器预设 / 人声 | 381 | |
| audio | `audio.preset.0` | 音频 / 均衡器预设 / 平直 | 382 | |
| audio | `audio.preset.3` | 音频 / 均衡器预设 / 影院 | 383 | |
| audio | `audio.preset.2` | 音频 / 均衡器预设 / 音乐 | 384 | |
| audio | `audio.reset` | 音频 / 复位音频处理 | 385 | |
| playback | `mute` | 音频 / 静音 | 386 | |
| playback | `volume` | 音频 / 音量… | 387 | |
| audio | `audio.workbench` | 音频 / 音频工作台… | 388 | |
| subtitles | `disable` | 字幕 / 关闭字幕 | 441 | |
| subtitles | `refresh` | 字幕 / 刷新字幕轨 | 442 | |
| subtitles | `load` | 字幕 / 加载外部 SRT… | 443 | |
| subtitles | `workbench` | 字幕 / 字幕工作台… | 444 | |
| playlist | `playlist.visibility` | 视图 / 显示播放列表 | 501 | |
| playback | `bar-visible` | 视图 / 显示播放栏 | 502 | |
| nes | `nes.window` | 模拟器 / NES 工具 | 561 | |
| nes | `nes.stop` | 模拟器 / 停止 NES | 562 | |
| nes | `nes.open` | 模拟器 / 打开 NES ROM… | 563 | |
| workshop | `workshop.open` | 插件 / 插件工坊… | 621 | |

新增项（不属于迁移，是本轮预期的增加）：播放信息插件「视图 / 播放信息」503；宿主「全屏」700、「退出」990（§18 Q2）；主菜单每行右侧出现快捷键文字（C4）。

**验证**（P1b 由维护者加，协调者复核）：

1. `commands.rs` 单元测试：按上表注册 46 条命令（打乱注册顺序），断言展开后的菜单树先序与现在 `snapshot()`（领域表 + 字节序）的结果完全相同。
2. 快照脚本 `menus-snapshot.test.ts`：`shortcut` 改为读 `declaration.bindings`（对比时 `null` ↔ `[]`，`"Space"` ↔ `["Space"]`），并多输出 `order`；`menus-after.json` 与 `menus-before.json` 对比路径、标题、快捷键，再把 `order` 与上表对比。现有快照不含顺序，所以第 1 条测试是顺序不变的主要证据。

## 15. 迁移写法（迁移者照抄）

### 15.1 命令：`shortcut` → `bindings`，加 `order`

```ts
// 旧
plugin.commands.add({ id: "toggle-pause", path: ["播放", "播放/暂停"], shortcut: "Space", run });
// 新
plugin.commands.add({ id: "toggle-pause", path: ["播放", "播放/暂停"], order: 263, bindings: ["Space"], run });
```

没有 `shortcut` 的命令只加 `order`（§14 表）。用表驱动注册的插件（playback `commands.ts`、playlist `main.ts`、audio 预设）在表里加一列 `order`。

### 15.2 面板可见性 → 位置状态

```ts
// 旧（playback）
entry("bar-visible", ["视图", "显示播放栏"], () => bar.toggle(), () => ready(true, bar.visibility?.preferred ?? true))
// 新
const bottom = plugin.panels.bottom;
entry("bar-visible", ["视图", "显示播放栏"], () => bottom.toggle(), () => ready(true, bottom.state?.preferred ?? true))

// 旧（playlist）
checked: () => (name === "visibility" ? (list.visibility?.preferred ?? true) : null),
reason: () => name === "visibility" && list.visibility?.reason === "space" ? "空间不足，放大窗口后可显示" : …,
case "visibility": await list.toggle(); break;
list.on("hide", () => list.setVisible(false));
// 新
const sidebar = plugin.panels.sidebar;
checked: () => (name === "visibility" ? (sidebar.state?.preferred ?? true) : null),
reason: () => name === "visibility" && sidebar.state?.reason === "space" ? "空间不足，放大窗口后可显示" : …,
case "visibility": await sidebar.toggle(); break;
list.on("hide", () => sidebar.setVisible(false));
```

`setInteraction` 不变。

### 15.3 测试

```ts
// 旧
assert.deepEqual(host.command("workshop.open").declaration, { title: "插件工坊…", path: ["插件", "插件工坊…"], shortcut: null });
// 新（写了 order，测试宿主收到完整形状，未写的字段是默认值）
assert.deepEqual(host.command("workshop.open").declaration, {
  title: "插件工坊…", path: ["插件", "插件工坊…"], order: 621, group: "domain",
  radio: false, bindings: [], focus: null, repeat: false });
```

测试宿主新增：`host.press("Space", { panel? })`（按绑定表执行命令，禁用时同样拒绝；`panel: true` 模拟面板有焦点）、`host.conflicts`、`host.layout`（两个位置的状态，可改后 `host.emit("window.state", {})` 模拟别处的变化）、`host.osd`（显示过的提示，最新在后）、`host.osdPanel`、`host.sequence`（当前提供者的标志）与 `host.step("next")`、`host.stats`（`player.stats()` 的返回值）、`host.menus`（`showMenu` 的记录）。`host.mounted().visibility` 在 P4 删除，迁移后改读 `host.layout`。

### 15.4 逐插件清单（P3）

| 插件 | 要改的 |
| --- | --- |
| playback（维护者参考迁移） | 14 条命令加 `order`；3 处 `shortcut` → `bindings`；`bar.toggle()`/`bar.visibility` → `plugin.panels.bottom` |
| playlist（维护者参考迁移） | 10 条命令加 `order`；侧栏可见性 → `plugin.panels.sidebar`；`mount` 加 `title: "播放列表"`；登记 `provideSequence`（复用现有 next/previous 逻辑） |
| video、audio、subtitles、capture、nes、youtube、bilibili | 每条命令加 `order`；测试里 `declaration` 断言改新形状 |
| workshop | `order: 621`；测试断言；`vendor/sdk` 用新 SDK 重新复制（`tests/vendor-sdk.test.mjs` 要求逐字节相同，命令写在该测试文件开头；P1b 起这条测试失败，直到复制） |
| examples/bookmark（维护者） | 「播放 / 书签…」是示例，不在快照里；加 `order: 270` |

## 16. 删除清单（P4，全部完成后代码中不再出现）

宿主（`src/host/`）：

- `commands.rs`：`Declaration.shortcut`；`canvas_shortcut()`；`Registry::resolve_shortcut()`；`Registry::new` 的领域参数与 `domains`、`domain_order`；「2..=6 segments」「menu domain is not allowed」「main-canvas shortcut already registered」检查；`snapshot()` 中的领域排名和 `path[1..]` 字母排序；对应旧测试改写。
- `main.rs`：Host 初始化里的 8 个领域名；`Host::canvas_shortcut`；`Event::WindowInput` 的 `shortcut:`、`toggleFullscreen`、`escape` 分支；`ui.mountState`、`ui.showMount` 的路由与能力表条目；`scope: "presentation"` 分支；试载改写 `path[1]` 和清空 `shortcut`；`diagnostic` 的两处写入；`sidebar_resize_requested`、`sidebar_resize` 及其计算（若 §18 Q7 通过）；producer 钩子中的 `"F11" | "Escape"`。
- `window_state.rs`：`reveal_sidebar_in_fullscreen`、`sidebar_override`；`CANVAS_MIN_WIDTH = 640`、`CANVAS_MIN_HEIGHT = 360`、`SIDEBAR_MIN_WIDTH = 240`、`SIDEBAR_TARGET_WIDTH = 280` 与「空间不足直接隐藏」规则（换成 318/3/317）。
- `ui.rs`、`composition.rs`：`Arial Unicode.ttf` 导入、`"Arial Unicode MS"`；三处各自的 `Compiler::default()`（合并为一个）。
- `host.slint`：Commands 按钮行与说明文字；`diagnostic` 属性与文字行；`padding: 8px`；`default-font-family: "Arial Unicode MS"`；canvas `key-pressed` 中写死的 F11/Escape/Space/←/→；1100×720、720×480；固定标题；`#111318` 等写死颜色。
- `menu-view.slint`：旧配色、圆角、文字 ✓。

SDK 与运行器（`src/sdk/`）：

- `plugin/types.ts`：`CommandOptions.shortcut`；`Panel.visibility`、`onVisibility`、`setVisible`、`toggle`；`PanelVisibility`。
- `plugin/ui.ts`：`PanelHandle` 的可见性读取与切换代码（移到模块级的位置状态）。
- `commands.ts`：`CommandDeclaration.shortcut`。
- `wire.ts`：`ui.showMount`、`ui.mountState`、`MountVisibility`。
- `plugin/testing.ts`：`ui.mountState` 假实现、`FakeCommand.declaration.shortcut`、面板可见性假实现。

文档：`doc/plugins/reference.md` §4 `shortcut` 说明与「顶层分组惯例」、§6.3 面板可见性；`doc/architecture/sdk-design.md` §2 表中 `commands`、`panels` 两行；`doc/architecture/README.md` §4 字体一句。

插件：全部 `shortcut:`、`.visibility`、`.onVisibility`、`panel.setVisible`、`panel.toggle()`；workshop `vendor/sdk` 旧副本。

## 17. 错误码汇总（新增用法）

| code | 出现在 |
| --- | --- |
| `invalid` | 绑定语法、路径/`order`/`group` 不合法（`commands.add`）；提示/信息层文字为空或过长；`showMenu` 参数；对底栏设 `detached` |
| `conflict` | 已有别的序列提供者；路径重复（不变）。**绑定冲突不报错**，见 §3.5 |
| `unavailable` | `next`/`previous` 没有提供者或不能前进后退；无界面模式下的提示、信息层、位置状态、分离；`showMenu` 时面板未显示 |
| `missing` | 位置上没有面板；`showMenu` 的命令不属于本插件或已释放 |
| `pending` | `layout.state` 在布局未确认时（SDK 内部处理，插件看不到） |
| `ui-prepare` | 浮动窗口编译失败 |
| `capability` | 未声明 `ui.osd` 却调用 `plugin.osd.*`，其余同现有规则 |

## 18. P2 裁决（2026-10-20）

协调者审查通过。下表是最终决定（原问题与选项见 git 历史中的 P1a 版本）。

| # | 问题 | 裁决 |
| --- | --- | --- |
| Q1 | 浮动窗口默认尺寸 | 300×408（最小 300×250） |
| Q2 | 宿主「全屏」「退出」进菜单顶层 | 进（`screen` 组 700、`exit` 组 990）；P1b 确认 ⌘Q 不与 macOS 默认应用菜单重复 |
| Q3 | 鼠标事件范围 | 左键单击、长按、右键双击不做；`Mouse:LeftDoubleClick` 作为宿主全屏的默认绑定 |
| Q4 | 插件间绑定冲突 | 先加载者胜（§3.5） |
| Q5 | 焦点范围 | 只做 `panel` |
| Q6 | 按住重复 | 命令可声明 `repeat: true`，默认 false（§3.1、§3.2） |
| Q7 | 自动放大窗口 | 删除 |
| Q8 | 面板右键菜单 | 单层、只在面板 |
| Q9 | 省略 `path` | 允许；不进主菜单，`title` 必填（§4.1、§4.2） |
| Q10 | 播放信息菜单位置 | 「视图 → 播放信息」 |
| Q11 | 快捷键文字格式 | `Ctrl+O`、箭头 |
| Q12 | 默认字体 | 按实测顺序；只能按文件导入时用系统自带 Hiragino Sans GB |
| Q13 | PotPlayer 样式控件 | `@vestra/widgets.slint` 提供 `Slider`、`Button`、`ToolTipArea`（§2.1）；其它控件不做 |
| Q14 | 全屏时浮动面板 | 已知限制 |

## 19. P1b 实现记录（2026-10-20）

与上文的出入和实现时查明的事实，按需在 P5 验证。

- **代码位置**：宿主新文件 `bindings.rs`（绑定表）、`ui_state.rs`（持久化）、`ui/vestra/theme.slint`、`ui/vestra/widgets.slint`；`commands.rs` 重写为新菜单模型（含 §14 的 46 条命令顺序测试）；`menu.rs` 增加分隔线、单选、快捷键列与单层面板菜单；`host.slint` 重写外壳；`composition.rs` 生成侧栏分隔条、按键转发层与浮动窗口源码。SDK 新文件 `sdk/plugin/bindings.ts`（与宿主同规则的语法检查）、`sdk/plugin/osd.ts`。新插件 `src/plugins/info/`；演示 `src/examples/core-ui/`（菜单分组与单选、`repeat` 绑定、不进菜单的命令、屏幕提示、`next/previous` 调用、侧栏开关与分离、主题控件窗口）与 `src/examples/panel-menu/`（面板右键菜单、面板焦点按键、序列提供者、浮动窗口标题）。
- **⌘Q**：Slint 1.16 的 winit 后端在 macOS 默认建立应用菜单（muda，含系统「退出 ⌘Q」，`i-slint-backend-winit` `muda.rs` `create_default_app_menu`），⌘Q 由系统菜单先处理，不会到达窗口，所以不会和宿主的 `Cmd+Q` 绑定重复触发；宿主绑定在 macOS 上只提供菜单里的快捷键文字。经系统菜单退出时宿主不经过自己的退出路径，界面状态最多丢失最后 1 秒内未写入的变化（右键菜单「退出」会先写入）。需 P5 实测。
- **主题令牌**：`Theme.font-family` 未提供（默认字体由宿主统一设置，§2.2）；`font-mono` 为 `Menlo`。
- **wire 方法**：`playback.stats` 由后端线程读 libmpv；`layout.set` 在答复前跑一次布局，紧接着的 `layout.state` 能看到结果。`ui.menu` 的宿主错误码按 §11.3。
- **滚轮步长**：每 40 逻辑像素一步（`WHEEL_STEP`），需按触控板与鼠标实测调整。
- **删除提前做了的部分**（因为外壳已换，旧代码不再有调用者）：`host.slint` 写死的 Space/←/→/F11/Esc 路由、`Host::canvas_shortcut`、`WindowInput` 的 `shortcut:`/`toggleFullscreen`/`escape` 分支、`Registry::resolve_shortcut`、领域列表、`diagnostic` 及后端为它维护的状态快照、`sidebar_resize*`（Q7）、Arial Unicode 字体。§16 其余项（`shortcut` 字段、旧面板可见性接口与 wire、`reveal_sidebar_in_fullscreen`、测试宿主旧字段）留到 P4。
- **迁移期菜单顺序**：未迁移插件的命令 `order` 为 0，P3 完成前它们在 GUI 里排在已迁移命令之前；按插件比较的菜单快照不受影响。

### P4 删除（2026-10-20）

§16 的清单已全部完成，迁移期的并存代码一并删除：

- SDK：`CommandOptions.shortcut`；`sdk/commands.ts` 的 `LegacyCommandDeclaration` 与旧声明形状（SDK 现在总发完整声明，未写的字段为默认值）；`Panel.visibility`、`onVisibility`、`setVisible`、`toggle` 与类型 `PanelVisibility`，以及 `PanelHandle` 里读取 `ui.mountState`、订阅 `window.state` 的代码（面板挂载后改为刷新 `panels.<位置>` 的状态）；wire 的 `ui.showMount`、`ui.mountState`、`MountVisibility`。测试宿主：`ui.showMount`、`ui.mountState` 假实现、面板假对象的 `visibility`、按 `shortcut` 查绑定的兼容分支；`FakeCommand.declaration` 只有完整形状。
- 宿主：`commands::Declaration.shortcut` 与 `all_bindings()`；`ui.mountState`、`ui.showMount` 的路由与能力表条目（含 `scope: "presentation"`）；试载时清空 `shortcut`；`window_state` 的 `reveal_sidebar_in_fullscreen`、`sidebar_override`（全屏时附着的侧栏总是隐藏，浮动窗口不受影响）。菜单行里的快捷键文字字段改名为 `keys`（与已删除的 `shortcut` 字段区分，含义不变）。
- 测试：SDK 测试与 playback 测试改读完整声明和 `host.layout`；`window_state` 的全屏覆盖测试换成「全屏隐藏附着侧栏、保留偏好与浮动窗口」。
- 文档：`doc/plugins/reference.md` 去掉两条迁移期说明；`doc/plugins/` 中不再出现已删除的接口。workshop 的 `vendor/sdk` 按其测试开头的命令重新同步（协调者授权）。

