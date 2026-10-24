# 2026-10 轮：侧栏页签与 NES 打开方式

开始于 2026-10-24。**本文是这一轮的唯一进度来源。**

## 侧栏页签

设计：[panel-slots.md](../../architecture/panel-slots.md)。用户已选定方案 A。

- [x] 第 0 步：挂载时直接报 `conflict`，不再连带激活失败；内置插件优先挂载（#2，真机验收通过）
- [x] 第 1 步（#3，2026-10-24）
  - [x] 维护者：宿主、运行时、SDK 1.5.2（已发布）；顺带修好窗口重开时上移一个标题栏高度的问题（浮动侧栏、主窗口、选项窗口都有）
  - [x] 播放列表作者：0.1.4，F6 改用 `list.toggle()`，有宿主页签条时隐藏自己的页签条
  - [x] 真机验收：页签切换、右键菜单、分离和附着、重启后恢复、F6 三种情况

## 请求 R1：本地文件按扩展名交给插件生成流（vestra.nes，§4.1）

**用户操作**

用打开文件（⌘O）、拖放、播放列表（包括上一个/下一个）打开 `.nes`，都应该直接启动模拟器。

现在这些入口都会把文件交给 mpv，打开失败，播放列表显示警告；只有「模拟器 ▸ 打开 NES ROM…」可用。NES 插件停用后，恢复成现在的行为。

**现有 API 为什么不够（作者的证据）**

1. `drops.provide({extensions})`：只覆盖拖到视频区这一个入口。⌘O、播放列表、上一个/下一个走的都是 `player.open(path)`。
2. `resolvers.add`：只认 http/https 加主机名，返回值只能是给 mpv 的 URL，不能返回生成流。
3. 监听 `onChange` 等 mpv 报错，再调用 `openStream`：
   - 失败提示已经出现，播放列表也已经标上警告；
   - `StreamOptions` 没有 `ifCurrent`，会和其它打开操作竞争；
   - 打开后 `source` 变成 `producer://<id>`，原路径丢失，播放列表的当前行和上一个/下一个就断了；
   - 和序列的自动推进互相竞争；
   - 插件只能靠猜来判断打开意图。
4. manifest 和能力表里都没有文件类型相关的字段。
5. 只保留模拟器菜单入口，正是用户报告的问题本身。改成让 playback、playlist、history 各自识别 `.nes`，要么需要插件间通信，要么核心要认识插件，两者都违反 §1。

作者的结论：打开意图、条件提交、来源命名、失败显示都属于宿主的来源仲裁（§4.3 第 4 条），而且这是通用能力，以后的模拟器、合成器、程序化来源都会用到。

**希望的用法**

`resolvers.add` 增加 `extensions: ["nes"]`，`resolve(path, signal)` 可以返回 `{ kind: "stream", width, height, frame, onStop }`。插件自己的菜单入口也改成「对话框 + `player.open(path)`」。具体形态由维护者决定。

**语义要求**

- 沿用原来那次 `open` 的仲裁、`ifCurrent` 和操作 id。
- 选中后 `kind === "stream"`，`source` 是原文件路径。
- 失败时 `phase: "error"`，错误文本由插件给出。
- 资源归登记解析器的插件。
- 取消通过 `signal` 传递。
- 同一扩展名只能有一个登记者，后来者 `conflict`，但不影响它的激活。
- 停用插件时注销解析器。
- 不出现任何系统特有的概念。

**兼容性**

类型上是可选扩展。有一处语义需要维护者确认：生成流的 `source` 变成文件路径后，只看路径、不检查 `kind` 的插件会受影响。

**验收场景**

⌘O 打开 ROM；混合播放列表里上一个/下一个；拖放；F4 停止；坏 ROM 报错；读取中被新的打开操作取代；停用后回退。用 TestHost 覆盖。

**作者顺带发现的问题（另行处理）**

- 宿主不接受命令行或系统传来的文件。
- 生成流的 `source` 实际是 `producer://<id>`，而需求文档写的是「`source` 为空」，两者不一致。

