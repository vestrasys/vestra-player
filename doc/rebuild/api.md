# 初版公共 API：vestra.experimental/0.1

> 本页保留阶段A范围，不能作为当前完整SDK清单。后续新增能力与待发布状态见[开发工作基线](development-baseline.md)，实际方法以当前SDK为准。

状态：阶段 A 工作版，供交互设计评估；没有发布为全部插件并行开发的稳定基线。SDK 位于 `src/sdk/index.ts`，宿主路由在 `src/host/src/main.rs`；这份文档只描述已实现能力。

## 包与生命周期

```json
{"id":"community.sample","apiVersion":"vestra.experimental/0.1","entry":"main.ts","capabilities":["playback.read","playback.control","events","ui.window"]}
```

manifest 拒绝未知字段、错误版本、未知 capability、空/非法 ID；entry 与 UI 文件必须是相对包路径，canonicalize 后仍在包目录内（包括符号链接）。重复运行 ID 被拒绝。此检查是包完整性约束，不是安全沙箱。插件可信，可使用 Node/fs/网络与成熟依赖；生产分发可带已构建 JS，用户不需要 Rust 编译器。

```ts
import type { Host } from "../../sdk/index.ts";
export async function activate(host: Host) {
  const caps = await host.capabilities();
  const off = await host.events.subscribe("playback.state", state => console.error(state));
  // 文件选择属于随后设计的产品插件；此处仅演示公开播放设施。
  await host.playback.open("/absolute/path/movie.mp4");
  await host.playback.set("pause", false);
  return async () => { /* 释放插件自己的 Node 资源 */ };
}
```

宿主每加载一个包启动一个 Node 进程。activate 可返回 dispose。加载返回 manifest 代表进程已启动；不等于 activate 已完成，也不是产品安装/升级成功。runtime.ready / runtime.failed 当前写宿主诊断，尚无稳定的激活状态公共模型。若在 activate 尚未返回时停用，进程会终止，尚未返回的 dispose 无法调用；插件应在激活期间自行约束外部资源副作用。正常退出、协议错误、停用都会释放该实例的窗口与订阅；每次运行拥有新的 generation，旧进程回包/回调不能路由给重新加载后的同名包。

停用从运行列表中移除并在后台请求 dispose，300ms 后仍未退出则 kill + wait。运行时停止后拒绝新 RPC、使未完成 Promise 以 disabled 失败；dispose 应只释放插件自有资源，不能依赖新的宿主请求。宿主窗口/订阅清理由 Rust 保证，不能依赖插件配合。全局播放设置和媒体会话不因停用而复位；可撤销处理资源还未设计。

## 能力与方法

| capability | 已实现方法与行为 |
|---|---|
| 无需声明 | `capabilities()`：契约版本、公开能力、backend 名、UI 是否可用 |
| playback.read | `playback.state()`：最近约100ms快照；无媒体的 position/duration 为 null |
| playback.control | `open(source)`、`seek(seconds)`、`set(property,value)`，走同一长期 libmpv |
| events | 订阅 playback.state、playback.backend、plugins.changed、host.error；返回 async unsubscribe |
| ui.window | `ui.open/set/show/close` 和 `onCallback`；独立 Slint 原生窗口 |
| plugins.manage | `plugins.list/load(directory)/disable(id)`；运行生命周期，尚非持久安装库 |

capabilities 列出宿主支持的能力，不代表每个插件都已获声明。请求方法须由该插件 manifest 声明对应能力；这是契约诊断，不是防恶意越权的安全边界。

播放状态包含 `session:number`、`source:string|null`、`paused:boolean`、`position:number|null`、`duration:number|null`、`volume?:number`、`rate?:number`、`videoFrames:number`。session 从0开始，每个接受的 open 增长，调参/seek/pause 不增长。videoFrames 是宿主软件 renderer 完成帧的累计计数，不能当作屏幕实际呈现帧数。多个消费者共享该状态。

