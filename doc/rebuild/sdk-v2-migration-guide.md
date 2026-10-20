# SDK v2 迁移指南（插件迁移者）

> P4 后旧扁平 `Host` API 已删除，全部插件已迁移；本指南保留为迁移记录，也适用于把外部旧插件改到 v2。

先读：[sdk-v2-migration.md](sdk-v2-migration.md)（第 6、7 节）、[sdk-v2.md](sdk-v2.md)（尤其第 2、4、6、9 节）、[插件示例](../plugins/examples.md)，以及参考迁移 `src/plugins/video/`（`main.ts` 75 行、测试 `tests/video.test.ts`）。

目标只有一个：**行为不变，代码只剩业务**。菜单路径与标题、窗口与控件、回调效果、错误提示语义都保持；协议细节交给 SDK。

## 1. 步骤

1. **入口**：把 `export async function activate(host: Host)` 改为

   ```ts
   import type { Plugin } from "../../sdk/plugin/index.ts";
   export default async function activate(plugin: Plugin) { … }
   ```

   运行器靠“默认导出”识别 v2。删除返回的清理函数和 `disposed`/`live`/`active` 一类标志：插件停用时 SDK 释放一切，宿主撤销 owner 资源。只从 SDK `import type`；节点、样式等类型也从 `sdk/plugin/index.ts` 导入（`VideoNode`、`AudioNode`、`SubtitleStyle`、`ResolvedMedia`、`Json`）。
