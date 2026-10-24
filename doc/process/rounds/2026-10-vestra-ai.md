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

- [ ] 纵向切片：菜单 → 无 key 横幅 → 设置页保存 key → 新对话 → 真实服务商流式回复（纯文本）→ write 工具卡片 → 文件出现在 `work/`；包括停止、错误条、新对话/切换会话。不需要新 API。
- [ ] 第 2 阶段：Markdown 消息（等 R-a）
- [ ] 第 3 阶段：插件工具、安装、导出 .vxp/zip（等 R-h）
- [ ] 真机验收：真实 key、中文输入法下按 Enter

## API 请求

- **R-a**：`MarkdownRef` 可以放进列表行；新增一个不带滚动的 `MarkdownBlocks` 组件。挡第 2 阶段。维护者答复：待定
- **R-h**：在已安装的播放器里也能跑 SDK 的 check/pack（`@vestra-player/sdk/tooling`）。现在 `stage-runtime.mjs` 不带 `bin/` 和 typescript。挡第 3 阶段。维护者答复：待定
- **R-c**（以后）：代码块的复制回调。
- 不提的：R-b、R-d、R-e、R-g；R-f 已证明不需要。

## 待用户确认

- 导出源码 zip 的用途：官方插件仓库的 `plugins/` 只收 `vestra.` 开头的 id；第三方插件要走 `registry/<id>.toml`，而这条路要等「从源码构建」上线后才开放。
