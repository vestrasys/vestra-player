# 阶段 A 独立审查

审查者：rebuild_bootstrap_review；日期：2026-10-17。审查范围仅限本轮新 `src/`、`doc/rebuild/` 与获准的产品要求/工作流，以及本轮独立执行产生的证据。未读取旧实现、Git 历史或历史产物；未构建、未启动 GUI。维护者冻结源码并确认二进制就绪后，独立运行了 headless 集成验证。

## 当前判断

**阶段 A 可进入统一产品交互设计。** 已发现的基础风险已通过小范围代码或契约修正收敛，独立 headless 验证通过。真实 libmpv 长期会话、Rust 所有的 RGB 缓冲区、Slint Image 接入、独立 Node 进程和公共 SDK 可作为设计的能力输入。它还不是产品插件共同基线，也不代表六探针或最终产品完成。

## 阶段 A 问题与处理结果

1. **插件传输可能阻塞 UI 与停用。** 当前 `plugins.rs::Plugin::send` 对 `ChildStdin` 同步 write/flush；`main.rs::broadcast/send` 从 Slint timer 路径调用。可信插件发生长时间同步计算而不读取 stdin 时，事件可填满管道并卡住画布。`Plugin::drop` 先同步发送 runtime.stop，再启动 300ms deadline，因此同一故障可能使 kill/wait 永远无法执行。需要将阻塞传输移出 UI，并确保停用的期限不受写管道阻塞影响；核验应覆盖不读输入的插件，而不仅正常 dispose。
2. **样例正常停用会发出被禁止的 RPC。** `runner.ts::stop` 先设置 stopping，再调用 dispose；示例 dispose 的 unsubscribe 会请求宿主，必以 disabled 失败。宿主本来已清理订阅，样例应只清本地资源，或 SDK 明确提供本地清理方式。不能让公开生命周期例子以预期错误结束。
3. **后端事件 session 标签不能解释为来源关联。** `backend.rs` 批量执行 open 后才 poll_event，积压的旧媒体事件将被打上当前 session。初版可明确诊断标签仅表示轮询时当前会话、不得驱动新媒体成功/失败判断；稳定异步状态与事件来源关联应进入 B/C。若声称已具备来源隔离，就必须修正实际行为并验证连续切换媒体。
4. **运行时日志及激活中停用的边界。** 当前仅重定向 console.log，普通 console.info/debug 仍可能污染协议 stdout；应重定向或明确限制。stop 与 await activate 可并发，activate 尚未返回时 dispose 尚未注册，不能承诺执行它；可在初版明确激活中停用直接结束进程，而不增加复杂两阶段协议。

以上均已向维护者提供代码路径和复现条件，并复读修正后的源码/契约：

- 传输已改为每实例64条有界 outbox，UI 只 try_send；专用 writer 执行阻塞管道写，队列满/断连停用实例。drop 非阻塞尝试发送 stop 后立即计算300ms期限，再 kill/wait。原有 UI 和 stop 期限受阻塞管道影响的路径已关闭。
- 示例 dispose 不再发 unsubscribe RPC；在途 UI 请求的预期 disabled 错误被明确处理。
- 后端诊断字段改为 currentSession，文档说明它只表示轮询时当前会话，不能证明事件来源隔离。稳定异步结果模型列入 B/C。
- console.log/info/debug 均重定向 stderr；契约说明 activate 未返回时停用不能保证调用尚未注册的 dispose。

本次复审未发现必须在进入设计前继续扩容实现的缺口。此结论不要求把后续全部任务/菜单/处理资源能力提前实现。

## 已确认的静态基础

