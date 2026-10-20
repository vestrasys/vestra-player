# 插件 API：SDK v2 on `vestra.experimental/0.1`

状态：SDK v2 是插件唯一的 API（2026-10-20 P4 起；旧扁平 `Host` API 已删除）。设计说明见 [sdk-v2.md](sdk-v2.md)，示例见 [sdk-v2-usage.md](sdk-v2-usage.md)，公共类型在 `src/sdk/plugin/types.ts`，入口 `src/sdk/plugin/index.ts`，运行器 `src/runtime/runner.ts`，宿主路由 `src/host/src/main.rs`。wire 协议版本不变：`vestra.experimental/0.1`。

## 包与生命周期

```json
{"id":"community.sample","version":"1.0.0","apiVersion":"vestra.experimental/0.1","entry":"main.ts","capabilities":["commands","ui.window"]}
```

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("sample.slint");
  plugin.commands.add({ id: "sample.open", path: ["插件", "示例…"], run: () => window.show() });
}
```

- manifest 拒绝未知字段、错误版本、未知 capability、空/非法 ID；`entry` 与 UI 文件必须是相对包路径，规范化后仍在包目录内（含符号链接）。这是包完整性约束，不是安全沙箱：插件可信，可使用 Node、文件系统、网络与 npm 依赖。`version` 为生产包版本（三段 u32），由构建工具写入 `dist/manifest.json`。
- 入口必须**默认导出** `activate(plugin)`。命名导出 `activate(host)` 的旧插件会以 `runtime.failed` 拒绝。插件代码只能 `import type` SDK；运行器提供插件对象，生产包没有对本仓库的运行时依赖（构建工具检查）。
- 宿主每加载一个包启动一个 Node 进程（`runtime/runner.ts`）。运行器读取来源状态（有 `playback.read` 时）、订阅需要的事件、调用 `activate`，等待激活期间发起的注册（命令、面板、解析器、订阅）全部成功后发送 `runtime.ready`；`activate` 抛错或任一注册失败则发送 `runtime.failed`。
- 停用：宿主发送 `runtime.stop`，按 owner 撤销该插件的全部宿主资源（菜单、窗口、面板、处理资源、字幕资源、快照、任务、解析器、producer）；运行器拒绝新请求、使挂起请求以 `disabled` 失败、释放插件的全部句柄（只做本地清理），300 ms 后仍未退出则被结束。插件不返回清理函数。每次运行拥有新的 owner 代次，旧进程的回包/回调不会路由给重新加载的同名包。
- 请求没有客户端超时：管道有序，宿主对每个请求都答复或停止插件；超过 10 秒的请求只写日志（[sdk-v2.md §5.1](sdk-v2.md#51-结果不确定的处理与依据)）。

## 能力与模块

请求所需 capability 必须在 manifest 中声明（契约诊断，不是防越权边界）。

| capability | SDK v2 模块 |
| --- | --- |
| commands | `plugin.commands.add` |
| ui.window | `plugin.windows.create` |
| ui.mount | `plugin.panels.mount("bottom" \| "sidebar", …)` |
| ui.dialog | `plugin.dialogs.openFile/openFiles/openDirectory/saveFile` |
| playback.read | `plugin.player.state`（初读）、`refresh`；滤镜读取处理状态 |
| events | 实时 `player.state` / `onChange`、面板可见性、字幕刷新 |
| playback.control | `player.open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput` |
| producer.av | `player.openStream` |
| processing.video / processing.audio | `plugin.video.filter` / `plugin.audio.filter`（另需 playback.read） |
| subtitles.read / subtitles.control | `plugin.subtitles.*` |
| media.read / media.control | `plugin.media.capture` → `Snapshot.export` → `ExportJob` |
| source.resolve | `plugin.resolvers.add` |
| plugins.manage | `plugin.packages.validate` → `trial` / `install`；`packages.rollback` |

宿主仍实现但 SDK v2 不提供给插件的 wire 方法：`host.capabilities`、`playback.state/open/seek`、`playback.set` 中除 `volume`/`muted` 外的属性、`window.state/setFullscreen`、`commands.snapshot/execute`、`plugins.list/load/disable/activation`、`processing.*` 与 `subtitles.*` 的原始操作查询，以及事件 `playback.state`、`plugins.changed`、`host.error`。它们供宿主自测与历史验证使用；若插件确有需要，按常规流程提 API 请求。

## Slint

- 工具窗口：`.slint` 必须恰好导出一个 Window 组件。属性与回调在 TS 中用 camelCase，SDK 转换为 kebab-case；窗口回调须在首次 `show()` 前绑定。
- 面板：文件导出一个 component 与一个 global（桥接属性与回调都在 global 上），宿主按 owner 别名导入；组件名与 global 名由 SDK 从文件推断。
- `bind` 的值只在变化时发送，`set` 的值总会发送一次；用户修改的 in-out 属性在其它回调后保留（[sdk-v2.md §4](sdk-v2.md#4-状态同步值绑定与更新轮)）。
- 回调参数与属性值限于 JSON 可表示的基本类型、数组与结构体（有界）；回调必须返回 void。

## 错误

失败以带稳定 `code` 的 `Error` 表示（`PluginError`）：宿主错误码原样保留，SDK 自己的码见 [sdk-v2.md §7](sdk-v2.md#7-错误模型)。处理函数中的错误交给 `scope.onError`。

## 工具

`npm run plugin -- check|build|test <name>`、`npm run plugins:build`、`npm run test:sdk`；插件测试用 `src/sdk/plugin/testing.ts` 的 `startPlugin` / `TestHost`。
