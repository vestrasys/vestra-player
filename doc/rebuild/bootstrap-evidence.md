# 阶段 A 作者验证记录

2026-10-17，操作者：全新 API maintainer `rebuild_api`。所有源码/媒体/测试均为本轮新建，未读取旧实现或旧测试。此记录不是六插件验收，也不是独立GUI验收。

## 版本、检查与入口

- macOS Apple Silicon；Homebrew Rust/Cargo 1.99.1，Node 26.12.0，mpv/libmpv 0.41.0；Slint/Slint interpreter锁定1.16.1，rsmpv锁定0.2.1。
- 最后源码检查：`cd src/host && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo build --locked`，整体退出0。
- `cd src && npm run format:check && npm run check && npm run boundaries`，整体退出0。
- 启动与已知缺口见 [bootstrap](bootstrap.md)；初版行为见 [API](api.md)，工程边界见 [规范](engineering.md)。

## 控制/render线程修正与真实后端

最初将同步libmpv控制与software render放同一工作线程。首次5秒集成没有成功标记，判为失败；随后延长诊断：开始约0.73s/frame23，45次亮度循环结束时8秒测试片已到EOF、暂停、累计frame85。不能用“session仍为1”掩盖调参明显变慢，也不能将此版本作为能力证据。

修正为同一个Arc<Mpv>、独立控制线程和独立safe software RenderContext线程，控制响应/事件与渲染不再互相等待。重新执行：

```sh
cd src/host && cargo build --locked
cd .. && npm run verify
```

本轮新建24秒640×360/30fps testsrc2+440Hz正弦媒体，经实际libmpv软件渲染成960×540帧。保存的 [作者结果](bootstrap-evidence/headless-result.json) / [原始stderr](bootstrap-evidence/headless.stderr.log)：45次连续brightness设置约1527ms；position 0.8333→2.2667s；videoFrames25→69；session均为1，未暂停。随后pause时钟稳定、seek恢复、无效speed返回invalid、真实独立helper加载/重复拒绝/停用/dispose与其他会话继续均通过。该次命令退出0。

这里的视频帧是renderer完成计数；声音使用`--silent`即mpv null audio sink，未声称扬声器输出/设备延迟已验收。未实现的全局菜单更没有菜单停留连续性证据。

## 管道背压修正与独立复跑边界

独立只读审查指出同步stdin write/flush可能阻塞Slint和让停用超时失效；已改为每实例64条有界outbox + 独立writer线程。UI只try_send，队列满停用该实例；后台Drop发送stop也不阻塞，300ms后kill/wait。

初次stalled fixture暂停stdin并发6000个请求，验证其他播放继续、运行列表移除及PID消失；但Node可能因未settled顶层await自行退出，该作者日志不能单独证明强制kill。最终fixture已添加setInterval使进程保持活跃，用于独立审查者最后复跑300ms kill/reap路径。最终源码冻结后，独立审查者执行`npm run verify`退出0，work=`vestra-stage-a-Wen50f`；其 [结果](bootstrap-evidence/reviewer-headless-result.json) / [stderr](bootstrap-evidence/reviewer-headless.stderr.log)已保存。45次1495.63ms，session1，0.7667→2.3s、frames24→70、42个states；验证stalled PID已消失，且保持进程活跃的最终fixture日志不含旧版unsettled top-level await警告。不以早期PID消失偷换强制清理证据。

示例dispose现仅释放本地callback，不在runner停止后请求unsubscribe。宿主清理订阅，避免正常退出必报disabled。console.log/info/debug均重定向stderr。activate未返回时停用会直接终止进程，不能承诺尚未返回的dispose被调用。

## 作者原生GUI实证（CUA）

所有物理UI交互仅用CUA，root分配独占UI时段。没有操作用户其他应用，没有复制旧bundle。Cargo裸可执行程序不被CUA识别，因此将本轮可执行文件复制到自己的开发.app，使用自建Info.plist和ad-hoc codesign；不是用户分发包。新bundle身份避免LaunchServices无参数实例与命令行媒体实例混淆。

第一轮自己的`local.vestra.rebuild.bootstrap`，CUA识别sample原生窗口，点击Toggle pause后状态停于5.63s/frame173；关闭sample，CUA看到宿主image和同一session状态，原生截图显示视频色条/嵌入时间码。该轮仅证明按钮回调/真实画布；未声称恢复按钮或连续拖动已独立验收。[Slint内部截图](bootstrap-evidence/native-canvas.png)是此前同一路线的`Window::take_snapshot`产物，单独分类为程序渲染截图，不能冒充CUA屏幕截图。

最终live run采用唯一身份`local.vestra.rebuild.live-evidence`，执行session21635：

```sh
ffmpeg -v error -y -stream_loop 9 -i /absolute/new-test/clock.mp4 -c copy /tmp/vestra-bootstrap-long.mp4
cd src/host
VESTRA_DEMO_MEDIA=/tmp/vestra-bootstrap-long.mp4 \
  './target/debug/Vestra Live Evidence.app/Contents/MacOS/vestra-host' \
  --silent --seconds 45 --plugin ../examples/capability-demo
```

.app的Contents/MacOS/vestra-host来自本轮cargo build；Info.plist含上述CFBundleIdentifier、CFBundleName=`Vestra Live Evidence`、CFBundleExecutable=`vestra-host`、CFBundlePackageType=`APPL`、CFBundleVersion=`0.1`、NSHighResolutionCapable=true；运行前`codesign --force --deep --sign -`本轮.app。

CUA工具原始观察（截图已在本轮工具对话输出，不冒称落盘原始截图）：

| 步骤 | 可见结果 |
|---|---|
| getApp唯一bundle ID | sample窗口，session1，0.80s/frame25 |
| AX setValue slider为18 | 控件值18，4.47s/frame135；仅证明AX值与持续状态，不冒称连续拖动/像素效果测量 |
| 点击sample关闭按钮 | 主宿主仍显示，session1，8.00s/frame243 |
| CUA screenshot主宿主 | 真正image画布有色条/移动图案，视频时间码10.033；诊断9.97s/frame303 |
| 后续AX+screenshot | 视频图案实际改变，时间码14.900；诊断14.93s/frame449，同session1 |

这证明实际mpv帧在Rust/Slint管理的原生窗口持续显示，关闭演示工具没有停播。未用外部mpv播放器窗口代替。该run45秒自动退出，exec最终退出0；[stdout](bootstrap-evidence/cua-live.stdout.log) / [stderr](bootstrap-evidence/cua-live.stderr.log)保存本轮进程日志（stderr仅runtime.ready）。完成后不再启动GUI。

阶段A不覆盖产品全局菜单、常驻播放栏布局、嵌入工具、原生文件对话框、处理链所有权、字幕/导出任务隔离或NES/工坊。它们是阶段B/C的真实缺口，不能从本记录推断通过。
