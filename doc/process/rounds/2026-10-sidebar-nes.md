# 2026-10 轮：侧栏页签与 NES 打开方式

开始于 2026-10-24。**本文是这一轮的唯一进度来源。**

## 侧栏页签

设计：[panel-slots.md](../../architecture/panel-slots.md)。用户已选定方案 A。

- [x] 第 0 步：挂载时直接报 `conflict`，不再连带激活失败；内置插件优先挂载（#2，真机验收通过）
- [ ] 第 1 步
  - [ ] 维护者：宿主、运行时、SDK 1.5.2
  - [ ] 播放列表作者：F6 改用 `list.toggle()`，有宿主页签条时隐藏自己的页签条
  - [ ] 真机验收

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
