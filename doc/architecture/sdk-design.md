# SDK 设计说明

插件 API（SDK v2）为什么这样设计：句柄、生命周期、更新轮、协议细节如何被 SDK 吸收、条件提交、错误模型。给 API 维护者和想深入理解的插件作者看；接口的精确定义见 [API 参考](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html)，写法见 [插件示例](https://vestrasys.github.io/vestra-player-sdk/documents/Examples.html)。

代码：npm 包 `@vestra-player/sdk`，独立仓库 [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk)。以下 SDK 路径相对于该仓库：公共类型在 `src/plugin/types.ts`，由包入口 `src/index.ts` 导出；测试宿主 `src/plugin/testing.ts`（`@vestra-player/sdk/testing`）；`createPlugin` 在 `@vestra-player/sdk/runtime`（仅供播放器）；单元测试 `test/`；构建工具 `bin/vestra-plugin.mjs`。播放器仓库里：运行器 `src/runtime/runner.ts`，最短的完整插件 `src/plugins/video/`。

## 1. 一句话

插件默认导出 `activate(plugin)`。`plugin` 上按领域分组的工厂创建**句柄**（命令、窗口、面板、滤镜、媒体、快照、任务……），句柄自带回调、自带最后写入生效的状态同步、自动随作用域释放。插件代码只描述“想要什么”，协议字段全部留在 SDK 里。

```ts
import type { Plugin } from "@vestra-player/sdk";

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
| `commands` | `add({ id, path?, title?, order?, group?, radio?, bindings?, focus?, repeat?, enabled?, checked?, busy?, reason?, run })` | `Command` | `set(state)`、`ready` | 全部 |
| `windows` | `create(file, { id? })` | `Window` | `on`、`set`、`bind`、`show`、`hide`、`close`、`opened` | video、audio、subtitles、capture、nes、bilibili、youtube、workshop、playlist |
| `panels` | `mount("bottom" \| "sidebar", file, { component?, global?, title? })`；`bottom`、`sidebar`（位置状态） | `Panel`；`PanelSlot` / `SidebarSlot` | Panel：`on`、`set`、`bind`、`setInteraction`、`showMenu`、`ready`；Slot：`state`、`onChange`、`setVisible`、`toggle`、`setDetached`（侧栏） | playback、playlist |
| `dialogs` | `openFile`、`openFiles`、`openDirectory`、`saveFile` | —（返回路径或 `null`） | | playback、playlist、subtitles、capture、nes、youtube、workshop |
| `player` | `state`、`onChange`、`refresh`、`isCurrent`、`open`、`setPaused`、`seek`、`setRate`、`restart`、`close`、`scrub`、`setOutput`、`openStream`、`provideSequence`、`next`、`previous`、`stats` | `Media`、`Scrub`、`Stream` | Media：`current`、`ready`、`onEnd`、`close`；Scrub：`seek`、`active`、`end`；Stream：`running`、`onStop`、`stop` | playback、playlist、audio、capture、nes、bilibili、youtube |
| `video` / `audio` | `filter(id)` | `VideoFilter` / `AudioFilter` | `set`、`status`、`onStatus`、`retry` | video、audio |
| `subtitles` | `state`、`onChange`、`refresh`、`select`、`load(bytes, { ifCurrent?, signal? })`、`remove`、`setStyle` | `SubtitleTrack`（值对象：`id`、`kind`、`embeddedId?`、`title`、`language`、`styleSupported`、`styleReason`、`owned`） | | subtitles |
| `media` | `capture({ at?, processed?, subtitles?, audio?, ifCurrent? })` | `Snapshot` → `ExportJob` | Snapshot：`export`、只读描述；Job：`state`、`onChange`、`done`、`cancel`、`refresh` | capture |
| `resolvers` | `add({ id, schemes, hosts, resolve })` | `Resolver` | `ready` | bilibili、youtube |
| `packages` | `validate(dir)`、`rollback(id)` | `PackageCandidate` → `PackageChange` | Candidate：`install`；Change：`status`、`message`、`onChange`、`done`、`cancel` | workshop |
| `osd` | `show(text)`、`panel()` | `OsdPanel` | `set(text \| null)` | info |
| 作用域 | `scope()`、`add`、`onDispose`、`onError`、`update`、`dispose` | `Scope` | | 全部（`onError`、`update` 按需） |

模块需要的 manifest 能力（宿主按能力授权，SDK 不改变这一点）见 SDK 文档站 [Reference: Capabilities](https://vestrasys.github.io/vestra-player-sdk/documents/Capabilities.html)（由 SDK 的 `CAPABILITY_INFO` 生成，`src/tests/sdk/host-agreement.test.ts` 与宿主 `dispatch` 的按方法检查核对）。没有 events 能力时，`media.ready()` 自动轮询来源状态。

字幕要点：比较字幕轨用 `track.id`（不透明字符串，不要解析其格式）；内嵌轨的轨道号用 `track.embeddedId`（外部轨没有该字段）。`load` 的 `signal` 在分块之间和提交前检查：已中止则 discard 上传并以 `AbortError` 拒绝；提交一旦发出就不再中止（已交给宿主的提交可能已生效）。上传 id 每个插件实例从 1 开始：宿主每次载入插件分配新的 owner 代次，并在 owner 释放时移除其上传 high-water。

命名取舍：用户可见的领域名词（`player`、`filter`、`panel`、`stream`、`snapshot`、`job`、`resolver`），宿主内部名（`processing`、`producer`、`source`、`mount`、`$mount`）不出现在插件 API 中。没有 `ctx`：插件拿到的对象就叫 `plugin`，一个功能模块拿到的是 `Scope` 或具体句柄。

## 3. 生命周期与释放

1. **激活**：运行器载入入口模块。运行器先创建插件对象（有 `playback.read` 时先读一次来源状态，有 `events` 时订阅 `source.state`），调用 `activate(plugin)`，再等待激活期间发起的全部注册（命令、面板挂载、解析器、事件订阅）完成，最后才报告 `runtime.ready`。任一注册失败 → 激活失败（`runtime.failed`）。插件代码里不需要 `await` 这些注册。
2. **声明即注册**：`commands.add`、`panels.mount`、`resolvers.add` 同步返回句柄，注册在后台排队进行；需要时可 `await handle.ready`。激活后创建的句柄，注册失败交给所在作用域的 `onError`。
3. **作用域**：每个句柄在创建时加入当前作用域。`scope.dispose()` 按创建的逆序释放其中所有句柄和子作用域；直接 `handle.dispose()` 会把它从作用域移除。重复释放无害，释放从不抛错（失败只写日志）。
4. **插件停用**：宿主发送 `runtime.stop` → 运行器拒绝新请求并释放根作用域。此时宿主已经按 owner 撤销了该插件的全部资源（菜单、窗口、面板、处理资源、字幕资源、快照、任务、解析器、producer），因此释放只做本地清理（停计时器、中止信号、关闭 producer 套接字），对宿主的请求被立即拒绝并忽略。插件**不需要**返回清理函数。
5. **各句柄释放时做什么**（运行期间释放子作用域时生效）：

| 句柄 | 释放 |
| --- | --- |
| Command | 不再分发；`commands.unregister` 从菜单与绑定表删除（同 id 的再次注册排在删除之后） |
| Window | `ui.close` |
| Panel | `ui.unmount` |
| 序列提供者（`provideSequence`） | `sequence.set(null)` |
| OsdPanel | 显示过文字则 `osd.panel(null)` |
| VideoFilter / AudioFilter | 若曾应用过，把本资源替换为空链（只撤销自己的贡献） |
| Snapshot | `snapshot.release` |
| ExportJob | 未结束则 `jobs.cancel` |
| Stream | 停止渲染，若仍是当前来源则关闭，释放 producer |
| Resolver | `sourceResolvers.unregister`，中止进行中的解析 |
| 监听（`onChange`、`onEnd`、`on`、`bind`…） | 退订 |

`Media` 不随作用域关闭来源：插件停用时当前播放继续（播放列表的既有行为）。需要关闭时显式 `media.close()`。

## 4. 状态同步：值、绑定与更新轮

连续变化的值只描述期望状态，SDK 负责合并、去重和顺序下发：

- `window.set(props)` / `panel.set(props)`：立即写入期望值。
- `window.bind(() => props)` / `panel.bind(...)`、命令选项里的函数（`enabled: () => ...`）：**绑定**，在每个更新轮重新求值。
- **更新轮**：SDK 在以下时机自动安排一轮（同一微任务内多次请求只跑一轮）：SDK 调用的任何插件处理函数（命令 `run`、窗口/面板回调、`onChange`、`onStatus`、`onEnd`、`onStop` 等）同步返回后与其 promise 结束后；播放器、字幕、面板可见性、滤镜状态、导出任务状态变化后。插件在自己的计时器里或长任务的中途改了状态时，调用 `plugin.update()`。
- 每个目标同一时间只有一个请求在途，期间的变化合并为下一次发送。`bind` 的值只在**期望值变化**时发送；`set()` 写入的值总会发送一次（即使与上次相同），所以 `set()` 也是把被用户改动过的控件“拉回模型值”的显式方式。回调本身不会触发重发：用户在 Slint 中修改的 in-out 属性（输入框文字、标签页、滑块位置）在其它回调之后保持不变。
- 写入失败：同一失败的绑定值不会自动重发，直到它变化；`set()` 的值在退避后重试（0.25 秒起翻倍，最长 30 秒）；一次连续失败只报告第一次错误。因此 `onError` 里写状态到一个已损坏的窗口不会形成重试循环。
- 窗口在第一次 `show()` 时用当前期望属性作为初始属性打开，不会先闪默认值；`close()` 后再 `show()` 会用最新属性重开。
- 名称转换：TS 中用 camelCase（`effectEnabled`、`toggleEffect`），SDK 转为 Slint 的 kebab-case；已是 kebab-case 的名字原样使用。结构体字段（如列表行的 `rowId`）不转换。

这是一个刻意很小的“响应式”：没有依赖追踪、没有虚拟 DOM，只有“求值、比较、发送变化”。插件状态就是普通变量。

## 5. 协议细节如何被吸收

| 宿主协议概念 | SDK 的做法 | 插件看到的 |
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

因此运行器**不给请求设客户端超时**。超时是此前“结果未知”的主要来源：客户端到点放弃等待，却无法知道宿主是否执行。现在请求超过 10 秒只写一条日志并继续等待真实答复。

SDK 仍然处理“答复丢失”（非宿主错误的拒绝，或客户端 `timeout`），规则如下：

| 领域 | 查询 | 结论 |
| --- | --- | --- |
| 处理资源 | `processing.operation(stage, resourceId, revision)` | `completed` → 已应用；`failed` → 失败；`unknown` → 由 1 可知从未被接纳、肯定未执行，用新修订号重发（连续 3 次则失败 `unknown`）；`unknownExpired` → 读 `processing.state`，本资源当前修订号等于该修订则已应用，否则未生效并重发；`pending` → 100 ms 后再查 |
| 来源 | `source.operation(id)` | `pending` 表示已接纳、仍在执行（例如 URL 解析中），按 40 ms 起、最长 500 ms 的间隔继续查直到 `completed`/`failed`；查询本身被宿主拒绝 → 失败 `unknown`。每个插件只能查自己的操作，宿主接受 `playback.control` 或 `playback.read`（`main.rs` `required_capability`） |
| 字幕 | `subtitles.operation(id)` | `completed` → 采用其状态；`failed` → 失败；`unknown` → 失败 `rejected`（未执行）；`unknownExpired` → 刷新状态并失败 `expired` |
| 快照/任务 | `jobs.operation(id)` | `completed` → 采用结果（快照会再读 `snapshot.state`）；`failed`/`unknown`/`unknownExpired` → 失败；`pending` → 250 ms 后再查 |
| 包安装/回退 | `packages.operation(requestId)` | 宿主保证 `operationId === requestId`，持续查询直到终态，从不重发 |

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
- `player.open()` / `openStream()` 的错误 code：`stale`（带 `ifCurrent` 时来源已变化，什么都没做）；`replaced`（宿主已接受，但在完成前被另一个来源替换或撤销，例如 URL 仍在解析时用户打开了别的文件；与 `media.ready()` 的 `replaced` 同义，不是这一项的失败）；`backend`（载入/解析失败，`message` 为后端文本）；`invalid`、`busy`、`conflict`、`unavailable`、`unsupported`（宿主拒绝接纳，未执行）；`aborted`（`AbortError`，`signal` 中止，已接受的会被关闭）。`openStream` 另有 `stream`（producer 报错，`message` 为宿主原因）、`device`（音频设备未就绪）、`timeout`（类型 `StreamStartError`，`error.stage` 为 `"prefill"`：首批音视频缓冲未在 10 秒内备好，或 `"activation"`：宿主选中后 10 秒内未激活；按 `stage` 区分，不要按消息文本）。`openStream` 被 `signal` 中止时，若选中请求已发出，SDK 先等宿主答复拿到来源代次，只关闭这一代来源，再释放 producer。
- SDK 调用的处理函数里抛出的错误（包括 async 处理函数的 reject）交给所在作用域的 `onError`；没有监听就上交父作用域，最终写入插件日志。插件不需要到处写 `.catch(report)`。
- 滤镜这类持续目标用状态表达失败：`filter.status`（`waiting`/`applying`/`applied`/`failed`）。`failed` 之后不会自动重试，直到 `set()`、`retry()` 或来源变化。
- 运行器对插件有 `unhandledRejection` 兜底（只记日志），忘记 `await` 的 promise 不会让插件进程崩溃。

## 8. 已知限制

- `media.close()` 以「仍是同一个来源（代次相同）」为条件，用最新来源戳提交：取消「我打开的媒体」时，即使期间用户暂停过，也能关闭。
- 来源控制失败的 `message` 是错误码或后端文本；需要特定文案的插件按 `code` 自行格式化。
- 运行器把超过 10 秒的请求写一条日志（不放弃等待），便于发现宿主卡住。
- 窗口回调必须在首次 `show()` 前绑定（宿主打开窗口时登记回调白名单）；面板没有这个限制。

测试工具与构建工具见 [Test a plugin](https://vestrasys.github.io/vestra-player-sdk/documents/Test_a_plugin.html) 与 [Command line](https://vestrasys.github.io/vestra-player-sdk/documents/Command_line.html)。