2. **manifest.json**：格式不变。`entry` 指向源码入口（`main.ts`，不要指向 `dist/`）；没有 `version` 的加上现在生产包的版本（见第 5 节表格），构建工具用它生成 `dist/manifest.json`。
3. **本地导入**：用 `./x.ts`（构建时改写为 `.js`）。`./x.js` 指向 `.ts` 文件的写法在开发清单直接载入 `main.ts` 时会找不到文件。
4. **逐项替换调用**：按 [sdk-v2.md 第 9 节对照表](sdk-v2.md#9-旧-api-对照表迁移记录) 和下面第 2 节的模式。
5. **删除自制轮子**：合并队列（`Latest`/`LatestValue`/`LatestProjection`/`paint`/`project`/`render` 循环）、结果查询与重试（`recover`、`mutation`、`submitOnce`、`operation()` 轮询、`unresolved` 记录）、过期事件过滤（`eventSequence`、`stamp`/`generation` 比较）、属性名转换与逐属性 `ui.set`、命令修订号与 `last` 缓存、手写清理清单（`offs`、`off.push`）。
6. **测试**：放到 `tests/*.test.ts`（或 `.test.mjs`），改用 `startPlugin` + `TestHost`（见 usage 第 12 节、sdk-v2.md 第 10 节）。原测试覆盖的行为必须继续覆盖；只测自制轮子本身的用例（例如 `Latest` 队列的单元测试）改为测试同一用户可见行为（例如“连续拖动只保留一个在途请求并以最后值结束”）。测试从 `../main.ts` 导入，不要从 `../dist/` 导入。
7. **目录清理**（第 7 节清单）：删除 `FREEZE.json`、`CANDIDATE.json`、`freeze-ready.json`、`CHECKS.json`、`PROVENANCE.json`、`publish.mjs`、`*.log`、`download-probe.json`、`real-extraction.json`、`manifest.production.json`、`build.mjs`/`build.cjs`/`check.mjs`、插件内 `tsconfig*.json`、`API_REQUESTS.md`（在报告中摘录仍未解决的条目）；`README.md` 改为简短的“功能、入口、限制”。已提交的构建产物从 git 移除：`git rm -r --cached src/plugins/<name>/dist`（`dist/` 已在根 `.gitignore` 中），然后删掉本地 `dist/` 或让构建重新生成。
8. **构建与自检**（在 `src/` 下）：

   ```
   npm run plugin -- check <name>
   npm run plugin -- build <name>
   npm run plugin -- test <name>
   grep -nE "expectedGeneration|eventSequence|intentVersion|operationId|unknownExpired|backendUncertain|\.operation\(|onCallback|onExecute|ui\.set\(" <源码文件>
   npm run check && npm run boundaries
   ```

## 2. 常见模式的新写法

### 合并连续更新（滑块、拖动、投影）

旧：`Latest` 队列 + `apply()` + 失败回调。新：直接写期望值。

```ts
window.on("adjust", (field: string, value: number) => { picture[field] = value; filter.set(nodesFor(picture)); });
window.bind(() => ({ ...picture, status: describe(filter.status) }));
```

SDK 对窗口属性、面板属性、命令状态、处理链、字幕样式、音量都是“一个在途 + 最新值”。

### 结果不确定（超时、`unknown`、“请检查状态”）

删除。v2 没有客户端超时；答复丢失由 SDK 按 [sdk-v2.md 5.1](sdk-v2.md#51-结果不确定的处理与依据) 给出确定结论。插件看到的只有：promise 完成、以 `code`/`message` 失败、或尚未结束。原来专门为“结果未知”设计的按钮（例如“检查状态 / 重试”）保留为普通的“重试”：`filter.retry()` 或再次执行同一操作。只为“结果未知”显示的文案会自然消失，这不算行为变化。

### 命令及其启用状态

```ts
plugin.commands.add({
  id: "next",
  path: ["文件", "播放列表", "下一项"],
  enabled: () => !!queue.neighbor(1) && !opening,
  busy: () => opening,
  reason: () => (opening ? "正在打开" : "当前没有可操作项"),
  run: () => advance(1),
});
```

- `title` 默认取 `path` 最后一段；不同时再写 `title`。
- `run` 里抛出的错误交给 `plugin.onError`。从菜单/快捷键执行时，宿主在命令禁用或忙碌时不会分发；但面板或窗口按钮直接调用同一动作时不经过宿主，这类入口仍要自己检查启用条件（例如 playback 的 `action` 回调）。
- 不要再维护修订号；状态没变 SDK 不会发。

### 窗口与回调

- `const window = plugin.windows.create("ui/x.slint")`（窗口 id 默认取文件名；同一文件开两个窗口时传 `{ id }`）。
- **在第一次 `show()` 之前绑定全部回调**（宿主只转发打开时登记的回调；打开后再绑定新名字会抛错）。
- 打开时用期望属性做初始属性；`show()` 可重复调用（已打开则显示）；`hide()` 保留状态；`close()` 后再 `show()` 重新打开。
- 回调名与属性名可以照抄 `.slint` 的 kebab-case，也可以写 camelCase。回调参数类型直接写在处理函数上：`(index: number, value: number) => …`。宿主按 `.slint` 声明的类型传参，旧代码里的 `typeof value !== "number"` 检查在声明类型相符时可以去掉；值域检查（clamp、整数、有限数）保留。

### 用户可编辑的 in-out 属性

`bind` 只在期望值变化时发送，回调不会触发重发，用户在输入框、标签页里的修改会保留。插件需要主动写入（打开编辑器时填初始值、“标记起点”写当前时间、菜单入口切到某个标签页）时用 `window.set({...})` / `panel.set({...})`：即使值与上次相同也会发送。不要为了“防止被覆盖”而把用户输入同步回插件状态。

### 面板（底栏、侧栏）

- `const bar = plugin.panels.mount("bottom", "ui/bar.slint")`；组件名和桥接 global 从文件推断（文件只导出一个 component 和一个 global 时）。
- 可见性：`bar.visibility`（`preferred`/`effective`/`reason`），菜单“显示…”用 `bar.toggle()`，面板内“隐藏”用 `bar.setVisible(false)`，变化时绑定自动刷新；不要再订阅 `window.state`。
- 焦点/拖动/弹出锁：`bar.setInteraction({ focused })`、`{ dragging }`、`{ popupOpen }`，分别更新，SDK 合并成完整的一组。

### 来源状态与过期事件

`plugin.player.state` 永远是最新的；`plugin.player.onChange(fn)` 只投递更新的状态。判断“是不是另一次载入”用 `state.sourceId`；判断“自从我看到这个状态以来来源有没有被改动过”用 `plugin.player.isCurrent(state)`。`state.endReason`（`natural` 等）与 `state.error`（载入失败原因）替代 `terminalReason`。`state.can.snapshot` 替代 `capabilities.snapshotSource`，音轨 `id` 替代 `trackId`，`exportable` 替代 `!external && streamIndex !== null`。

### 打开、等待、取消、自动推进

```ts
const media = await plugin.player.open(url, { signal });  // 宿主接受载入后返回
await media.ready({ signal });                            // 就绪；失败 code: failed / replaced / ended
media.onEnd((ended) => plugin.player.open(next, { ifCurrent: ended }));
await media.close();                                      // 仍是自己的来源才关闭
```

超时用 `AbortSignal.timeout(ms)` 与自己的 `AbortController` 组合（`AbortSignal.any`）；中止时 SDK 关闭这个媒体。

### 网络媒体超时：是否关闭媒体

`open`/`ready` 的 `signal` 一旦中止，SDK 会关闭这个媒体（若仍是当前来源）。因此超时有两种写法，按旧行为选：

- **超时即取消并关闭**（bilibili 旧行为：超时后 `close`）：把超时合入 signal。

  ```ts
  const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(60_000)]);
  const media = await plugin.player.open(url, { signal });
  await media.ready({ signal }); // 超时 → 关闭媒体并以 AbortError 失败；用 controller.signal.aborted 区分用户取消
  ```

- **超时只停止等待、媒体继续载入**（youtube 旧行为）：signal 只放用户取消，用计时器与 `open`/`ready` 竞速。

  ```ts
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error("仍未确认就绪；未重新提交")), 120_000);
  });
  try {
    const media = await Promise.race([plugin.player.open(url, { signal: controller.signal }), timeout]);
    await Promise.race([media.ready({ signal: controller.signal }), timeout]);
  } finally {
    clearTimeout(timer);
  }
  ```

### 来源控制与条件提交

`setPaused`、`seek`、`setRate`、`restart`、`close` 默认以插件看到的 `player.state` 为条件，与旧代码传 `expected: state.stamp` 等价，因此**旧代码里的 `expected: state.stamp` 直接删掉**。旧代码传的是更早保存的戳（表单开始时、播完时、手势开始时）的地方，改为保存当时的 `PlayerState` 并传 `{ ifCurrent }`；拖动手势用 `player.scrub()`。详见 [sdk-v2.md 第 6 节](sdk-v2.md#6-显式条件提交)。

### 错误显示

`plugin.onError((error) => (status = error.message))` 统一接住所有处理函数里的错误，替代到处的 `.catch(report)`。需要区分时看 `error.code`（例如 `stale` → “来源已变化”）。一个功能模块可以在自己的子作用域上 `onError`。

### 计时器与手动刷新

SDK 处理函数之外改变的状态（`setInterval`、长任务中途的进度）需要 `plugin.update()` 让绑定重新求值。计时器用 `plugin.onDispose(() => clearInterval(t))` 清理。

### 清理

删除。需要单独释放的一组功能用 `plugin.scope()` 创建，`scope.dispose()` 释放。

## 3. 各插件的特别提示

| 插件 | 提示 |
| --- | --- |
| audio | `AudioController` 的 `unresolved`/`epoch`/`sequence`/`busy`/`failed` 全部由 `plugin.audio.filter("workbench")` 取代：`busy` = `filter.status.state === "applying"`，失败文案 = `status.error.message`，“检查状态/重试” = `filter.retry()`。按来源+音轨复位见 usage 第 8 节（从未应用过时复位不会发请求，与旧 `owned` 判断等价）。预设命令的 `checked` 用函数。“上一操作结果待确认”文案随 bug 一起消失。版本 1.1.0。 |
| bilibili | `submitOnce`/`openBilibili` → `player.open(url, { signal })` + `media.ready({ signal })`，60 秒“媒体加载超时，请重试。”用 `AbortSignal.any([controller.signal, AbortSignal.timeout(60_000)])`，失败时按 `controller.signal.aborted`（用户取消）区分取消与超时文案；“取消”= `abort()` 后 `window.close()`。manifest 没有 `events` 能力：SDK 在 `ready()` 期间自动轮询来源状态，无需改能力。旧 `dispose` 里把命令置为禁用的逻辑删除。`entry` 改为 `main.ts`，`dist/` 从 git 移除。版本 0.1.0。 |
| capture | 表单期间的来源守卫 `ensureForm` → `player.isCurrent(formState)`；`freeze` → `plugin.media.capture({ ifCurrent: formState, processed, subtitles, audio, at })`（`at` 省略 = 接受时原子地取当前位置；`audio` 传音轨 id 或 `"none"`）；`mutation()`、`jobs.operation` 轮询删除；`runJob` → `snapshot.export(...)` + `job.onChange` + `await job.done`；连拍的每张用同一快照多次 `export({ format: "png", sampleTime })`；5 分钟保留 = 持有 `Snapshot` 句柄、到期 `dispose()`；取消 = `job.cancel()`（返回 `requested`/`tooLate`/`terminal`）；“查询”= `job.refresh()`。错误窗口是第二个 `windows.create("capture-error.slint")`。`event.revision === 1` 判断删除。`behavior.test.mjs` 移入 `tests/`。`entry` 改为 `main.ts`。版本 1.0.0。 |
| nes | `stop`/`pump`/`openRom` 中的 producer 逻辑整体由 `player.openStream({ width: 256, height: 240, frame: () => core.frame(), signal })` 取代；“打开中被停止”的 `openingAttempt` → `AbortController`；`state.error ?? 'NES source replaced.'` → `stream.onStop((error) => …)`。`core.ts` 不变；`main.ts` 的 `./core.js` 改为 `./core.ts`。`package.json`/`package-lock.json` 保留（`jsnes`），构建工具在缺 `node_modules` 时执行 `npm ci`。测试改为从 `../main.ts` 导入并用 `TestHost`（它用真实 Unix 套接字接收帧，可断言预填充帧数与释放）。删除 `tsconfig.json`、`PROVENANCE.json`。版本 0.1.0。 |
| playback | `SeekFence` + `seeks` 队列 → `player.scrub()`（拖动开始时新建；方向键/跳转在 `!scrub?.active` 时新建，与旧“`!seekFence?.valid` 才新建”一致；`seek()` 返回 `false` 时不清草稿）；`seek-cancelled` 后忽略后续 `seek-changed` 的 `blockedGesture` 是业务状态，保留。`volumes` 队列 → `player.setOutput({ volume, muted: false })`（返回 `true` 时清 `volumeDraft`）；静音 → `setOutput({ muted })`。`operation()` → `setPaused`/`setRate`/`close`/`restart`/`open`（删掉 `expected: state.stamp`）。`project()` 投影与命令 `last` 缓存 → `bar.bind` 与命令函数。编辑器：`bar.set({ editorValue })` + `bar.setInteraction({ popupOpen: true })`；编辑器打开时的来源戳检查 → 保存 `PlayerState`，确认时 `player.isCurrent(saved)`。“显示播放栏” → `bar.toggle()`，`checked: () => bar.visibility?.preferred ?? true`。`report` → `plugin.onError`。版本 0.1.1。 |
| playlist | `Queue`（行、选中、重复检测、`realpath`）是业务，保留，但 `Queue.observe` 不再接收 `SourceState`：绑定关系改为持有 `Media`（`media.current` 为假 → “已切换到列表外来源”），自然结束改为 `media.onEnd` + `{ ifCurrent: ended }`（usage 第 4 节）；行状态（播放中/已暂停/正在打开/失败）从 `player.state.phase`/`paused`/`error` 推导。`pending`/`recover`/1 秒轮询/“结果未知”删除：`await player.open()` 在宿主接受后返回，失败以 `code` 抛出（`stale` → “自动推进已过期，保留较新的来源”）。`LatestProjection` → `list.bind`。链接窗口是第二个 `windows.create("ui/link.slint")`。`focus-changed` → `list.setInteraction({ focused })`；“隐藏”→ `list.setVisible(false)`；“显示播放列表”→ `list.toggle()`。版本 0.1.0。 |
| subtitles | `recover()`、`operationId`/`styleRevision`/`serial`、`accept()` 过期过滤、分块上传与 sha256 → `subtitles.load(bytes)`、`select`、`remove`、`setStyle`。`readSrt`（扩展名、大小、UTF-8 校验）是业务，保留。旧 `same()` 比较 → `track.id`；`resources` 集合 → `track.owned`。`run()` 的串行与 `busy` 是用户可见行为（菜单 busy），保留为一个简单的串行包装；`epoch`/`guard` 的“媒体已切换”取消可以改为在操作开始时保存 `subtitles.state` 并传 `{ ifCurrent }`（宿主以 `stale` 拒绝）。样式的“字幕轨已变化，请在当前轨道重新调整”用 `{ ifCurrent: 开始调整时的 state }` 并把 `stale` 映射为该文案。窗口打开时每 1.5 秒刷新的行为保留（`setInterval(() => subtitles.refresh())`）。版本 0.1.2。 |
| workshop | `trial`/`stopTrial`/`trialStateByRequest`/`installed`/`install`/`rollback`/`operation` 轮询 → usage 第 10 节；`recover-trial` 按钮在 v2 下不再需要启用（试载答复丢失由 SDK 取回原试载），控件保留、`can-recover` 恒为 `false`。构建：`main.js`（已提交的打包产物）删除，`entry` 改为 `main.ts`，`package.json` 加 `"vestra": { "bundle": { "external": ["./vendor/typescript/lib/typescript.js"] }, "assets": ["vendor"] }`；`vendor/` 不动。`package-lock.json` 与 `package.json` 不同步（`npm ci` 失败）：请在报告中说明，不要擅自升级依赖。测试脚本移入 `tests/` 并改名 `*.test.mjs`；测试生成的 `evidence/` 不提交。**注意**：工坊生成的计数器插件代码（`engine.mjs` 的 `counterSource`）仍使用旧扁平 API，类型来自 `vendor/sdk`；本轮先保持不变（运行器仍支持旧 API），作为已知问题在报告中注明，由协调者决定 P4 前的处理方式。版本 1.0.0。 |
| youtube | `YouTubePlayback`（自有操作 id、查询、轮询 400×300 ms）→ `player.open(url, { signal })` + `media.ready({ signal })`（用 `AbortSignal.timeout(120_000)` 保持原上限）；取消 = `abort()`。下载（`service.download`）不涉及宿主 API，保留。`test.mjs` 移入 `tests/`。dev 清单没有 `version`，构建工具回退到 `package.json` 的 0.1.1（也可以把 `version` 写入 `manifest.json`）。删除 `manifest.production.json`、`publish.mjs`、`*.log`、`download-probe.json`、`freeze-ready.json`；`dependencies.lock.json` 保留。 |

## 4. 迁移阻塞的判断

先在 usage 文档和对照表里找写法。只有当某个**现有用户可见行为**用 v2 无法表达时才是阻塞：停止该部分，按任务说明第 9 节格式报告（现有行为、旧代码位置、为何无法表达、最小复现），继续迁移其它部分。不要绕过 SDK 直接使用底层 transport，也不要在插件里重新实现协议细节。

## 5. 版本对照（写入 manifest.json 的 `version`）

| 插件 | 生产版本 | 来源 |
| --- | --- | --- |
| audio | 1.1.0 | 旧 `build.mjs` 默认值 / 旧 `dist/manifest.json` |
| bilibili | 0.1.0 | `manifest.json` 已有 |
| capture | 1.0.0 | 旧 `dist/manifest.json` |
| nes | 0.1.0 | `manifest.json` 已有 |
| playback | 0.1.1 | 旧 `build.mjs` 默认值 |
| playlist | 0.1.0 | 旧 `build.mjs` 默认值 |
| subtitles | 0.1.2 | `manifest.json` 已有 |
| video | 1.0.1 | 已迁移 |
| workshop | 1.0.0 | `manifest.json` 已有 |
| youtube | 0.1.1 | `package.json` |