- 第一方 crate 同时设置 `#![forbid(unsafe_code)]` 和 Cargo lint；代码通过 rsmpv safe software render 接口持有一个 Mpv，控制与 renderer 分线程，UI 只消费最新 CPU 帧。未看到外部播放器窗口或每次参数更新 loadfile 的替代路径。
- `playback.set` 直接在长期 Mpv 上设置受限属性；`open` 才增长 session。真实连续提交是否顺畅仍须当前运行证据支持，静态结构不能证明显示性能。
- Rust 以 generation 标识插件实例；窗口键、RPC 回包和回调按实例路由。disable 移除窗口与订阅，不依赖插件自觉释放。公共播放参数是共享状态，不是插件拥有的可撤销处理资源，文档已明确。
- renderer 创建后的正常 Result 早退仅在初始化握手失败处，此时 renderer 已失败或即将退出；成功初始化后的控制循环没有 Result 早退，单次 execute 失败回包后继续。正常停止 signal renderer 后 join。未发现可复现的控制错误使活 renderer 脱离所有权路径；初始化失败后仍显式 join 属可改进的清晰度，不据此阻止阶段 A。
- Node 是独立进程，插件只消费公共 Host；manifest、能力声明、包内文件路径有校验。文档诚实声明可信运行时并非安全沙箱。
- Slint 直接使用 interpreter、widgets、properties/callbacks；`ui::compile` 拒绝多导出组件，示例仅导出一个 Window。不会静默从多组件中任选一个作为工具。
- open 接受与解码完成、set 接受与屏幕呈现、约100ms状态延迟、timeout不等于取消等限制已有记录。

## 交给阶段 B/C 的能力设计

全局菜单树、常驻播放栏挂载、工具/侧栏共存、焦点与快捷键、文件对话框及关闭通知应由产品设计提出，再补公共设施。首次 Slint 编译在 UI 线程这一限制必须进入打开工具的实际行为设计和验收。

处理链与可撤销资源、字幕异步来源隔离、截图/导出任务及取消、NES 自有影音时钟、Pi 工坊真实校验/回退都尚未实现。它们阻止发布最终产品共同基线，但不阻止在最小宿主可信后开始统一交互设计。原生菜单驻留期间画面/音频连续性必须后续真实验证，不能借 headless 渲染帧计数追认为通过。

## 证据边界

独立执行命令：在 `src/` 执行 `npm run verify`，退出码0。本轮工作目录 `.../vestra-stage-a-Wen50f`，使用当前新测试生成的24秒视频/音频媒体、独立 Node 消费端和实际 libmpv；headless 使用 `--silent`，不构成音频设备听验。已核对维护者复制的[独立结果](bootstrap-evidence/reviewer-headless-result.json)和[原始stderr](bootstrap-evidence/reviewer-headless.stderr.log)与本次工具输出一致；stderr明确含 full channel 背压诊断，未含旧fixture的 unsettled await 警告。

45次亮度提交用时1495.63ms；session保持1，position从0.7667前进至2.3秒，软件完成帧数从24增至70，收到42次状态事件。测试同时检查暂停时钟稳定、seek、非法speed拒绝、重复加载拒绝、helper正常dispose以及独立停用不重置播放会话。

当前 stalled fixture 在暂停读取 stdin 后发送6000个请求，并设置 interval 保持 Node 活跃；独立测试通过了 outbox满后实例移除、其他播放继续、进程最后不存在的断言。`stalledReaped:true` 是脚本字段名；PID不存在本身不单独证明某个强杀分支执行。kill/wait路径的保证同时来自本次静态复审；不把早期可能因 unsettled await 自然退出的旧fixture结果追认为此项证据。

独立查看了维护者保存的 `bootstrap-evidence/native-canvas.png`，可见测试媒体、Slint画布和宿主诊断。它是 Slint 内部快照，不冒称原始 CUA 屏幕截图。已复读[作者原生验证记录](bootstrap-evidence.md)：CUA实测主画布同session1从9.97秒/frame303至14.93秒/frame449，媒体时间码10.033至14.900且像素改变，唯一live bundle的45秒运行退出0；该项操作者是维护者，我没有独立操作GUI。单张截图本身只证明显示，不能证明连续性。

类型检查、格式、clippy和构建由维护者执行，本审查不冒称独立重跑。当前结论限于进入设计的最小基础可信性；原生菜单驻留、音频设备、所有产品工具连续操作、跨平台及最终插件组合仍待后续验收。