open Promise 在 mpv 接受 loadfile 后返回 session。文件实际读取/解码成功、失败是异步结果；初版通过 playback.backend 提供 mpv 事件诊断字符串（currentSession 只是 poll 时的当前会话，不是事件来源关联；连续 open 时旧事件可能在新会话期间排出，不能用它完成异步结果隔离）（`event` 的 Debug 文本不是稳定解析协议），阶段 B/C 需要更明确的产品 loading/error 状态。state 可能仍有上一轮约100ms快照，调用方不能立即把旧快照当新会话结果。seek 使用非负秒数，mpv absolute+exact。它是后端接受语义，不能当帧已显示的 fence。

| set property | 类型/范围 | 单位 |
|---|---|---|
| pause | boolean | 暂停 |
| volume | 0..200 | mpv百分比 |
| speed | 0.05..10 | 倍速 |
| brightness / contrast / saturation / gamma | -100..100 | mpv视频参数 |
| audio-delay / sub-delay | -60..60 | 秒 |

set 在 mpv 同步设置完成后答复，视频实际显示的生效时刻仍由 renderer 决定。数值须有限，范围外为 invalid。没有按插件名/探针名分支，也没有直接给 TS 任意 mpv 命令入口。连续提交不重载文件；控制线程与渲染线程分离，不能将同步普通 mpv 调用和 render 放回同一等待链。

## Slint 接入例子

包内 `sample.slint` 必须恰好导出一个 Window 组件（多export会被拒绝），使用 Slint 自有 widgets/layout。示例 `src/examples/capability-demo` 是完整可运行消费端；只演示 API，不形成产品布局约束。

```ts
await host.ui.open("sample", "sample.slint", { status: "Ready" }, ["toggle", "adjust"]);
const off = host.ui.onCallback(({view, callback, args}) => {
  if (view === "sample" && callback === "adjust") {
    void host.playback.set("brightness", Number(args[0])).catch(console.error);
  }
});
await host.ui.set("sample", "status", "Playing");
await host.ui.show("sample", false); // 隐藏；组件仍存活
await host.ui.show("sample", true);
await host.ui.close("sample");       // 释放组件；之后可同 ID 重新 open
```

properties 支持 string/number/boolean；callbacks 为返回 void、参数为这些 primitive 的导出回调。其他参数目前不能视为可用；复杂 model、image、嵌入 slot、菜单、快捷键和关闭通知尚未暴露。view ID 在插件实例内唯一，其他插件不能寻址它；停用时释放。用户关闭工具窗口仅隐藏 Slint 窗口，插件进程和播放会话继续，当前缺少公共关闭通知。不要把工具关闭当作卸载。

首次编译 .slint 在 UI 线程发生，可能短暂停帧；set/show 不重新编译，也不暂停/seek。该限制需在产品设计阶段处理，尚不能宣布“打开所有工具无卡顿”通过。

## 通信、错误与取消

Node stdout 专用于 UTF-8 JSONL，console.log/info/debug 重定向 stderr。请求 `{id,method,params}`，答复 `{id,result}` 或 `{id,error:{code,message}}`；事件 `{event,data}`。包应通过 SDK 使用，避免耦合私有 runtime 诊断。宿主出站队列每实例最多64条，独立writer线程写管道；队列满时停用该实例，UI不等待其stdin。单行最大1MiB；无效JSON/结构导致实例停用。不要把大图像/音频样本塞入这条控制信道。

HostError.code：invalid（参数）、unsupported（未暴露的方法/属性）、capability（manifest缺能力）、unavailable（headless UI）、missing（不存在的资源）、conflict（view重复）、backend（mpv失败）、host/ui（宿主/Slint错误）；客户端另有 timeout/disabled。错误包含 message。10秒 timeout 表示结果未知，不能推定后端操作已撤销。

本版没有长任务和 AbortSignal/任务取消 API。已接受的瞬时播放操作不支持回滚式取消；停用插件会结束其待答请求和拥有的 UI/订阅，不能撤销已经提交的全局播放改变。导出/翻译/生成任务须后续另设任务所有权、取消和会话隔离，不用 timeout 冒充取消。

SDK 对同一事件的多个本地监听者计数，只在最后一个 unsubscribe 后解除宿主订阅。订阅无历史重放，初始状态用 playback.state 主动读取。
