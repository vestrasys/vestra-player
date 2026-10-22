# 架构总览（维护者）

给 API 维护者的地图：组件怎么分工、插件怎么被加载和停止、SDK 与宿主之间说什么协议、代码在哪。插件作者看 [doc/plugins/](../plugins/README.md)；SDK 的设计取舍见 [sdk-design.md](sdk-design.md)；项目原则见 [doc/product/](../product/direction.md)。

## 1. 组件

```text
┌──────────── 宿主进程（Rust，src/host）────────────┐
│ Slint 原生界面：主窗口、底栏/侧栏挂载、工具窗口、菜单 │
│ libmpv 播放后端、处理链、字幕、快照/导出工作进程      │
│ 插件管理：加载、停用、试载、安装/回退、owner 资源回收  │
└───────────────▲─────────────────────────────────┘
                │ JSONL over stdin/stdout（wire：vestra/1）
┌───────────────┴──── 每插件一个 Node 进程 ──────────┐
│ 运行器 src/runtime/runner.ts                        │
│ SDK  @vestra-player/sdk（独立仓库，npm 依赖）：plugin/ 句柄式 API → wire 客户端（内部） │
│ 插件 src/plugins/<名字>/：main.ts + .slint           │
└────────────────────────────────────────────────────┘
```

- **Rust 宿主**只提供公共设施：播放、界面组合、处理链、字幕、导出、插件生命周期。不认插件身份，不含插件业务（站点解析、存档、预设等都在插件里）。第一方代码禁止 `unsafe`。
- **TypeScript 插件**是业务的唯一位置。插件之间完全独立，只通过公开宿主状态协作。
- **Slint** 是插件界面的唯一形式：工具窗口（独立窗口）或挂载到底栏/侧栏的面板，由宿主编译和组合。

## 2. 插件的加载与停止

- **开发加载**：`vestra-host --plugin <目录>`（可多次），直接运行入口 `.ts`（Node 26 去类型执行；安装包用内置 Node，见 `src/host/src/runtime_paths.rs`）。
- **托管安装**：`packages.validate` → `trial`/`install`/`rollback`，包存放在 `VESTRA_PACKAGE_STORE`。只接受构建产物（`dist/`，入口为 `.js`，manifest 带 `version`）。
- **激活**：宿主启动 `node runner.ts <包目录>` → 运行器读取 manifest、按能力读初始状态和订阅事件 → 调用默认导出的 `activate(plugin)` → 激活期间的注册全部成功后发送 `runtime.ready`，否则 `runtime.failed`。
- **停用**：宿主按 owner 撤销该插件的全部资源（菜单与绑定、窗口、面板与浮动面板窗口、处理资源、字幕层、快照、任务、解析器、producer、播放序列提供者、屏幕信息块），发送 `runtime.stop`；运行器释放句柄，300 ms 内未退出则被结束。每次运行有新的 owner 代次，旧进程的迟到消息不会路由给重新加载的同名插件。

