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
│ SDK  @vestra-player/sdk（src/packages/sdk/）：plugin/ 句柄式 API → wire 客户端（内部） │
│ 插件 src/plugins/<名字>/：main.ts + .slint           │
└────────────────────────────────────────────────────┘
```

- **Rust 宿主**只提供公共设施：播放、界面组合、处理链、字幕、导出、插件生命周期。不认插件身份，不含插件业务（站点解析、存档、预设等都在插件里）。第一方代码禁止 `unsafe`。
- **TypeScript 插件**是业务的唯一位置。插件之间完全独立，只通过公开宿主状态协作。
- **Slint** 是插件界面的唯一形式：工具窗口（独立窗口）或挂载到底栏/侧栏的面板，由宿主编译和组合。

## 2. 插件的加载与停止

- **开发加载**：`vestra-host --plugin <目录>`（可多次），直接运行入口 `.ts`（Node ≥ 22.18 去类型执行）。
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

- 编译：宿主、面板、工具窗口、包校验共用一个编译配置（`ui.rs` 的 `compiler()`）：库路径 `vestra` → `ui/vestra/`（`@vestra/theme.slint`、`@vestra/widgets.slint`），风格 `fluent-dark`。默认字体是系统自带的 Hiragino Sans GB（按文件导入并设为各窗口的默认字体）：GUI 实测（2026-10-20）不指定字体时插件窗口的中文显示为空白，Slint 1.16 在这里没有按文字回退。`VESTRA_FONT_FALLBACK=system` 可关闭，用于检验以后 Slint 版本自身的回退。主题的 `font-mono`（Menlo）只含西文，含中文的文字不要用它。
- 工具窗口：插件 `.slint` 文件恰好导出一个组件；宿主用 slint-interpreter 编译，按 SDK 登记的回调白名单转发回调。
- 面板：文件导出一个 component 和一个 global（属性与回调的桥接对象）；宿主按 owner 别名把它导入主界面（`host.slint` 的插入标记），转发 global 上的全部回调。主窗口外壳：视频列（视频 + 底栏）在左，3px 分隔条与全高侧栏在右；全屏时底栏覆盖在视频底部并按 `window_state::Policy` 的空闲规则隐藏（指针停在底栏上时不隐藏），附着的侧栏变成右边缘的弹出层。
- 浮动面板（C14）：分离时宿主另外编译一个窗口，导入同一个组件与桥接 global；global 每个窗口实例一份，所以宿主用保存的 `Mount.properties` 初始化浮动窗口，之后每次 `ui.setMount` 同时写主窗口与浮动窗口，两边回调都以 `$mount` 转发。附着时只关闭浮动窗口，主窗口不重建。
- 输入：主窗口最外层、两个面板各包一层 FocusScope。Slint 的按键先给焦点控件，未接受的逐级冒泡；面板那一层以「面板 + 全局」查表，最外层以全局查表（`bindings.rs`）。查表在 UI 线程上同步进行（`Rc<RefCell<InputShared>>`，不借用 Host），绑定了就返回 accept 并发 `Event::Binding`，主循环再按注册表检查启用状态后执行。视频区鼠标（双击、中键、侧键、滚轮）同理。全屏、退出全屏、退出是宿主自己的命令（owner 0），和插件命令在同一张表与同一棵菜单树里。
- 菜单：`commands.rs` 按 `order`（子菜单取其最前的命令）建树，`menu.rs` 投影为面板列表（分隔线、单选、快捷键文字、行位置），`menu-view.slint` 只负责绘制。面板右键菜单（C11）复用同一投影的单层形式，弹在面板当前所在的窗口。
- 模拟器输入捕获（`producer.declareInput`）在 winit 层先拦截按键；绑定在宿主命令上的键（全屏等）放行。

## 5. 代码地图

| 路径 | 内容 |
| --- | --- |
| `src/host/src/main.rs` | 宿主主循环、RPC 路由与能力检查、事件广播 |
| `src/host/src/plugins.rs`、`package_*.rs` | 插件进程、manifest 校验、托管包存储/试载/安装/回退 |
| `src/host/src/backend.rs`、`source_*.rs`、`session.rs` | libmpv 后端、来源控制与操作簿、网络来源解析 |
| `src/host/src/processing*.rs`、`effects.rs` | 视频/音频处理链 |
| `src/host/src/subtitle_*.rs`、`native_subtitle.rs` | 字幕上传、选择、样式层 |
| `src/host/src/media_*.rs`、`offline_*.rs` | 快照与离线导出工作进程 |
| `src/host/src/producer*.rs` | 插件生成的音视频流（NES） |
| `src/host/src/composition.rs`、`ui.rs`、`menu.rs`、`ui/*.slint` | 界面组合（主窗口、浮动面板窗口）、编译配置、插件窗口、共享右键菜单 |
| `src/host/src/commands.rs`、`bindings.rs` | 命令注册表与菜单树、输入绑定表（解析、作用范围、冲突、菜单快捷键文字） |
| `src/host/src/plugins.rs` | 插件进程；`VESTRA_PLUGIN_DATA_DIR` 传入每插件数据目录（`plugin-data/<manifest id>`，试载的停止时删除） |
| `src/host/src/window_state.rs`、`ui_state.rs` | 底栏/侧栏显示规则（宽度、空闲隐藏、分离）、界面状态持久化 |
| `src/host/ui/vestra/` | 插件可导入的主题与控件（`@vestra/theme.slint`、`@vestra/widgets.slint`） |
| `src/runtime/runner.ts` | 插件运行器 |
| `src/packages/sdk/` | npm 包 `@vestra-player/sdk`（workspace）：`src/plugin/` 插件 API（公共类型 `types.ts`，测试宿主 `testing.ts`），`src/*.ts` wire 客户端（内部），`bin/vestra-plugin.mjs` 插件命令行工具，`slint/` 控件库只读副本，`test/` SDK 单元测试。入口与发布格式见 [生态计划 S0 记录](../product/ecosystem-plan.md) |
| `src/scripts/plugin.mjs`、`check-boundaries.mjs` | 仓库内调用 `vestra-plugin` 的快捷方式（按插件名）、导入边界检查 |
| `src/scripts/sync-sdk-slint.mjs`、`stage-runtime.mjs` | 把 `host/ui` 的控件库同步进 SDK 包；按打包布局生成 `Resources/runtime` |
| `src/tests/sdk/` | 播放器侧测试：runner、SDK 常量与宿主一致、控件库副本一致 |
| `src/plugins/` | 10 个产品插件 |
| `src/examples/bookmark/` | 教程示例插件 |
| `src/examples/core-ui/`、`src/examples/panel-menu/` | 核心界面接口的演示插件（菜单、绑定、屏幕提示、播放序列、侧栏状态与分离、面板右键菜单、主题控件） |

## 6. 界面状态文件

`$VESTRA_UI_STATE`，默认 `~/Library/Application Support/Vestra/ui-state.json`；无界面模式不读不写。内容：主窗口位置与尺寸、侧栏宽度、按 manifest id 记录的侧栏面板显示/分离状态与浮动窗口位置、用户绑定覆盖层（`"<manifest id>/<命令 id>": [绑定…]`，`[]` 为解绑；本轮只读）。变化后 1 秒合并写入（临时文件 + 改名），退出时再写一次；读不了的文件改名为 `ui-state.json.bad` 后用默认值。格式见 [core-ui-design.md §8](core-ui-design.md#8-c8-界面状态持久化)。
