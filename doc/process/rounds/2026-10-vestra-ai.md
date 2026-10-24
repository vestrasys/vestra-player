# 2026-10 轮：Vestra Player AI（插件工坊改造）

开始于 2026-10-24。需求和用户裁决见 [vestra-ai.md](../../product/vestra-ai.md)。**本文是这一轮的唯一进度来源。**

## 作者方案（2026-10-24，协调者已审）

- **manifest**：id 保持 `vestra.workshop`（改 id 会换安装实例和数据），显示名改为 Vestra Player AI；能力加上 `secrets`、`ui.settings`、`clipboard`。
- **进程结构**：插件进程负责界面、会话元数据、密钥、设置和特权工具（typecheck/pack/validate/trial/install/导出）。每个运行中的会话一个 agent 子进程：`spawn(process.execPath, …)` 加 IPC，跑 pi-coding-agent 1.4.0（pi-ai、pi-agent-core 同版本，都锁精确版本）。
- **子进程权限**：
  - `--permission --allow-net`；
  - 可读：插件包目录、本会话目录、SDK 类型目录；可写：只有本会话目录；
  - 不允许子进程、worker、addons、wasi；
  - 环境变量：`PI_OFFLINE=1`、`PI_TELEMETRY=0`、`PI_PACKAGE_DIR`。
  - 已用探针实测：越界读写、spawn 都被拒，IPC 正常。
- **pi 的配置**：内存凭据（不写 `auth.json`）、内存设置；ResourceLoader 什么都不自动发现；系统提示和 SDK 文档由插件提供。
- **key 的传递路径**：钥匙串 → `secrets.get` → IPC → 子进程内存。不经过环境变量、命令行参数或文件；测试断言会话 JSONL 里没有 key。
- **工具**：
  - read/write/edit/ls/find 用 pi 的工具定义，但注入我们自己的文件层：限定在 `work/` 内，判断用 realpath；符号链接、以 `.` 开头的路径分量一律拒绝；单文件 ≤1 MB。
  - grep 用纯 JS 实现（pi 自带的会 spawn rg）。
  - 插件相关的工具（scaffold/typecheck/check/trial/install）经 IPC 交给插件进程执行；install 需要先在界面里确认，再经过宿主确认。
- **打包**：pi 全量打包 14.2 MB，minify 后 7.6 MB；目标是 worker ≤5 MB（去掉 tui/CLI/bedrock/jiti/highlight.js）。
- **会话目录**：`<data>/sessions/<sid>/{meta.json, *.jsonl, work/<plugin-id>/, dist/<id>-<ver>.vxp}`。

## 进度

- [x] 纵向切片代码完成（2.0.0，PR 待合并，2026-10-24；pi bundle 4.8 MB，vxp 5.1 MB，测试 18/18）——待真机验收：菜单 → 无 key 横幅 → 设置页保存 key → 新对话 → 真实服务商流式回复（纯文本）→ write 工具卡片 → 文件出现在 `work/`；包括停止、错误条、新对话/切换会话。不需要新 API。
- [ ] 第 2 阶段：Markdown 消息（等 R-a）
- [ ] 第 3 阶段：插件工具、安装、导出 .vxp/zip（等 R-h）
- [x] 真机验收（2026-10-24，MiniMax-M3.1-Flash，「自定义 · OpenAI 格式」）：2.0.3 已通过——
  - 回复、会话、工具调用（write/read/ls）、滚动状态行、展开、获取模型、旧设置迁移；
  - 拒绝钥匙串授权时插件照常运行，有提示条，不能发送；
  - key 不落盘，会话目录里没有隐藏文件，只有一个 `--permission` 子进程。
  - 还没测：中文输入法下按 Enter、停止。

## API 请求