**维护者答复（2026-10-24）：改写后接受**

- **核实作者证据**：第 2、3、5 条成立。第 1 条有一处不准确：`.nes` 拖到视频区时，现在是被静默忽略，并没有交给 mpv。不改 API 的替代方案（A1–A5）都做不到用户要的效果。满足 §4.3 第 4 条：来源仲裁归宿主，按来源分派属于核心的打开职责，而且是通用能力。
- **范围**：只认本地绝对路径，只返回生成流；http(s) 解析器不变。
- **形态**：`ResolverOptions` 改为联合类型，新增 `{ id, extensions, resolve(path, signal): Promise<ResolvedStream> }`。`ResolvedStream = { kind:"stream", width, height, frame, started?(stream) }`：作者提的 `onStop` 改成交出 `Stream` 句柄，概念只有一个。
- **wire**：`register` 新增 `{id, extensions}` 形式；`respond` 新增 `{kind:"stream", producerId}`，宿主回 `{stamp}`。
- **能力与 manifest**：需要 `source.resolve` 和 `producer.av`；不新增 manifest 字段；NES 设 `minPlayerVersion`。
- **匹配**：来源不含 `://`、是绝对路径、扩展名不分大小写；扩展名分支先于 URL 解析（Windows 的 `C:\`）。
- **冲突**：一个扩展名只能有一个登记者，后来者 `conflict`，不导致激活失败。
- **来源命名**：`source` 保留原路径，`kind === "stream"`；直接 `openStream` 的来源仍是 `producer://<n>`。
- **失败**：错误用插件给的原文，所有解析器都去掉 `resolver failed: ` 前缀。
- **取消、停用、旧宿主**：取消沿用现有机制（被取代、30 秒期限、停用、注销）；停用后回到今天的行为；旧宿主靠 `minPlayerVersion` 挡住。
- **版本**：SDK 1.5.3（patch，排在 1.5.2 之后）。
- **需要回归的插件**：
  - 必须改：playlist（`queue.ts:531` 遇到 stream 会 detach）；
  - 要确认：marks、info、subtitles（`hasMedia` 不看 `kind`）；
  - 只跑测试：其余插件、youtube、bilibili。
- **工作量**：维护者约 2–2.5 天（人工估计），NES 约 40 行，playlist 几行。
- **待裁决的两项取舍**：
  1. host 形式的解析器冲突是否也改成不导致激活失败？
  2. 去掉错误前缀是否适用于所有解析器？
- **顺带发现**：
  1. 命令行和系统打开文件另立宿主条目（平台模块统一投递到 `Open` intent），R1 不依赖它。
  2. `producer://` 保持不变，改文档：`reference.md` 中 `PlayerState.source` 由维护者改；`feature-requirements.md:508` 由维护者提供文案、协调者落笔。

**作者回复（2026-10-24）：接受，不反驳**

作者逐项对照了验收场景，都能做到。另外请维护者在实现和文档里写明以下 4 点：

1. `resolve` 返回之后、激活之前如果被取消：释放生成端，不调用 `started`。
2. `started(stream)` 在激活后同步调用，早于任何 `onStop`。
3. `player.open(path, {paused:true})` 透传到流的 `startPaused`。
4. 解析期间调用 `player.close()` 会中止 `resolve` 的 `signal`。

NES 侧改动计划（约 40 行）：
- 解析器、拖放（`drops.provide` → `player.open`）；
- 入口改为「对话框 + `player.open`」；
- 删掉手写的打开、取消逻辑；
- 补测试，版本升到 0.3.0，`minPlayerVersion`。

**裁决（维护者，2026-10-24）：改写后接受，已实现**

实现在分支 `feat/file-resolvers`（播放器 worktree `wt-r1`）和 SDK 1.5.3（`sdk-r1`）。

- 作者提的 4 点都已实现，并有测试覆盖。
- 两项取舍：
  1. 解析器冲突一律不导致激活失败，host 形式也一样，与 #2 定下的原则一致；
  2. 所有解析器都去掉 `resolver failed: ` 前缀，SDK 改为发送 `error.message`。
