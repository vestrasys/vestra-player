# 全插件真实交互：执行记录

日期：2026-10-17。用户实际打开文件闪退后新建 goal。目标为全部现有 9 个插件的已实现功能，不以先前 52 项 TS 测试、1 项 Rust 测试或 67 步回调场景代替真实用户路径。

## 责任与证据

- root：实际原生应用操作、窗口/系统打开保存对话框、全插件加载与各界面入口；记录截图及可复现操作，不伪造用户输入。
- api_maintainer：共享 SDK、Rust 宿主与运行时修复，单点构建；必要 API 改动先分类、版本与迁移，不让插件作者追着未说明的变化修改。
- acceptance：独立验收，[完整清单](interactive-acceptance.md)、源代码审查、自动回归与最终证据核验。UI 控制分时交接，禁止多个 agent 同时点击同一窗口。

当前独立清单包含 9 插件、68 个注册 action、30 个贡献菜单命令。可见 UI 与菜单分别验证；没有用户入口的测试 action 另列，不能把 RPC 调用算作实际点击。

## 初次实际失败

“打开文件”失败栈为 `i-slint-core-1.18.1/timers.rs:350: Recursion in timer code`，调用链包括 `apple_display_link::DisplayLinkTarget::tick` 和 `update_timers_and_animations`。维护者核对 rfd 0.15.4：同步 `pick_file` 调用 `NSOpenPanel.runModal`，嵌套 AppKit 事件循环重入正在执行的 Slint Timer。分类为宿主实现 bug，不是已确认的插件或公开 API 缺口。

真实右键画面没有菜单。Slint TouchArea 在任意鼠标按下时 GrabMouse，子控件截走事件，祖先 ContextMenuArea 无法处理右键。root 将菜单区域调整为内容上层的独立元素，等待真实鼠标复验；不凭源码修改宣布修复。

## 当前执行状态

异步原生对话框与右键路由已实现，root 已实际展开菜单、通过系统选择器添加文件播放插件、在真实 MP4 选择器等待至少 90.277 秒后确认并看到视频帧；这些仅是局部原生证据，全部 9 插件及剩余入口尚未通过。测试用原生 app bundle 位于已忽略的 `.cache/Vestra Interactive.app`；它只是复制当前 Rust binary、设置 bundle 身份及工具 PATH，便于 UI 自动化识别，不修改插件 API 或用 Web UI 替代 Slint。

候选修订 `probe-0.3.1` 保持 SDK 方法和类型，仅明确原生交互等待不受普通 30 秒 RPC deadline 限制，owner 停用和宿主关闭仍终止等待；旧基线快照保留。长驻留还复现了旧播放状态积压：返回后画面已动但进度暂时回放旧状态。runtime 已合并等待中的最新快照，修前实际 100 次旧状态回放、修后只处理最新状态，等待原生独立复验。文件播放插件错误信息被状态刷新覆盖的问题由功能审计者窄修，见[业务审计](functional-audit.md)。

共享非 GUI 检查退出 0：60 项 TS、1 项 Rust 对话框单测、1 项真实 Rust 播放后端测试，strict、依赖边界、格式、构建、Clippy、9 插件组合 UI 编译均通过。日志 `.cache/evidence/api-maintainer/interactive-shared-check.log`。这些不替代原生全功能验收。

## 实际环境授权等待

独立验收者通过 LaunchServices 重启原生 bundle 后，选中 manifest 并确认时挂起。维护者通过进程栈与 macOS `tccd` 日志确认：应用身份 `local.vestra.interactive` 正在等待 Documents 文件夹授权。选择单个 manifest 不等于授权读取相邻 `ui.slint`。这项环境权限等待与 Slint 定时器崩溃分开记录，不误判为另一个 API bug。

root 已向用户询问：允许这一本地实验应用访问 Documents，或不授权并改用独立测试目录打包。尚未替用户点击允许、扩大权限、杀进程或搬移路径掩盖现场。其他可独立推进的检查已完成；此前连续三个 goal 回合缺少此决定，曾将 goal 标记为 blocked，未标完成。

goal 恢复后首次只读复核：PID 38826 仍存在；系统日志在 2026-10-17 16:04:42 再次记录该应用的 `AUTHREQ_PROMPTING`，未看到授权结果；原生应用清单仍显示 Vestra Interactive 正在运行。恢复 goal 本身不视为同意扩大 Documents 权限，因此当前 goal 为 active、等待原问题的选择；没有重新执行已通过的检查冒充进展。权限路径确定后继续原范围的九插件原生全功能验收，不缩减清单或复用旧通过结果。

恢复后的第二、第三个 goal 回合均重新核对了同一个 PID 38826 及最新 `tccd` 授权日志：进程仍存在，未观察到授权结果，用户也未选择授权路径。第三回合满足再次阻塞的审计条件，goal 重新标为 blocked；全部九插件原生验收仍未完成，未变更成功标准。

用户随后明确表示“刚才已经允许了”，并询问为何不能代点。root 已接受这项明确授权：同一应用的 Documents 提示可以确认，不再重复询问。最新检查 PID 38826 已不存在，原生应用清单也不再显示播放器；尚不能据此判定发生了新崩溃。独立验收者接管 UI，重新启动、核对实际权限并继续全量交互；维护者另行检查旧预览日志中的 shutdown 错误是否需要修复，避免与 UI 操作互相干扰。

随后独立验收者通过 `src/scripts/preview.sh` 一次预载全部九插件，PID 47074，日志 `.cache/evidence/interactive/independent/preview-nine.log`。真实右键可见八个普通面板、播放列表及贡献菜单；Documents 等待没有再次阻止加载。九插件同时加载时，真实 Open 面板选择 demo.mp4、停留 46.6 秒后确认，主窗口显示正确路径、暂停按钮及 1/180s，视频实际播放；没有再次递归崩溃或积压旧状态。这是局部通过，不能覆盖其余所有功能。

用户接管窗口试用后表示“基本没什么问题”，要求先 commit。验收者停止自动点击以免干扰用户，保留窗口；提交当前源码、文档和已有证据。用户试用反馈与独立逐项验收分开记录，完整原生清单仍有待执行项，不将本次提交描述为全量验收完成。旧 preview shutdown 堆栈已由维护者确认来自旧 runtime 显式错误日志，当前 closing 守卫回归证明 worker 退出码 0、stderr 为空且停止后无 UI RPC；没有因旧堆栈臆断新宿主崩溃。