- **R-a**：`MarkdownRef` 可以放进列表行；新增一个不带滚动的 `MarkdownBlocks` 组件。挡第 2 阶段。
  - **维护者答复（2026-10-24）：改写后接受。**
  - `MarkdownRef` 可以出现在属性值的任意深度，所在的 Slint 类型必须是 `[MarkdownBlock]`；新增 `MarkdownBlocks` 组件（不滚动，高度随内容），可以选择打开 `code-copy`，通过 `copy-code(string)` 回调。
  - R-b 不提供 API，由宿主内部完成：按指纹缓存、未变的文档不重新解析、块模型原地 diff、追加行时原地 push。
  - 嵌套的 `ImageRef` 明确报 `invalid`。
  - SDK 1.5.4，排在 1.5.2、1.5.3 之后。
  - **作者：接受。**
- **R-h**：在已安装的播放器里也能跑 SDK 的 check/pack（`@vestra-player/sdk/tooling`）。现在 `stage-runtime.mjs` 不带 `bin/` 和 typescript。挡第 3 阶段。
  - **维护者答复（2026-10-24）：拒绝，给出替代方案。**
  - 不该让每个用户的 runtime 都多背约 20 MB（gzip 后约 3.6 MB），而用到它的只有一个插件。
  - 替代方案：插件包里自带 SDK 的 `bin/`、`dist/`、typescript 和 `@types/node`（现在的工坊已经自带 tsc），用子进程跑 `vestra-plugin check|pack`，再交给已有的 `packages.validateArchive`、`trial`、`install`。
  - 如果确实需要结构化诊断，再给 CLI 加 `--json`（SDK patch）。
  - **作者：接受替代方案。**如果 SDK 构建跳过 vendor 里 `node_modules` 布局的问题无法解决，作者会按迁移阻塞报告。
- **R-c**（以后）：代码块的复制回调。
- 不提的：R-b、R-d、R-e、R-g；R-f 已证明不需要。

## 用户裁决

- 2026-10-24 **试载功能整体删除**（宿主、SDK、文档），不保留弃用期。核实过只有 AI 插件调用 `candidate.trial()`：商城、侧载、插件管理、SDK 示例和插件仓库都不用；info 和插件管理两个插件只对 origin 为 trial 的情况有显示文字，一并删掉。AI 插件的流程改为「写 → 类型检查 → 测试 → 检查 → 安装（可回退）」。R-i、R-j 作废。SDK 1.5.8 移除（patch），`apiVersion` 仍为 `vestra/1`。

- 2026-10-24 **编程套餐、订阅类服务商一律不放进列表**（Kimi For Coding、Z.AI/智谱 Coding、Qwen/小米 Token Plan、OpenCode Go 等）。这些服务商的政策会变，我们来不及跟进，用户可能因此被封号。需要的用户可以自己用「自定义 · OpenAI/Anthropic 格式」填 URL 接入，后果自负。以后再说。（调查结论：pi 发送的 `User-Agent: pi/<ver>` 头和遥测设置无关，我们绝不能改它。）

- 2026-10-24 服务商：**不为单个服务商做特例**。
  - 下拉列出 pi 支持的全部主流服务商（只限 API key 方式），模型列表来自 pi 的目录；
  - 另加「自定义 · OpenAI 格式」和「自定义 · Anthropic 格式」两项，填 Base URL 和 key，尽量自动获取模型列表，失败时手填模型 id；
  - 每个服务商的 key 分别存进钥匙串。

- 2026-10-24 消息渲染：**用 Markdown，不做 HTML+JS**。HTML 要在宿主里嵌浏览器内核，改动和安全面都太大。流式显示按约 100 ms 节流、整条消息重新解析。工具调用显示为**一行滚动状态**：最新一条原地替换上一条，这一轮结束后可以展开查看全部，必须实时刷新。回复文本的流式显示如果有困难，可以先不做。

- 2026-10-24 导出源码 zip：**自己的仓库加 registry 登记**。zip 是一个完整的插件项目（README、测试、发布用 CI），可以直接建成用户自己的 GitHub 仓库；上架时到官方插件仓库提交 `registry/<id>.toml` 登记。插件仓库那边需要补上「第三方登记、从源码构建」这条路，属于插件仓库 infra 的工作，单独立项。

## 遗留

- `src/packaging/third-party.toml` 里 workshop 一条只列了 4 个包，实际打进去的有 61 个；应改为指向 `dependency-inventory.json`。归打包维护。
- 「重试」会把上一条用户消息再发一遍，记录里会出现重复（pi 1.4 没有 regenerate）。

