# SDK v2 设计说明

状态：P1 交付（2026-10-20，API 维护者）。任务依据见 [sdk-v2-migration.md](sdk-v2-migration.md)，示例见 [sdk-v2-usage.md](sdk-v2-usage.md)，迁移步骤见 [sdk-v2-migration-guide.md](sdk-v2-migration-guide.md)。

代码：`src/sdk/plugin/`（公共类型在 `types.ts`，入口 `index.ts`），测试宿主 `src/sdk/plugin/testing.ts`，单元测试 `src/tests/sdk/`，运行器 `src/runtime/runner.ts`，构建工具 `src/scripts/plugin.mjs`。参考迁移：`src/plugins/video/`。

## 1. 一句话

插件默认导出 `activate(plugin)`。`plugin` 上按领域分组的工厂创建**句柄**（命令、窗口、面板、滤镜、媒体、快照、任务……），句柄自带回调、自带最后写入生效的状态同步、自动随作用域释放。插件代码只描述“想要什么”，协议字段全部留在 SDK 里。

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const filter = plugin.video.filter("picture");
  const window = plugin.windows.create("picture.slint");
  window.on("adjust", (field: string, value: number) => filter.set(nodesFor(field, value))); // nodesFor：插件自己的映射
  plugin.commands.add({ id: "video.picture", path: ["视频", "画面调节…"], run: () => window.show() });
}
```

## 2. 模块与句柄一览

`Plugin` 是根作用域；`Scope` 与它有同样的模块（子作用域创建的句柄只属于子作用域）。

| 模块 | 工厂 / 成员 | 句柄 | 句柄上的操作 | 使用的插件 |
| --- | --- | --- | --- | --- |
| `commands` | `add({ id, path, title?, shortcut?, enabled?, checked?, busy?, reason?, run })` | `Command` | `set(state)`、`ready` | 全部 |
| `windows` | `create(file, { id? })` | `Window` | `on`、`set`、`bind`、`show`、`hide`、`close`、`opened` | video、audio、subtitles、capture、nes、bilibili、youtube、workshop、playlist |
| `panels` | `mount("bottom" \| "sidebar", file, { component?, global? })` | `Panel` | `on`、`set`、`bind`、`visibility`、`onVisibility`、`setVisible`、`toggle`、`setInteraction`、`ready` | playback、playlist |
| `dialogs` | `openFile`、`openFiles`、`openDirectory`、`saveFile` | —（返回路径或 `null`） | | playback、playlist、subtitles、capture、nes、youtube、workshop |
| `player` | `state`、`onChange`、`refresh`、`isCurrent`、`open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput`、`openStream` | `Media`、`Scrub`、`Stream` | Media：`current`、`ready`、`onEnd`、`close`；Scrub：`seek`、`active`、`end`；Stream：`running`、`onStop`、`stop` | playback、playlist、audio、capture、nes、bilibili、youtube |
| `video` / `audio` | `filter(id)` | `VideoFilter` / `AudioFilter` | `set`、`status`、`onStatus`、`retry` | video、audio |
| `subtitles` | `state`、`onChange`、`refresh`、`select`、`load`、`remove`、`setStyle` | `SubtitleTrack`（值对象） | | subtitles |
| `media` | `capture({ at?, processed?, subtitles?, audio?, ifCurrent? })` | `Snapshot` → `ExportJob` | Snapshot：`export`、只读描述；Job：`state`、`onChange`、`done`、`cancel`、`refresh` | capture |
| `resolvers` | `add({ id, schemes, hosts, resolve })` | `Resolver` | `ready` | bilibili、youtube |
| `packages` | `validate(dir)`、`rollback(id)` | `PackageCandidate` → `PackageTrial` / `PackageChange` | Candidate：`trial`、`install`；Trial：`status`、`stop`；Change：`status`、`message`、`onChange`、`done`、`cancel` | workshop |
| 作用域 | `scope()`、`add`、`onDispose`、`onError`、`update`、`dispose` | `Scope` | | 全部（`onError`、`update` 按需） |

模块需要的 manifest 能力（宿主按能力授权，SDK 不改变这一点）：`commands` → commands；`windows` → ui.window；`panels` → ui.mount；`dialogs` → ui.dialog；`player.state` 初读与 `refresh` → playback.read，实时更新 → events；`player` 的控制与 `open` → playback.control；`openStream` → producer.av；`video`/`audio` 滤镜 → processing.video / processing.audio 加 playback.read（读处理状态）；`subtitles` → subtitles.read / subtitles.control；`media` → media.read、media.control（`processed` 截图另需 playback.read、subtitles.read）；`resolvers` → source.resolve；`packages` → plugins.manage。没有 events 能力时，`media.ready()` 自动轮询来源状态。

命名取舍：用户可见的领域名词（`player`、`filter`、`panel`、`stream`、`snapshot`、`job`、`resolver`），宿主内部名（`processing`、`producer`、`source`、`mount`、`$mount`）不出现在插件 API 中。没有 `ctx`：插件拿到的对象就叫 `plugin`，一个功能模块拿到的是 `Scope` 或具体句柄。

## 3. 生命周期与释放

1. **激活**：运行器载入入口模块。默认导出函数的模块走 SDK v2：运行器先创建插件对象（有 `playback.read` 时先读一次来源状态，有 `events` 时订阅 `source.state`），调用 `activate(plugin)`，再等待激活期间发起的全部注册（命令、面板挂载、解析器、事件订阅）完成，最后才报告 `runtime.ready`。任一注册失败 → 激活失败（`runtime.failed`），与旧插件“`activate` 中 `await register` 抛错”的效果相同，但插件代码里不需要 `await`。
2. **声明即注册**：`commands.add`、`panels.mount`、`resolvers.add` 同步返回句柄，注册在后台排队进行；需要时可 `await handle.ready`。激活后创建的句柄，注册失败交给所在作用域的 `onError`。
3. **作用域**：每个句柄在创建时加入当前作用域。`scope.dispose()` 按创建的逆序释放其中所有句柄和子作用域；直接 `handle.dispose()` 会把它从作用域移除。重复释放无害，释放从不抛错（失败只写日志）。
4. **插件停用**：宿主发送 `runtime.stop` → 运行器拒绝新请求并释放根作用域。此时宿主已经按 owner 撤销了该插件的全部资源（菜单、窗口、面板、处理资源、字幕资源、快照、任务、解析器、producer），因此释放只做本地清理（停计时器、中止信号、关闭 producer 套接字），对宿主的请求被立即拒绝并忽略。插件**不需要**返回清理函数。
5. **各句柄释放时做什么**（运行期间释放子作用域时生效）：

| 句柄 | 释放 |
| --- | --- |
| Command | 不再分发；宿主没有注销 RPC，命令保持在菜单中但置为禁用（reason “已停用”） |
| Window | `ui.close` |
| Panel | `ui.unmount` |
| VideoFilter / AudioFilter | 若曾应用过，把本资源替换为空链（只撤销自己的贡献） |
| Snapshot | `snapshot.release` |
| ExportJob | 未结束则 `jobs.cancel` |
| Stream | 停止渲染，若仍是当前来源则关闭，释放 producer |
| Resolver | `sourceResolvers.unregister`，中止进行中的解析 |
| PackageTrial | `stopTrial` 并等待清理确认 |
| 监听（`onChange`、`onEnd`、`on`、`bind`…） | 退订 |

`Media` 不随作用域关闭来源：插件停用时当前播放继续（播放列表的既有行为）。需要关闭时显式 `media.close()`。

## 4. 状态同步：值、绑定与更新轮

连续变化的值只描述期望状态，SDK 负责合并、去重和顺序下发：

- `window.set(props)` / `panel.set(props)`：立即写入期望值。
- `window.bind(() => props)` / `panel.bind(...)`、命令选项里的函数（`enabled: () => ...`）：**绑定**，在每个更新轮重新求值。
- **更新轮**：SDK 在以下时机自动安排一轮（同一微任务内多次请求只跑一轮）：SDK 调用的任何插件处理函数（命令 `run`、窗口/面板回调、`onChange`、`onStatus`、`onEnd`、`onStop` 等）同步返回后与其 promise 结束后；播放器、字幕、面板可见性、滤镜状态、导出任务状态变化后。插件在自己的计时器里或长任务的中途改了状态时，调用 `plugin.update()`。
- 每个目标同一时间只有一个请求在途，期间的变化合并为下一次发送；只发送与宿主当前值不同的属性。窗口/面板收到回调后，SDK 认为用户可能改动了 `in-out` 属性（如滑块），下一轮重发全部属性，宿主侧对相同值不重绘。
- 窗口在第一次 `show()` 时用当前期望属性作为初始属性打开，不会先闪默认值；`close()` 后再 `show()` 会用最新属性重开。
- 名称转换：TS 中用 camelCase（`effectEnabled`、`toggleEffect`），SDK 转为 Slint 的 kebab-case；已是 kebab-case 的名字原样使用。结构体字段（如列表行的 `rowId`）不转换。

这是一个刻意很小的“响应式”：没有依赖追踪、没有虚拟 DOM，只有“求值、比较、发送变化”。插件状态就是普通变量。

## 5. 协议细节如何被吸收（逐条对应 4.2）

| 4.2 条目 | SDK 的做法 | 插件看到的 |
| --- | --- | --- |
| `revision` 计数 | 命令：注册用 1，之后每次状态变化递增；处理资源：取 `max(本地, 宿主现值)+1`；字幕样式：本地递增 | 无 |
| `expectedGeneration` / `expected` | 处理资源：每次提交前读 `processing.state`，用当前来源代次；来源控制：默认用插件看到的 `player.state` 的来源戳；打开：默认无条件；字幕：默认用提交前读到的选择戳；快照：默认 `player.state` | 默认无；需要时 `{ ifCurrent: state }`（第 6 节） |
| `operationId` 与 `*.operation(...)` | 每个领域一个递增计数器（来源、字幕、媒体各自的操作簿）；宿主答复即最终结果；只有“答复丢失”才按同一 id 查询（见下） | Promise：完成 / 失败（`code` + 可显示 `message`）/ 进行中（尚未结束） |
| `unknown` / `unknownExpired` | 见下表 | 不出现 |
| `eventSequence` / 代次比较 | `player.state` 只接受序号不小于已投递值的状态；相同内容不重复投递。字幕状态丢弃较旧代次或同代次较旧修订 | `onChange` 只收到更新的状态 |
| UI 名称转换、逐属性 `ui.set`、回调白名单 | 第 4 节；窗口打开时的回调白名单 = 打开前已绑定的处理函数名（打开后再绑定新回调名会抛错，提示在首次 `show()` 前绑定）；面板由宿主转发全局的所有回调，无白名单 | 无 |
| `backendUncertain` | 滤镜提交前检查；该阶段被宿主阻塞时状态为 `failed`，`error.code === "backend-uncertain"`，`message` 为宿主原因；`retry()` 会重新检查 | 统一的 `filter.status` |

### 5.1 “结果不确定”的处理与依据

依据（均为现有宿主行为）：

1. **管道有序**：插件 stdout 的请求由宿主单线程按序读取；处理、来源、字幕、媒体请求再按序进入后台串行工作线程。所以，在某请求之后发出的查询，一定在该请求被接纳或拒绝之后才被处理。
2. **宿主对每个请求都答复**，或者停止该插件进程（此时运行器拒绝全部挂起请求，插件也不再需要结论）。
3. 宿主的拒绝（RPC error，如 `stale`、`busy`、`unsupported`）是**确定**结果：请求未执行，或其失败已被记录。

因此 v2 的运行器**不再给请求设客户端超时**（旧扁平 API 仍保留 10 秒超时，见第 8 节）。超时是此前“结果未知”的主要来源：客户端到点放弃等待，却无法知道宿主是否执行。现在请求超过 10 秒只写一条日志并继续等待真实答复。

SDK 仍然处理“答复丢失”（非宿主错误的拒绝，或客户端 `timeout`），规则如下：

| 领域 | 查询 | 结论 |
| --- | --- | --- |
| 处理资源 | `processing.operation(stage, resourceId, revision)` | `completed` → 已应用；`failed` → 失败；`unknown` → 由 1 可知从未被接纳、肯定未执行，用新修订号重发（连续 3 次则失败 `unknown`）；`unknownExpired` → 读 `processing.state`，本资源当前修订号等于该修订则已应用，否则未生效并重发；`pending` → 100 ms 后再查 |
| 来源 | `source.operation(id)` | `pending` 表示已接纳、仍在执行（例如 URL 解析中），按 40 ms 起、最长 500 ms 的间隔继续查直到 `completed`/`failed`；查询本身被宿主拒绝 → 失败 `unknown` |
| 字幕 | `subtitles.operation(id)` | `completed` → 采用其状态；`failed` → 失败；`unknown` → 失败 `rejected`（未执行）；`unknownExpired` → 刷新状态并失败 `expired` |
| 快照/任务 | `jobs.operation(id)` | `completed` → 采用结果（快照会再读 `snapshot.state`）；`failed`/`unknown`/`unknownExpired` → 失败；`pending` → 250 ms 后再查 |
| 包安装/回退 | `packages.operation(requestId)` | 宿主保证 `operationId === requestId`，持续查询直到终态，从不重发 |
| 试载 | `packages.trialStateByRequest(requestId)` | 取回原试载，不重新申请 |

这些分支在 v2 下实际几乎不会发生（没有客户端超时），但保留它们使 SDK 在传输异常时仍然给出确定结论，并且**从不盲目重发非幂等操作**（打开、提交字幕、启动任务、安装）。可安全重发的只有“最后写入生效”的处理资源，而且只在确定未执行时。

音频工作台“上一操作结果待确认”卡死的根因（插件把 `unknown` 当成需要用户确认的状态，并在此期间阻塞全部更新）在 v2 中不存在：`unknown` 由 SDK 依据规则 1 判定为未执行并处理，插件只看到 `applying` → `applied`/`failed`。

## 6. 显式条件提交

默认语义：**操作作用于插件看到的状态**。

- 来源控制（`setPaused`、`seek`、`setRate`、`restart`、`close`）与 `media.capture` 默认以 `player.state` 为条件：如果别的插件或用户在此之后改变了来源（打开、暂停、跳转都会推进来源戳），调用以 `code: "stale"` 失败，且什么都不做。SDK 在每个自己的操作完成后刷新 `player.state`，所以连续的自有操作不会互相判旧。
- `player.open` 默认**无条件**替换当前来源（用户发起的打开就该如此）。
- 字幕的选择、移除、样式默认以提交前读到的当前选择为条件。
- 处理资源总是以提交时的当前来源为条件（宿主要求），对插件透明。

需要绑定到更早快照时，保存一个 `PlayerState`（或 `SubtitlesState`），再传 `{ ifCurrent: state }`：

| 场景 | 写法 | 插件 |
| --- | --- | --- |
| 自然播完后自动推进：只在播完之后没人改过来源时打开下一项 | `media.onEnd((ended) => player.open(next, { ifCurrent: ended }))` | playlist |
| 截图：冻结用户在确认前看到的那一帧对应的来源 | `const seen = player.state; …确认…; media.capture({ ifCurrent: seen })` | capture |
| 拖动进度条：同一手势内的每次跳转以上一次跳转的结果为条件，任何外部变化都终止手势 | `const scrub = player.scrub(); scrub.seek(t)` | playback |
| 字幕样式只作用于用户开始调整时选中的轨 | `subtitles.setStyle(style, { ifCurrent: editing })` | subtitles |
| 只判断不提交 | `player.isCurrent(state)` | capture（表单期间提示来源已变化） |

示例代码见 usage 文档的“播放栏”“播放列表”“截图导出”“字幕”。

## 7. 错误模型

- 失败一律以普通 `Error` reject/抛出，带稳定的 `code`（类型 `PluginError`）：宿主错误码原样保留（`stale`、`busy`、`unsupported`、`unavailable`、`conflict`、`invalid`…），来源操作失败的 `{ backend }` 映射为 `code: "backend"`、`message` 为后端文本；SDK 自己的码：`replaced`、`failed`、`ended`（`media.ready`），`aborted`（`AbortError`），`backend-uncertain`，`unknown`/`rejected`/`expired`（见 5.1）。
- SDK 调用的处理函数里抛出的错误（包括 async 处理函数的 reject）交给所在作用域的 `onError`；没有监听就上交父作用域，最终写入插件日志。插件不需要到处写 `.catch(report)`。
- 滤镜这类持续目标用状态表达失败：`filter.status`（`waiting`/`applying`/`applied`/`failed`）。`failed` 之后不会自动重试，直到 `set()`、`retry()` 或来源变化。
- SDK v2 插件在运行器中有 `unhandledRejection` 兜底（只记日志），忘记 `await` 的 promise 不会让插件进程崩溃。

## 8. 与旧 API 共存及删除步骤

运行器按入口模块的导出区分：

- `export default function activate(plugin)` → SDK v2：传入 `Plugin`，请求无客户端超时，激活等待注册完成，停用时自动释放。
- `export function activate(host)` → 旧扁平 `Host`：行为与之前完全相同（含 10 秒超时、返回的清理函数）。

两者共用同一条管道和事件分发，但一个插件只会是其中一种，因此各自的操作计数器不会冲突。旧 API 不再新增任何东西。

P4 删除步骤：

1. 确认 `grep -rn "export async function activate\|export function activate" src/plugins` 无结果（全部改为默认导出），且 `src/examples/` 已迁移或标注为宿主测试用底层示例。
2. 运行器删除 `module.activate` 分支和 `deadline` 参数（只保留无超时请求）。
3. `src/sdk/index.ts`：不再导出 `createSdk`/`Host`/`PluginModule` 给插件；扁平客户端（`commands.ts`、`processing.ts`、`source.ts`、`subtitles.ts`、`media.ts`、`producer.ts`、`source-resolvers.ts`）保留为 `sdk/plugin/` 的内部实现，类型中仅保留插件需要的节点/样式等定义（`VideoNode`、`AudioNode`、`SubtitleStyle`、`ResolvedMedia`、`MediaJob`），由 `sdk/plugin/index.ts` 再导出。可考虑把 `sdk/plugin/index.ts` 提升为 `sdk/index.ts`，届时插件的导入路径统一修改一次。
4. 插件工坊生成的计数器插件目前使用旧 API（见第 11 节），必须在删除前改为 v2。
5. 更新 `doc/rebuild/api.md`。

## 9. 旧 API 对照表

| 旧（扁平 Host） | 新（SDK v2） |
| --- | --- |
| `export async function activate(host)` + 返回清理函数 | `export default async function activate(plugin)`；无需清理 |
| `commands.register(id, 1, decl, state)` + `commands.onExecute` 按 id 分发 + `commands.update(id, ++rev, state)` | `commands.add({ id, path, enabled: () => …, run })`；`command.set({...})` |
| `ui.open(view, file, props, [callbacks])` + `ui.onCallback` 按 view/callback 分发 + 逐属性 `ui.set` | `windows.create(file)`；`window.on(name, fn)`；`window.bind(() => props)` / `set`；`show`/`hide`/`close` |
| `ui.mount({slot, file, component, global})`、`ui.setMount`、`ui.showMount`、`ui.mountState` + `window.state` 订阅、`ui.interaction` | `panels.mount(slot, file)`；`on`/`bind`/`set`；`visibility`、`onVisibility`、`setVisible`、`toggle`；`setInteraction({ dragging })` |
| `dialogs.open({ kind })` → `{ cancelled, paths }` | `dialogs.openFile()` 等 → 路径或 `null`/`[]` |
| `source.state()` + `events.subscribe("source.state")` + `eventSequence` 过滤 | `player.state`、`player.onChange` |
| `source.submit({ kind: "open", … }, id)` + 失败后 `source.operation(id)` 轮询 | `await player.open(path, { signal?, ifCurrent? })` → `Media` |
| 轮询 `source.state` 比较 ticket 判断就绪/替换/失败 | `await media.ready({ signal })` |
| 比较 `stamp`/`cycle` 判断自然结束一次 | `media.onEnd(fn)` |
| `source.submit({ kind: "close", expected: ticket.stamp })` | `media.close()` |
| `source.submit({ kind: "control", control, expected: state.stamp })` | `player.setPaused/seek/setRate/restart/close(…, { ifCurrent? })` |
| `SeekFence` + `LatestValue` | `player.scrub()` |
| `playback.set("volume"/"muted")` + 自制合并队列 | `player.setOutput({ volume, muted })` |
| `producer.prepare/connect/state/ack/ackInput/release` + 预填充/激活/节拍循环 + `openProvider` | `player.openStream({ width, height, frame })` → `Stream` |
| `processing.state` + `processing.replace({ revision, expectedGeneration })` + `processing.operation` 恢复 + `Latest` 合并 | `video.filter(id)` / `audio.filter(id)`；`filter.set(nodes)`；`filter.status`；`filter.retry()` |
| `subtitles.state/select/remove/style` + `operationId` + `expected` 选择戳 + `recover()` | `subtitles.state`、`select`、`remove`、`setStyle(style, { ifCurrent? })` |
| `subtitles.begin/append/discard/commit` 分块上传 + sha256 | `subtitles.load(bytes)` → `SubtitleTrack` |
| `snapshot.capture({ operationId, expected, expectedProcessingRevision, expectedSubtitle … })` + `snapshot.release` | `media.capture({ processed, subtitles, audio, at, ifCurrent })` → `Snapshot`；`snapshot.dispose()` |
| `jobs.start` + `jobs.state` 轮询 + `jobs.cancel(jobId, opId)` + `jobs.operation` | `snapshot.export({ format, destination, … })` → `ExportJob`；`job.done`、`onChange`、`cancel()` |
| `sourceResolvers.register(decl, handler)` → `{ dispose }` | `resolvers.add({ id, schemes, hosts, resolve })` |
| `packages.validate/trial/trialStateByRequest/trialState/stopTrial/installed/install/rollback/operation/cancel` | `packages.validate(dir)` → `candidate.trial()` / `candidate.install()`；`packages.rollback(id)`；`trial.stop()`；`change.done`、`change.cancel()` |
| `processing.state().backendUncertain` + 自制“检查/重试”界面 | `filter.status`（`failed`，`backend-uncertain`）+ `filter.retry()` |
| `host.events.subscribe("playback.backend")`、`playback.state`、`window.setFullscreen`、`plugins.*`、`host.capabilities` | 无插件使用（字幕刷新由 SDK 内部订阅 `playback.backend`）；未提供 |

## 10. 测试工具

`src/sdk/plugin/testing.ts` 提供 `TestHost`（一个说真实 wire 协议的内存宿主）和 `startPlugin(activate, { directory })`。SDK 的修订号、条件提交、恢复、合并逻辑在测试中照常运行，插件测试只关心业务：

- 操作：`host.command(id).run()`（禁用/忙碌时与宿主一样拒绝）、`host.window(id).emit(callback, ...args)`、`host.mounted().emit(...)`、`host.dialogAnswers.push([path] | null)`。
- 模拟来源：`host.openElsewhere(path)`（另一个插件打开）、`host.setSource({ phase: "ready", duration })`。
- 断言：`host.window(id).props`（kebab 名）、`host.command(id).state`、`host.sent(method)`、`host.processing.resources`、`host.subtitles`、`host.snapshots`、`host.jobs`、`host.producers`。
- 注入故障：`host.handle(method, fn)` 抛 `HostError(code, message)` 模拟宿主拒绝，抛普通 `Error` 模拟答复丢失；`host.reset(method)` 恢复。
- 同步：`await host.settle(ms?)` 等到没有在途请求（可额外等待计时器）。

语义是简化的：没有解码、没有 Slint、作业每次查询前进一步、producer 用真实 Unix 套接字接收帧并立即“消费”。

## 11. 公共构建工具

`npm run plugin -- <check|build|test> <name…|--all> [--version x.y.z]`（`src/scripts/plugin.mjs`），`npm run plugins:build` 构建全部插件。

- `check`：用仓库的 TypeScript 与 `src/tsconfig.json`（严格、`erasableSyntaxOnly`）检查插件全部 `.ts`/`.mts` 及 `tests/*.test.ts`。
- `build`：先 `check`；清空并生成 `dist/`：`.ts` 经 TypeScript 去类型并把相对 `.ts` 导入改为 `.js`；`.slint`、`.mjs`、数据 JSON、`licenses/` 等原样复制；跳过 `tests/`、`node_modules/`、`vendor/`、`dist/`、点文件、`*.md`、`*.log`、`*.d.ts`、`tsconfig*.json`、`package*.json`；写 `dist/package.json`（`type: module`）与 `dist/manifest.json`（入口改为 `.js`，写入 `version`）。版本取 `--version`，否则 `manifest.json` 的 `version`，否则 `package.json` 的 `version`，三段 u32。产物若在运行时导入仓库 SDK 则报错（只允许 `import type`）。
- 依赖：插件 `package.json` 有依赖而缺 `node_modules` 时执行 `npm ci`（失败则提示修复锁文件）。nes 的 `jsnes` 由 `dist/` 向上解析到插件自己的 `node_modules`。
- 扩展点（插件 `package.json` 的 `"vestra"` 字段）：`"bundle": true | { "external": [...] }` 用插件自己的 esbuild 依赖打包入口；`"assets": ["vendor"]` 原样复制额外文件/目录。workshop 的配置应为 `{"bundle": {"external": ["./vendor/typescript/lib/typescript.js"]}, "assets": ["vendor"]}`（已在临时副本上验证可构建、产物可载入）。
- `test`：`node --test tests/*.test.{ts,mjs}`。
- `dist/` 已加入根 `.gitignore`（`src/plugins/*/dist/`）；各插件迁移时 `git rm -r --cached src/plugins/<name>/dist`。

## 12. 建议的宿主改动（本轮未做，供协调者决定）

SDK 已用现有 RPC 正确实现以下各项；这些改动只会让实现更简单或更省请求，不影响插件 API：

1. **`commands.unregister`**：目前释放子作用域里的命令只能禁用、无法从菜单移除。影响：仅 SDK `commands.ts`；需补 `commands.rs` 测试。
2. **`processing.state` / 字幕状态事件**：SDK 现在在来源阶段变化时读 `processing.state`，字幕在来源变化或 `playback.backend` 事件时重读。若宿主广播 `processing.state` 与 `subtitles.state` 事件，可去掉这些读取与插件侧的字幕轮询（subtitles 插件窗口打开时每 1.5 秒刷新，这一行为本轮由插件保留）。
3. **`source.operation` 区分 `unknown`**：从未见过的 id 现在返回 `unknown-expired` 错误，与“已被淘汰”混在一起；处理资源的操作簿已区分。依据有序管道，SDK 目前把它当作确定失败，实际不可达。
4. **窗口回调全转发**：若 `ui.open` 像面板一样转发根组件的全部回调，SDK 可取消“首次 `show()` 前绑定”的限制。

## 13. 已知限制与有意的细微差异

- `media.close()` 以“仍是同一个来源（代次相同）”为条件，用最新来源戳提交；旧 bilibili/youtube 的取消以打开时的精确来源戳为条件（期间若用户暂停过，旧代码关闭失败，新代码会关闭）。这是迁移中唯一的条件语义变化，新语义更符合“取消我打开的媒体”。
- 来源控制失败的 `message` 是错误码或后端文本，不再是 `JSON.stringify` 的结果；需要原文案的插件按 `code` 自行格式化。
- 运行器仍会把 v2 插件超过 10 秒的请求写日志，便于发现宿主卡住。