- 旧宿主：使用文件解析器的插件设 `minPlayerVersion` 为下一版播放器的版本（暂定 0.4.1）。
- 合并顺序：SDK 1.5.2（侧栏页签）→ 把 1.5.3 rebase 上去 → 发布 → 播放器升级依赖 → NES 作者、playlist 作者按迁移说明适配。

## 适配 R1（2026-10-24）

- [x] 播放列表作者：0.1.5（PR #9）。混合列表里经过生成流时，当前行、上一个/下一个、自动推进都正常；解析中按 F4 关闭不算失败；从别处打开的本地生成流也会接管进列表（`producer://` 来源除外）。
- [ ] NES 作者：0.3.0（进行中）
- [ ] 真机验收

## 请求 R2：列出可由插件解析的文件类型（vestra.playlist，§4.1 草案）

- **用户操作与预期结果**：把 ROM 拖进列表，或扫描含 ROM 的文件夹，ROM 应该和视频一样加入列表；现在会显示「跳过 N 个不支持的项目」。
- **现有 API 为什么做不到**：没有接口能列出已登记的文件解析器扩展名。写死 `.nes` 违反 §1；全部接受又会把无关文件也加进列表。
- **希望的用法**：`resolvers.fileTypes(): string[]` 加一个变化事件。只读，只给小写扩展名，不暴露登记者。
- **状态与资源归谁；失败、取消、能力缺失时**：状态归宿主；插件停用后，对应扩展名随之消失；没有能力时返回空列表，不报错。
- **是否破坏现有契约**：纯新增。
- **最小验收场景**：NES 启用时把 game.nes 拖进列表，它被加入；停用 NES 后同样拖放，显示「跳过」。
- **作者顺带的建议**：解析被取消时，给一个独立的错误码（如 `cancelled`），不再要求插件比较「source resolution cancelled」这段文字。
- **维护者答复（2026-10-24）：改写后接受（未实现，等作者回复）**
  - 核实：拖进列表、添加文件或文件夹时，列表按自带的扩展名表过滤，插件无从得知哪些扩展名有解析器。⌘O 和拖到视频区打开的 ROM，0.1.5 已经会接管进列表。
  - 形态：只读快照 `plugin.resolvers.fileTypes` 加 `resolvers.onFileTypesChange`。只含已登记的文件解析器扩展名（小写、去重、排序），不给登记者；读取不需要能力，变化通知需要 events；旧播放器上返回 `[]`。SDK 1.5.8。
  - `cancelled` 错误码：拒绝。判据改用 `code === "backend" && player.state.phase === "empty"`，写进 reference §11。
- 相关修复：SDK 1.5.7。流被替换或关闭时，`onStop` 收到 null；TestHost 和真实宿主一样回收生成端（NES 实测时发现的缺陷）。

## 缺陷报告 D1 与请求 R3：大列表的面板属性（vestra.playlist，§4.1，2026-10-24）

来源：用户在 Windows 真机（0.4.2）上加入约 3000 个文件，列表比 PotPlayer 和单独的 mpv 慢得多。播放列表 0.1.6 已在插件内解决（只发视图附近 200 行，只为屏幕上的行读时长），下面是插件绕不开、应由维护者处理的部分。

### D1（缺陷）：属性值上限没有写进文档，TestHost 也不执行

