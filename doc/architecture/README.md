# 架构总览（维护者）

给 API 维护者的地图：组件怎么分工、插件怎么被加载和停止、SDK 与宿主之间说什么协议、代码在哪。插件作者看 [doc/plugins/](../plugins/README.md)；SDK 的设计取舍见 [sdk-design.md](sdk-design.md)；项目原则见 [doc/product/](../product/direction.md)。

## 1. 组件

```text
┌──────────── 宿主进程（Rust，src/host）────────────┐
│ Slint 原生界面：主窗口、底栏/侧栏挂载、工具窗口、菜单 │
│ libmpv 播放后端、处理链、字幕、快照/导出工作进程      │
│ 插件管理：加载、停用、试载、安装/回退、owner 资源回收  │
└───────────────▲─────────────────────────────────┘
                │ JSONL over stdin/stdout（wire：vestra.experimental/0.1）
┌───────────────┴──── 每插件一个 Node 进程 ──────────┐
│ 运行器 src/runtime/runner.ts                        │
│ SDK  src/sdk/plugin/（句柄式 API）→ src/sdk/*.ts（wire 客户端，内部） │
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
- **停用**：宿主按 owner 撤销该插件的全部资源（菜单、窗口、面板、处理资源、字幕层、快照、任务、解析器、producer），发送 `runtime.stop`；运行器释放句柄，300 ms 内未退出则被结束。每次运行有新的 owner 代次，旧进程的迟到消息不会路由给重新加载的同名插件。

manifest 规则见 [API 参考 §2](../plugins/reference.md#2-manifest-与能力)。能力是契约声明，宿主据此拒绝未声明的调用（`code: "capability"`），不是安全沙箱。

## 3. wire 协议

- 每行一个 JSON：请求 `{ id, method, params }`，答复 `{ id, result }` 或 `{ id, error: { code, message } }`，事件 `{ event, data }`。
- 管道有序；宿主对每个请求都答复，或者停止该插件。所以运行器不设客户端超时（超过 10 秒只写日志）。
- 协议细节（修订号、来源代次、操作 id、事件序号等）全部由 SDK 吸收，插件看不到，见 [sdk-design.md §5](sdk-design.md#5-协议细节如何被吸收)。
- wire 方法只服务于 SDK。SDK 不再调用的方法应从宿主删除，不为假想的调用方保留（2026-10-20 已删除旧接口：`host.capabilities`、`playback.state/open/seek`、`playback.set` 中除音量/静音外的属性、`window.state`（RPC）/`setFullscreen`、`commands.snapshot/execute`（RPC）、`plugins.list/load/activation/disable`、`jobs.list`、`subtitles.upload.status`，以及 `playback.state`、`plugins.changed`、`host.error` 事件）。
- 插件可订阅的事件只有 `source.state`、`playback.backend`、`window.state`。
- `producer.declareInput` 与物理输入子系统保留：SDK 尚未暴露键盘输入，但这是模拟器键盘输入的现成能力。

## 4. Slint 组合

- 工具窗口：插件 `.slint` 文件恰好导出一个组件；宿主用 slint-interpreter 编译，按 SDK 登记的回调白名单转发回调。
- 面板：文件导出一个 component 和一个 global（属性与回调的桥接对象）；宿主按 owner 别名把它导入主界面的底栏或侧栏，转发 global 上的全部回调。
- 插件界面默认使用宿主内置的 CJK 字体，保证中文显示。

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
| `src/host/src/composition.rs`、`ui.rs`、`menu.rs`、`ui/*.slint` | 界面组合、插件窗口编译、共享右键菜单 |
| `src/runtime/runner.ts` | 插件运行器 |
| `src/sdk/plugin/` | 插件 API（公共类型 `types.ts`，测试宿主 `testing.ts`） |
| `src/sdk/*.ts` | wire 客户端（SDK 内部使用） |
| `src/scripts/plugin.mjs`、`check-boundaries.mjs` | 插件构建工具、导入边界检查 |
| `src/tests/sdk/` | SDK 单元测试 |
| `src/plugins/` | 10 个产品插件 |
| `src/examples/bookmark/` | 教程示例插件 |
