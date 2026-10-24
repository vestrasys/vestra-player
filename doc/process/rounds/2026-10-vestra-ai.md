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

- 2026-10-24 服务商：**不为单个服务商做特例**。
  - 下拉列出 pi 支持的全部主流服务商（只限 API key 方式），模型列表来自 pi 的目录；
  - 另加「自定义 · OpenAI 格式」和「自定义 · Anthropic 格式」两项，填 Base URL 和 key，尽量自动获取模型列表，失败时手填模型 id；
  - 每个服务商的 key 分别存进钥匙串。

- 2026-10-24 消息渲染：**用 Markdown，不做 HTML+JS**。HTML 要在宿主里嵌浏览器内核，改动和安全面都太大。流式显示按约 100 ms 节流、整条消息重新解析。工具调用显示为**一行滚动状态**：最新一条原地替换上一条，这一轮结束后可以展开查看全部，必须实时刷新。回复文本的流式显示如果有困难，可以先不做。

- 2026-10-24 导出源码 zip：**自己的仓库加 registry 登记**。zip 是一个完整的插件项目（README、测试、发布用 CI），可以直接建成用户自己的 GitHub 仓库；上架时到官方插件仓库提交 `registry/<id>.toml` 登记。插件仓库那边需要补上「第三方登记、从源码构建」这条路，属于插件仓库 infra 的工作，单独立项。

## 遗留

- `src/packaging/third-party.toml` 里 workshop 一条只列了 4 个包，实际打进去的有 61 个；应改为指向 `dependency-inventory.json`。归打包维护。
- 「重试」会把上一条用户消息再发一遍，记录里会出现重复（pi 1.4 没有 regenerate）。