- **代码位置**：`src/host/src/ui_values.rs`：`MAX_ARRAY = 1024`、`MAX_NODES = 4096`、`MAX_TEXT_BYTES = 1 MiB`（字段名也计入）。每次 `ui.setMount`、`ui.set` 都经过 `encode` 检查。
- **现象**：播放列表每行是 8 个字段的结构体，算 9 个值，所以超过约 450 行时整个 `rows` 写入都失败。0.1.5 加入 3000 个文件时，模拟中共发了 6417 次 `rows`（每次约 1 MB，总计约 2.8 GB），按宿主规则全部会被拒绝。真实播放器里列表停在旧内容上；宿主还要在 UI 线程上逐次解析这 1 MB JSON 再报错，这就是用户感到「卡」的主要原因。
- **为什么插件测试没发现**：TestHost 的 `ui.setMount` 直接收下任何值。reference.md 只在 Markdown 一节提到「共 4096」，普通属性没写这几个上限。
- **期望**：reference §6（属性）写明这三个上限和失败语义；TestHost 执行同样的检查，属性写入失败走与宿主相同的 `onError`。参照命令上限的先例（reference.md 第 1837 行：TestHost 没执行上限，插件测试没发现）。
- **最小复现**：`panel.bind(() => ({ rows: Array.from({ length: 500 }, (_, i) => ({ a: String(i), b: "", c: "", d: "", e: false, f: false, g: false, h: false })) }))`。在 TestHost 里通过，在播放器里每次写入都失败。

### R3（请求）：让宿主持有长列表，插件按行增删改

- **用户操作与预期结果**：列表里有 3000 到 20000 项（`ROW_LIMIT`）时，像 PotPlayer 那样，拖动滚动条到任意位置都立刻显示行，没有空白。
- **现有 API 为什么做不到**：受 D1 的上限约束，插件只能发一个窗口（0.1.6：200 行，约 68 KB），由 Slint `ScrollView` 按 `row-count` 定高度，`first-row` 定位置。窗口跟随滚动回调移动，要经过一次「Slint → 插件 → Slint」往返。滚轮和小幅拖动在 40 行边距内不受影响；一下拖到很远时，新位置的行要晚一次往返才出现。另外，一行的任何变化（选中、时长、当前项）都要把整个窗口重发一遍，宿主 reconcile 时对 200 行逐行 `to_json` 比较。
- **希望的用法**（示意，形态由维护者定）：
  ```ts
  const rows = panel.model("rows");           // Slint 侧 [PlaylistRow]，由宿主持有
  rows.splice(start, deleteCount, ...items);  // 增删，一次 ≤ 1024 行
  rows.set(index, item);                      // 改一行
  rows.reset(items);                          // 整体替换（排序后），可分批
  ```
  这样 Slint `ListView` 能在本地完成虚拟化，滚动时不用经过插件。
- **状态与资源归谁**：模型归宿主，所属面板卸载或插件停用时一并释放。超出总上限（如 20000 行）时 `limit`；没有能力时 `unsupported`。
- **是否破坏契约**：纯新增；现有的整值属性不变。
- **替代方案（不改 API）及代价**：即 0.1.6 现在的窗口方式，代价如上：大跳转时短暂空白，每次窗口移动重发 200 行。
- **最小验收场景**：20000 行的列表拖动滚动条从头到尾，每一帧都有行；选中一行只改一行。
- **影响**：目前只有 vestra.playlist；以后的媒体库、书签、历史等长列表插件也会遇到。

### 测量（TestHost + 插件进程内计时，3000 个空 `.mkv`，假 ffprobe 立即回答；宿主侧按 `ui_values.rs` 的规则校验每次写入）

| 操作 | 0.1.5 | 0.1.6 |
| --- | --- | --- |
| 添加文件（对话框，3000 个） | 27.4 s，5529 次写入，2.8 GB；串行 realpath；每个 ffprobe 结果都重发全部行 | 0.12 s，10 次写入，134 KB；ffprobe 49 次（只读屏幕上的行） |
| 拖放文件夹 / 添加文件夹 | 18.5 s / 19.6 s，约 1.8 GB | 0.42 s / 0.39 s，约 200 KB |
| 滚动一步 | 8 ms CPU（每次都 stringify 整个列表的快照） | 1.3 ms CPU（含 TestHost）；200 步里窗口移动 24 次 |
| 播放器状态推送一次（进度等） | 9 ms CPU | 1.4 ms |
| 点选、全选、播放一行 | 每次重发约 1 MB | 68 KB |
| 下一个 / 上一个 | 每次约 80 ms CPU，重发 3 MB | 约 10 ms，200 KB |
| 排序（标题、文件夹、乱序…） | 18 到 80 ms | 3 到 60 ms（大小、日期需要 stat） |
| 全选后删除 3000 行 | O(n²)（每行 indexOf、splice 和过滤等待队列） | 一次过滤 |
| 宿主会拒绝的写入 | 6417 次（rows 3000 > 1024） | 0 |