manifest 规则见 [API 参考 §2](../plugins/reference.md#2-manifest-与能力)。能力是契约声明，宿主据此拒绝未声明的调用（`code: "capability"`），不是安全沙箱。

## 3. wire 协议

- 每行一个 JSON：请求 `{ id, method, params }`，答复 `{ id, result }` 或 `{ id, error: { code, message } }`，事件 `{ event, data }`。
- 管道有序；宿主对每个请求都答复，或者停止该插件。所以运行器不设客户端超时（超过 10 秒只写日志）。
- 协议细节（修订号、来源代次、操作 id、事件序号等）全部由 SDK 吸收，插件看不到，见 [sdk-design.md §5](sdk-design.md#5-协议细节如何被吸收)。
- wire 方法只服务于 SDK。SDK 不再调用的方法应从宿主删除，不为假想的调用方保留（2026-10-20 已删除旧接口：`host.capabilities`、`playback.state/open/seek`、`playback.set` 中除音量/静音外的属性、`window.state`（RPC）/`setFullscreen`、`commands.snapshot/execute`（RPC）、`plugins.list/load/activation/disable`、`jobs.list`、`subtitles.upload.status`，以及 `playback.state`、`plugins.changed`、`host.error` 事件）。
- 插件可订阅的事件只有 `source.state`、`playback.backend`、`window.state`。宿主不经订阅直接推给某个插件的事件：`commands.execute`（菜单或绑定触发了它的命令）、`ui.callback`、`sequence.step`（它是播放序列提供者时）等。
- `source.state`（事件与 RPC 答复）的数据由宿主并入 `sequence: { next, previous }`（播放序列提供者的标志）；标志变化时宿主以相同 `eventSequence` 重发一次当前来源状态，SDK 接受「序号相同、内容不同」的状态。
- `producer.declareInput` 与物理输入子系统保留：SDK 尚未暴露键盘输入，但这是模拟器键盘输入的现成能力。

## 4. Slint 组合与输入

- 编译：宿主、面板、工具窗口、包校验共用一个编译配置（`ui.rs` 的 `compiler()`）：库路径 `vestra` → `ui/vestra/`（`@vestra/theme.slint`、`@vestra/widgets.slint`），风格 `fluent-dark`。字体见下一条。
- 字体（用户裁决 2026-10-22）：宿主不指定任何窗口的字体，各窗口用系统默认字体，缺字（西文字体里的中文、`font-mono` Menlo 里的中文）由 Slint 1.18 按字符回退到系统里有这个字的字体（parley/fontique；macOS 真机与无界面探针已确认，Windows 待 S6 真机）。各平台系统自带中文字体：macOS 苹方，Windows 微软雅黑，Linux 发行版通常带 Noto CJK，Android/iOS 系统字体。1.16 时代强制 Hiragino Sans GB / Microsoft YaHei 的注入（`platform::ui_font`、`ui::font_import`、窗口根上的 `default-font-family`、`host.slint` 的 `// VESTRA-FONT` 标记、`VESTRA_FONT_FALLBACK`）已删除，旧代码见 2f8e6ab 的 `ui.rs`、`composition.rs`、`settings_window.rs`。
  - 若某个平台回退失败（例如没有中文字体的精简 Linux），只给该平台补回退字体，不改其它平台：在该平台模块里加 `ui_fallback_font() -> Option<(family, path)>`（其它平台模块返回 `None`，`platform/mod.rs` 的表里登记），再把上面删掉的注入恢复为调用它（工具窗口仅在插件没写 `default-font-family` 时注入，主窗口、浮动窗口、选项窗口直接注入）。字体用 Noto Sans SC（OFL）：字幕已经带了一份 Noto Sans CJK SC（`src/assets/fonts/noto-cjk/`，16 MB，`font_carrier.rs` 会把它写到临时目录，可以直接 `import` 那个路径），不用再加安装包体积。注意：Slint 没有只把字体加进「按文字回退」链的公开接口，设为默认字体后该平台的西文也会换成 Noto；`SLINT_FONT_PATH` 环境变量效果相同，且要在 Slint 初始化前设置（Rust 里 `set_var` 需要 `unsafe`，不用）。
  - 斜体：系统中文字体都没有斜体，FemtoVG 与软件渲染器不合成斜体，所以 Markdown 斜体仍另加颜色（`ui_markdown.rs` `EMPHASIS_COLOR`）。
- Slint 版本锁定 `=1.18.1`（`Cargo.toml` 与 `unsafe/render`）。依赖的私有接口两处，升级时必须复测：`ui_markdown` 的 `parse_markdown`（带占位参数的 Markdown）、`ui::settle_layout` 的 `WindowInner::ensure_tree_instantiated`。后者的原因：1.17 起 `if`/`for` 元素在每帧渲染和每个输入事件前的一遍里创建和删除，不在读取布局时更新，所以宿主切换 `sidebar-visible` 等条件后、下一帧之前读到的几何是旧的；宿主读视频区和窗口尺寸前先调用它（窗口 tick、画中画定位、`window.setVideoArea`、拖放落点）。
- 工具窗口：插件 `.slint` 文件恰好导出一个组件；宿主用 slint-interpreter 编译，按 SDK 登记的回调白名单转发回调。
- 面板：文件导出一个 component 和一个 global（属性与回调的桥接对象）；宿主按 owner 别名把它导入主界面（`host.slint` 的插入标记），转发 global 上的全部回调。主窗口外壳：视频列（视频 + 底栏）在左，3px 分隔条与全高侧栏在右；全屏时底栏覆盖在视频底部并按 `window_state::Policy` 的空闲规则隐藏（指针停在底栏上时不隐藏），附着的侧栏变成右边缘的弹出层。
- 浮动面板（C14）：分离时宿主另外编译一个窗口，导入同一个组件与桥接 global；global 每个窗口实例一份，所以宿主用保存的 `Mount.properties` 初始化浮动窗口，之后每次 `ui.setMount` 同时写主窗口与浮动窗口，两边回调都以 `$mount` 转发。附着时只关闭浮动窗口，主窗口不重建。
- 输入：主窗口最外层、两个面板各包一层 FocusScope。Slint 的按键先给焦点控件，未接受的逐级冒泡；面板那一层以「面板 + 全局」查表，最外层以全局查表（`bindings.rs`）。查表在 UI 线程上同步进行（`Rc<RefCell<InputShared>>`，不借用 Host），绑定了就返回 accept 并发 `Event::Binding`，主循环再按注册表检查启用状态后执行。视频区鼠标（双击、中键、侧键、滚轮）同理。全屏、退出全屏、退出是宿主自己的命令（owner 0），和插件命令在同一张表与同一棵菜单树里。
- 菜单：`commands.rs` 按 `order`（子菜单取其最前的命令）建树，`menu.rs` 投影为面板列表（分隔线、单选、快捷键文字、行位置），`menu-view.slint` 只负责绘制。面板右键菜单（C11）复用同一投影的单层形式，弹在面板当前所在的窗口。
- 模拟器输入捕获（`producer.declareInput`）在 winit 层先拦截按键；绑定在宿主命令上的键（全屏等）放行。
- 等宽字体：`Theme.font-mono` 读 `VestraPalette.font-mono`（`ui/palette.slint`，默认 Menlo），宿主在每个视图（主窗口、浮动面板、工具窗口、选项窗口）下发配色时一并设置为 `platform::monospace_font(SYSTEM)`（macOS Menlo、Windows Consolas、Linux DejaVu Sans Mono；纯函数，各系统在任何机器上都有测试），`theme::apply_fonts`；与配色无关，Vestra 配色下也设置。
- 窗口标准快捷键（用户裁决 2026-10-22）：工具窗口、浮动面板、选项窗口按系统约定关闭（macOS ⌘W；Windows Ctrl+W、Ctrl+F4；Linux Ctrl+W），主窗口不响应。表在平台层 `platform/shortcuts.rs`：按系统分开的纯函数 `window_shortcuts(System)`，每个平台模块只声明自己的 `SYSTEM`，所以每个系统的条目在任何机器上都有测试；以后加 Linux 桌面变体、Android、iOS 只加条目（移动端表为空，返回手势由那个平台模块映射到 `WindowAction::Close`）。`ui::WindowShortcuts` 在这些窗口的 winit 钩子里最先检查按键（字母按当前布局不带修饰键的字符，非拉丁布局按美式键位；修饰键必须完全一致），命中时调用 `slint::Window::dispatch_event(CloseRequested)`，即标题栏关闭按钮的同一路径（工具窗口 `ViewClosed` → 插件 `ui.closed`，浮动面板 `FloatingClosed`，选项窗口 `Event::Settings(_, None)`，与「关闭」按钮的 `close-window` 相同），并返回 `PreventDefault`：Slint 收不到这次按键，所以输入框获得焦点时也有效，也不会进绑定表或插件的 Slint 处理器。之前选项窗口的 ⌘W 写在 Slint 里，看的是 `modifiers.meta`，而 Slint 在 macOS 上把 ⌘ 报告为 `control`（⌃ 才是 `meta`，见 `SLINT_CONTROL_IS_COMMAND`），所以从来没生效；该处理器已删除。

## 5. 代码地图

| 路径 | 内容 |
| --- | --- |
| `src/host/src/main.rs` | 宿主主循环、RPC 路由与能力检查、事件广播 |
| `src/host/src/plugins.rs`、`package_*.rs` | 插件进程、manifest 校验、托管包存储/试载/安装/回退 |
| `src/host/src/backend.rs`、`source_*.rs`、`session.rs` | libmpv 后端、来源控制与操作簿、网络来源解析 |
| `src/host/src/processing*.rs`、`effects.rs` | 视频/音频处理链 |
| `src/host/src/subtitle_*.rs`、`native_subtitle.rs` | 字幕上传、选择、样式层 |
| `src/host/src/media_*.rs`、`offline_*.rs` | 快照与离线导出工作进程 |
| `src/host/src/producer*.rs` | 插件生成的音视频流（NES）。连接在两帧之间断开（`TransportClosed`）先等 0.5 秒（`CLOSE_GRACE`）：来源在此期间关闭或替换就不算失败（SDK 1.1.2 及以前先断连接再关来源），否则报 `producer transport closed`；帧中途断开与协议错误立即报告 |
| `src/host/src/composition.rs`、`ui.rs`、`menu.rs`、`ui/*.slint` | 界面组合（主窗口、浮动面板窗口）、编译配置、插件窗口、共享右键菜单 |
| `src/host/src/commands.rs`、`bindings.rs` | 命令注册表与菜单树、输入绑定表（解析、作用范围、冲突、菜单快捷键文字） |
| `src/host/src/plugins.rs` | 插件进程；`VESTRA_PLUGIN_DATA_DIR` 传入数据目录（按 `DataKey`：已安装 `plugin-data/<安装实例 GUID>`，开发加载 `plugin-data/development/<id>`，试载 `plugin-data/<id>.trial.<owner>`，停止时删除）；`secrets` 的服务名同样按 `DataKey`，用过的名字记在安装库的 `secrets/<服务名>.json`（`secret_index.rs`，只记名字，每服务 ≤ 256），带数据卸载与休眠实例被丢弃时据此删除 |
| `src/host/src/manifest.rs` | manifest 结构与规则表（开发加载与托管包共用；SDK CLI 用同一张表，`host-agreement` 测试比对） |
| `src/host/src/package_store.rs`、`package_archive.rs`、`package_manager.rs` | 安装库（`packages/catalog.json`：每个 id 的当前安装实例与休眠实例）、`.vxp` 解压、安装/替换/回退/卸载与内置插件播种（`Resources/plugins/builtin.json`）。设计见 [S4 提案](store-api-proposal.md) |
| `src/host/src/ui_markdown.rs`、`ui/vestra/markdown.slint` | `plugin.ui.markdown`：pulldown-cmark 解析成 `[MarkdownBlock]`，行内样式用 Slint `styled-text` |
| `src/host/src/window_state.rs`、`ui_state.rs` | 底栏/侧栏显示规则（宽度、空闲隐藏、分离）、界面状态持久化 |
| `src/host/ui/vestra/` | 插件可导入的主题与控件（`@vestra/theme.slint`、`@vestra/widgets.slint`） |
| `src/runtime/runner.ts` | 插件运行器 |
| `src/node_modules/@vestra-player/sdk` | npm 包 `@vestra-player/sdk`，源码在独立仓库 [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk)（`src/plugin/` 插件 API，公共类型 `types.ts`、测试宿主 `testing.ts`；`src/*.ts` wire 客户端（内部）；`bin/vestra-plugin.mjs` 插件命令行工具；`slint/` 控件库只读副本；`test/` SDK 单元测试），版本由 `src/package.json` 锁定。入口与发布格式见 [生态计划 S0 记录](../product/ecosystem-plan.md)，改动流程见 [工作流 §1.6](../process/community-workflow.md) |
| `src/scripts/plugin.mjs`、`check-boundaries.mjs` | 仓库内调用 `vestra-plugin` 的快捷方式（按插件名）、导入边界检查 |
| `src/scripts/stage-plugins.mjs` | 按打包布局生成 `Resources/plugins/`：`src/plugins/` 每个插件的 `.vxp` 与 `builtin.json` |
| `src/scripts/sync-sdk-slint.mjs`、`stage-runtime.mjs` | 校验已安装 SDK 的控件库副本与 `host/ui` 一致（`--write <SDK 仓库>` 把 `host/ui` 复制进 SDK 仓库）；从已安装的 SDK 按打包布局生成 `Resources/runtime` |
| `src/tests/sdk/` | 播放器侧测试：runner、打包布局、已安装 SDK 的常量（能力表、`apiVersion`、命令上限）与控件库副本和宿主一致 |
| `src/plugins/` | 内置产品插件（`youtube`、`bilibili` 在插件仓库 [vestra-player-plugins](https://github.com/vestrasys/vestra-player-plugins)） |
| `src/examples/bookmark/` | 教程示例插件 |
| `src/examples/core-ui/`、`src/examples/panel-menu/` | 核心界面接口的演示插件（菜单、绑定、屏幕提示、播放序列、侧栏状态与分离、面板右键菜单、主题控件） |

## 6. 界面状态文件

`$VESTRA_UI_STATE`，默认 `~/Library/Application Support/Vestra/ui-state.json`；无界面模式不读不写。内容：主窗口位置与尺寸、侧栏宽度、按 manifest id 记录的侧栏面板显示/分离状态与浮动窗口位置、用户绑定覆盖层（`"<manifest id>/<命令 id>": [绑定…]`，`[]` 为解绑；本轮只读）。变化后 1 秒合并写入（临时文件 + 改名），退出时再写一次；读不了的文件改名为 `ui-state.json.bad` 后用默认值。格式见 [core-ui-design.md §8](core-ui-design.md#8-c8-界面状态持久化)。

## 7. 文件路径与持久化（跨平台）

- 规范化路径一律用 `dunce::canonicalize`（`src/host/clippy.toml` 禁用 std 的 `canonicalize`）。Windows 上 std 返回 `\\?\C:\…` 形式，Slint 的 `import` 找不到这种路径，mpv、FFmpeg 也不一定认；dunce 在普通路径指同一个文件时去掉前缀，其它系统与 std 相同。去不掉时（超长路径、保留名）`composition::import_path` 直接报错。包内路径的「在包内」检查两边都用 dunce，结果一致；形式不一致时 `starts_with` 失败，是拒绝而不是放行。
- 包内相对路径（manifest 字段、`.vxp` 条目名、目录捕获后的文件名）在所有系统上都用 `/`，同一套规则（`manifest::relative_path`）。`.vxp` 条目名里的 `\` 拒绝（zip 规范要求 `/`），不当作分隔符；目录捕获（`package_stage::relative_name`）按路径组件拼 `/`，所以 Windows 的 `ui\a.slint` 与 macOS 的 `ui/a.slint` 摘要相同；Unix 文件名里的 `\` 拒绝（在 Windows 上它是分隔符）。
- 目录持久化屏障：`platform::sync_dir`。安装库每次原子替换（`tempfile` persist、`fs::rename`）后调用。Unix 对目录 `fsync`。Windows：重命名（`MoveFileExW`/`SetFileInformationByHandle`，不带 `MOVEFILE_WRITE_THROUGH`）在 NTFS 上是原子的（元数据有日志），但日志落盘前崩溃可能回滚，所以对目录句柄 `FlushFileBuffers`：目录只能以 `FILE_FLAG_BACKUP_SEMANTICS` 打开，且刷新需要写权限，只读的 `File::open` 会报 Access denied。只用 std 的 `OpenOptionsExt::custom_flags`，不需要 `unsafe`。两个系统都把错误返回给调用方（catalog 已发布，持久性未知，调用方查询/恢复而不是重做）。