## 第 3 阶段（2.2.0，分支 ai/stage3，2026-10-24）

完成：
- 知识目录：SDK 的 .d.ts、示例和 llms.txt；
- 系统提示重写，写明完成标准；
- 工具：scaffold / typecheck / test（`--permission` 子进程）/ check / pack / trial / install；
- 结果卡片：试载、安装、回退、导出 .vxp、导出源码 zip、打开文件夹；
- 作者前缀用 `local.` 或 GitHub 用户名；
- 评估：6 道题加 harness。

作者新提的请求（等维护者答复）：
- **R-i（宿主）**：试载只支持 commands/ui.window/events，其余能力返回 `unsupported`，大多数生成的插件因此不能试载。希望试载支持全部能力，停止时撤销全部资源。
- **R-j（SDK）**：`trial()` 的 STARTING 列表缺少宿主的 `"pending"`，状态于是不再更新，失败原因也丢了；作者暂时用 `packages.list()` 轮询顶替。
- **R-k（宿主）**：安装时宿主不弹自己的确认，只有插件侧的确认。
- **R-l（宿主）**：`validate` 的候选从不释放，每次运行上限 32 个；希望能释放候选，或提供只编译 Slint 的检查。
- **R-m（SDK CLI，不阻塞）**：在 `--permission` 下，`existsSync` 遇到无权限路径会抛错，所以 CLI 改用不受限子进程运行。
- 另外：`third-party.toml` 需要补上 vendor 进来的 SDK。

## 实战（2026-10-24，2.2.0，MiniMax-M3.1-Flash，作者 zysilm）

1. **计时器窗口**（`zysilm.timer`：菜单 + 窗口 + 按钮）
   - 约 1 分钟，24 次工具调用，0 次失败，一次通过：先读 llms.txt、.d.ts 和控件库，再 scaffold、写代码，typecheck、test、check 全部通过。
   - **试载失败**：卡片提示「试载没有出现在插件列表里」，原因未查明，可能和 R-j 有关。
   - 安装成功；菜单「工具 ▸ 计时器」可用，+1 计数正确。
2. **剩余时间 OSD**（`zysilm.remaining-time`：快捷键 Shift+R + ui.osd + playback.read）
   - 约 2 分钟，51 次工具调用，8 次失败，最后 typecheck、test（6 项）、check 全部通过。
   - 因为 R-i（试载只支持 commands/ui.window/events），没有试载。
   - 还没安装，也还没在真机上实际使用。
3. 第 3 题未做。

**观察到的问题**
- 回复中混有英文过程语（「I'll start by reading…」）。系统提示应要求过程说明也用用户的语言，或者干脆不输出过程说明。
- 8 次失败的原因还没逐条分析。
- 计时器的试载失败要查清原因。

### 维护者答复 R-i 到 R-m（2026-10-24）

- **R-i、R-j：不再适用**，试载已删除。
  - 实战中试载失败的根因：宿主把试载插件列为 `<id>.trial.<n>`，而插件按原 id 查找；另外 SDK 的「启动中」列表缺少 `"pending"`。计时器插件本身没有问题。
- **R-k：拒绝。**
  - 理由：`plugins.manage` 是受信任能力，各个入口在自己的界面里确认；宿主再弹一次会和商城等流程的确认重复。
  - 替代：保持插件侧的确认，列出 id、版本、能力。
- **R-l：按缺陷修复（播放器 0.4.2）。** 只保留最近 32 个候选；旧 token 安装时报 `stale`。「只编译 Slint 的检查」：拒绝。
- **R-m：按缺陷修复（SDK 1.5.8）。** CLI 遇到 `ERR_ACCESS_DENIED` 按文件不存在处理；类型检查在权限模式下仍需要 `--allow-child-process`。
- **SDK 1.5.8 / 播放器 0.4.2**（PR sdk#24、player#15）：删除试载，R2 `resolvers.fileTypes`，R-l，R-m。

## 实战第 2 轮第 3 题与 2.2.2（2026-10-24，作者）