### 维护者答复 D1（2026-10-24）：按缺陷修，已实现（分支 `fix/ui-value-limits`，SDK 1.5.10，未提交）

- **核实**：成立。上限只在 `PropValue` 的 TSDoc 和 reference 的 Markdown 一节出现，`TestHost` 不检查。两处更正：
  1. 被拒的写入并非完全无声：SDK 把每段连续失败的第一次交给 `onError`（code `host`），但播放列表没有 `onError`，只进了插件日志。
  2. JSON 解析在宿主的读管道线程上，不在 UI 线程；UI 线程的转换到上限就停。0.1.5 的「卡」主要在插件侧（2.8 GB 的 stringify）和管道。另一个更危险的点：一行超过 1 MiB 时，宿主直接停止插件（`plugins.rs`）。3000 行、路径较长时，0.1.5 已经接近这条线。
- **修复**：
  - **文档**：reference §6.1 新增「属性值上限」表，写明失败语义；§13 `limit`、§14 TestHost、§17 变更说明同步更新；`PropValue`、`View.set`、`View.bind` 的 TSDoc 也已更新。
  - **TestHost**：`ui.open`、`ui.set`、`ui.mount`、`ui.setMount`、`settings.register`、`settings.set` 执行与宿主相同的检查（同样的顺序和消息）。新常量 `UI_VALUE_LIMITS` 从 `@vestra-player/sdk/testing` 导出；`tests/sdk/host-agreement.test.ts` 核对它与 `ui_values.rs` 一致。
  - **失败可见**：
    - 超限的 code 由 `host` 改为 `limit`，消息以属性名开头（`rows: UI array exceeds 4096 rows`）。
    - 面板、设置页的初始属性，以及设置窗口未打开时的 `settings.set`，以前不检查，到重建视图时才被悄悄丢掉（只写宿主日志）。现在在请求时检查：
      - `settings.set` 超限即失败；
      - 初始属性超限时不带它挂上，并发 `ui.error`。不让挂载失败，是因为激活期间的挂载失败会让整个插件激活失败：main 上的播放列表 0.1.5 恢复 20000 行时就会这样，`C17: a list above 20 000 rows is refused` 用 TestHost 1.5.10 跑正好复现。
      - 工具窗口的 `ui.open` 照旧整体失败。
    - 宿主之后才发现的界面失败（重建时图片已删、回调参数转换失败、浮动侧栏准备失败）作为 `ui.error` 发出，现在带 `code`；以前 SDK 根本不处理这个事件，现在交给根作用域的 `onError`。
    - 插件运行器在发送前拒绝超过 1 MiB 的请求（`limit`），不再让宿主停止插件。
- **上限本身（裁决：放宽两项，其余不变）**：
  - 在 release 构建（Apple M 系列）下测量一次写入在 UI 线程上的开销（转换加 reconcile），8 字段的行：200 行约 1.3 ms，1000 行约 4.9 ms。主要耗时在 reconcile：它把整值转成 JSON 比较一遍，又逐行再转一遍。已改为复用第一次的结果，reconcile 减少约 45%：200 行约 0.8 ms，1000 行约 3.2 ms，1800 行约 5.8 ms。
  - 改后每个值约 0.35 µs。所以 `MAX_NODES` 4096 → **16384**，`MAX_ARRAY` 1024 → **4096**：满额写入约 1800 行、约 6 ms，Windows 慢机估计在一帧上下，只在写入时发生。
  - 深度 8、单串 64 KiB、全部文本 1 MiB 不变。全部文本受 1 MiB 消息上限约束，放宽没有意义；要放宽就得改管道分帧，代价不小，这也说明整值属性不是长列表的出路（见 R3）。
  - 这样中等列表（章节、字幕条目、几百到一千多项的文件列表）可以整值发送；20000 行仍然做不到，也不应该这样做。
- **对播放列表的影响**：0.1.6 的 200 行窗口不受影响。`scroll.ts` 注释里写死的「1024 rows and 4096 values」，等 SDK 1.5.10 发布后请改为引用 `UI_VALUE_LIMITS`。`WINDOW_ROWS` 可以加大，取舍见 R3 的 A1。

### 维护者答复 R3（§4.3 第 2 步，2026-10-24）：倾向改写后接受，暂不实现

**先核实作者的论据**

- 窗口方式的两项代价都成立：
  - 远距离跳转要等一次「Slint → 插件 → 宿主 → Slint」往返，新位置的行才出现。
  - 任何一行变化都重发整个窗口。
- 往返的实际时长没有在真机上测过。按组成部分估算：回调入队、Node 处理并 stringify 68 KB、管道、读线程解析约 0.3 ms，UI 线程转换加比较约 0.8 ms（D1 改后的数字）。插件空闲时大约是 1–2 帧；插件正忙时（排序、批量 ffprobe 回来）会更长。请作者在第 3 步用真机数据确认或推翻这个估计。

**不改 API 的做法与代价**

| 方案 | 做法 | 代价 / 做不到什么 |
| --- | --- | --- |
| A0 现状（0.1.6） | 200 行窗口；`row-count` 定高度、`first-row` 定位置；滚动回调移动窗口 | 远距离跳转有一次往返的空白；每次变化重发 200 行（约 68 KB，宿主约 0.8 ms） |
| A1 加大窗口（D1 放宽后可行） | `WINDOW_ROWS` 改为 600–1000，边距相应加大 | 滚轮和拖动几乎不再触发窗口移动，但远距离跳转照旧空白；每次变化的重发量和宿主开销按行数线性增加（1000 行约 340 KB、约 3 ms） |
| A2 占位行 | 在 Slint 里按视口为窗口外的格子画占位（序号加骨架条），数据到了再替换 | 跳转时看到的是占位而不是空白，Explorer、多数虚拟列表都这样做；只改插件的 Slint 文件，约 30 行。做不到「每一帧都有真实的行」 |
| A3 拆出易变状态 | 行内容和选中、当前项、时长分开发：`current-row: int`、与窗口等长的 `[bool]` 选中表，时长单独一表 | 点选、播放、时长回填只发几百字节，不再重发行；插件和 Slint 的绑定复杂一些。不解决跳转 |
| A4 降低需求 | 接受「跳转后约 1–2 帧占位」作为验收标准，而不是「每一帧都有行」 | 与 PotPlayer 的体验有可见差距，只在快速拖动滚动条时出现 |

A1 到 A3 可以组合使用，不用等核心，作者可以先做。但它们都做不到验收场景的原文：20000 行从头拖到尾，每一帧都有真实的行。要做到这一点，数据必须在 Slint 那一侧，而 Slint 的模型只能由宿主持有，插件拿不到。

**倾向的裁决：改写后接受**

依据 §4.3 第 4 条：

- **需要宿主内部状态**：Slint 模型在宿主进程里，只有宿主能让 `ListView` 在本地虚拟化。
- **通用基础能力**：媒体库、历史、书签、日志、搜索结果，以后的长列表插件都会遇到；让每个插件各自用窗口方式加占位来近似，就是重复造轮子。

改写的要点：

- 不限于面板：窗口、面板、设置页都能用。
- 行只读：插件是唯一的数据源，宿主不回读。
- 整体替换是原子的，不会露出半排序的列表。
- 选中等易变状态不放进行里（见下面的建议）。

**如果接受，形态**