第 3 题「速度微调」（`zysilm.speed-nudge`，2.2.1，MiniMax-M3.1-Flash）：69 次工具调用，28 次失败；typecheck、6 项测试、check 通过，安装成功，菜单「播放 ▸ 速度微调」在。

**28 次失败的分类**
- **23 次（ls 1、read 2、grep 20）：参考资料的绝对路径抄错。** 路径约 150 个字符，模型丢掉中间一段（`-Users-ziyang-Develop-vestra-player` 变成 `-Users-vestra-player`），同一个文件对一次错一次。原因在我们：提示词和 grep 结果里给的都是长绝对路径。连带影响：testing.d.ts 没读成，后面 3 次 typecheck 失败（`TestHost.state`、`player`、`playback` 都是猜的）。
- **3 次 typecheck、2 次 test**：正常迭代，其中 1 次 test 是 `Shift+]` 被 SDK 拒绝（见下）。

**2.2.2 的修复**
- 参考资料用短路径 `sdk/…`（`sdk/llms.txt`、`sdk/dist/plugin/types.d.ts`），read / ls / find / grep 都认，grep 结果也这样显示；`sdk/` 下的写入和删除一律拒绝。绝对路径仍然可用。
- 开场白（「I'll start by reading the SDK documentation.」）：提示词已要求过程说明用用户的语言，模型照样用英文写第一句。只靠提示词不可靠，所以两层：提示词加「第一次工具调用之前不输出文字」；界面上，一轮里第一次工具调用之前、只有一行（≤200 字符）的文字不显示，对话记录保留原文。之后的过程说明和不调用工具的回复照常显示。只按「位置 + 一行」判断，不做语言检测。代价：没有工具调用的回复，第一行要等第二行开始（或超过 200 字符、或回复结束）才出现。
- 快捷键：提示词写明符号键写实际输入的字符（Shift+] 写 `}`，Shift+[ 写 `{`），用户要的键原样实现。模型这次把 SDK 的报错「write the character Shift produces instead of Shift+]」理解成「去掉 Shift」，绑了 `]` 和 `[`，和 A-B 循环冲突，被播放器忽略（只在宿主日志里）。
- 冲突可见：公开 API 能查到（`bindings.list()` 的 `shadowed`），所以没有提请求。安装完成后插件读一次绑定表，被占用的按键写进卡片状态和 install_plugin 的结果，例如「快捷键没有生效：]（加快 0.25×）已被vestra.marks「设置 B 点」占用」，并让 AI 建议换键。为此 manifest 加了 `bindings.manage`（只读用）。

**作者新提的请求（等维护者答复）**

- **R-n（SDK CLI，不阻塞）：`build` / `pack` 在 `--permission` 下失败。**
  - 位置：`vestra-plugin build`（`bin/vestra-plugin.mjs` 的 `build()` 用 `fs/promises` 的 `cp` 复制包里的文件）。
  - 复现：`node --permission --allow-fs-read=<项目>,<SDK> --allow-fs-write=<项目>/dist,<tmp> --allow-child-process vestra-plugin.mjs build <项目>`，项目里有 `ui/main.slint` → `ERR_ACCESS_DENIED`（FileSystemRead，resource 是项目的上级目录）。原因：Node 的 `fs.cp` 检查目标的每一级上级目录（checkParentPaths），对子目录里的文件会一路 stat 到根目录。`copyFile` 和 `cpSync` 不受影响（已实测）。
  - 希望：复制单个文件改用 `mkdir(dirname, { recursive: true })` + `copyFile`，让 build / pack 也能在 `--permission` 下运行。
  - 影响：只是实现细节，不改契约。现在工坊只让类型检查（`check`）在权限模式下运行，check 和 pack 仍然不受限。
  - 验收：上面的命令 build 和 pack 都通过。
- **R-o（SDK 文案，不阻塞）**：`binding Shift+]: write the character Shift produces instead of Shift+]` 改成直接给出字符，例如 `binding Shift+]: write "}" (the character Shift+] types)`。字符依赖键盘布局时，至少给出美式布局的例子。模型照字面理解成去掉 Shift，绑了 `]`。