- **API**（SDK 新增，纯新增）：
  ```ts
  const rows = panel.model<PlaylistRow>("rows"); // View.model(property)：该属性改由宿主持有的列表
  await rows.replace(items);                     // 整体替换（排序、重新载入）；SDK 分块发送，宿主收齐后一次换上
  await rows.splice(start, deleteCount, ...items);
  await rows.set(index, item);                   // 一行
  rows.length;                                   // SDK 侧镜像，只读
  rows.dispose();                                // 属性回到普通属性，变为空列表
  ```
  - 同一属性再用 `set`、`bind` 会抛错（`conflict`）。
  - 调用按顺序执行，不合并，与 `Latest` 不同。
  - 一次调用失败：SDK 用 `replace` 重新同步整个列表，并交给 `onError`。
  - Slint 侧照常声明 `in property <[PlaylistRow]> rows;`，配 `ListView`。行里的 `in-out` 编辑不回传，文档写明「行只读，编辑走回调」。
- **wire**：
  - `ui.model.splice {view, property, start, deleteCount, rows}`
  - `ui.model.set {view, property, index, row}`
  - `ui.model.replace {view, property, rows, final}`：分块暂存，`final: true` 时换上
  - `ui.model.release {view, property}`
  - `view` 为窗口 id、`$mount` 或 `$settings:<id>`，与 `ui.callback` 相同。
  - 宿主持有 `VecModel<Value>` 的 `ModelRc`。视图重建（别的面板挂载、换主题、浮动与停靠互换）时把同一个模型装回新实例；停靠和浮动两份实例共用一个模型。
- **校验**：
  - 每块 `rows` 按 D1 的属性值上限计（一块 ≤ 4096 行、≤ 16384 个值、≤ 1 MiB 文本，且整条请求 ≤ 1 MiB）。
  - 新行的形状必须与已有的行一致（宿主现有的 `fits` 规则），否则 `invalid`。
  - 下标越界为 `invalid`。
- **上限**：
  - 每个列表 ≤ 100 000 行、文本合计 ≤ 32 MiB；每个插件 ≤ 8 个列表。超出为 `limit`。
  - 内存估算：20000 行 × 每行约 300 B，约 6 MB。
  - 首次填满 20000 行约 12 块，每块约 6 ms UI 线程时间，分摊在多帧里。
- **生命周期**：
  - 列表属于视图：窗口关闭时释放，下次 `show` 时 SDK 用镜像重新 `replace`。
  - 面板卸载、作用域释放、插件停用时一并释放。
  - 旧播放器不认识这些方法，靠 `minPlayerVersion` 挡住，SDK 文档写明需要的版本。
- **TestHost**：
  - `host.window(id).props.rows`、`host.mounted().props.rows` 给出应用 splice 后的完整数组。
  - 执行同样的校验和上限。
  - `host.sent("ui.model.set")` 用来断言「选中一行只改一行」。
  - 视图重建不模拟。
- **对作者的设计建议**：
  - 选中、当前项不要放进 `PlaylistRow`：全选 20000 行会变成 20000 次 `set`。
  - 用 `current-row: int`，选中用一个同样由宿主持有的 `[bool]` 列表，全选时整体 `replace`。或者用区间（`[{from, to}]`），Slint 侧按 `index` 判断。
- **工作量（人工估计）**：
  - 宿主约 2 天：RPC、按视图登记、重建时装回、释放、上限、Rust 测试。
  - SDK 约 1 天：API、分块与重新同步、TestHost、TSDoc、reference、示例。
  - 播放列表迁移约 1 天（作者）：去掉窗口逻辑，`ScrollView` 改 `ListView`，选中改为单独的列表。
  - 另需协调者在 Windows 真机上做一次 GUI 回归。
- **排期**：D1 先发（SDK 1.5.10 与对应宿主）。R3 等作者第 3 步的答复后再裁决，作为单独一轮实现；与 D1 不合并，因为 D1 是缺陷修复，不应等待新能力。

**请作者在第 3 步回答**

1. 是否接受上面的形态，特别是「行只读」和「选中不放进行里」。
2. 在 A1 + A2 + A3 落地之后，真机（Windows，20000 行）上远距离跳转的占位或空白持续多少帧。如果只有 1–2 帧、而且作者认为可以接受，那么 R3 改为拒绝，以 A1–A3 作为替代方案。
