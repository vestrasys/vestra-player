# V12 修后必要场景独立复验

2026-10-17，操作者 `rebuild_bootstrap_review`。仅此操作者使用CUA；root和维护者不并发操作GUI。绑定维护者明确存活的唯一 `local.vestra.rebuild.shared-ui-v12`、原PID88534。未构建、未改共享代码或仓库样例；故障注入仅本次staged包副本，结束已恢复。[启动](launch.json)、原始[stderr](stderr.log)/[stdout](stdout.log)、故障原文/备份及提取的公开状态均在本目录。截图在本轮CUA会话，未声称存在仓库原始截图。

## 有限结论

**本次必要修后场景通过：实际缩窗空间策略、恢复选择、全屏几何、真实栏内焦点/菜单抑制、临时侧栏与普通偏好分离，以及清理失败错误实际送达。** 不追改V7/V10失败，不据此宣布完整M1/产品/共同基线通过。启动还出现一次公开window-policy错误，见下文。

## 观察与公开事实

| 场景 | 真实操作和独立结果 |
| --- | --- |
| 基线与实际缩窗 | 首次大窗截图2200×1504，根逻辑1100×720、bar60。选择Second row后拖右下角缩小：实际截图1536×1046，公开logical768×491/canvas752×353；侧栏真实消失，preferred=true/effective=false/reason=space，sidebarWidth=0。没有伪造viewport或调用其他UI自动化 |
| 放大恢复 | 从实际缩小窗拖回大窗，侧栏重新可见，`second: Second independent row`保留；52.28秒/frame1570，同source1。没有额外点击侧栏去恢复选择 |
| 全屏修复 | 本次通过视图菜单Toggle fullscreen进入。真实截图2940×1846，视频完整铺到可用区域并按contain留正常黑边，未再出现V10白色外区/左残影。公开logical1470×923、canvas同1470×923，底栏overlay不扣canvas高度。截图独立支持实际布局恢复；不单靠fullscreen请求布尔 |
| 真焦点抑制 | Tab活动使栏恢复。为避免3秒闲置在两次工具调用间再次隐藏，同一CUA调用获取新鲜完整AX后点击Bar focus lock并输入`held`，AX明确焦点在该LineEdit。随后明确静置4秒，截图栏仍可见、文本输入仍focused；公开effective=true/overlay=true/nextDeadlineMs=null |
| 放焦与idle | 点击画布后静置4秒，栏从原生树消失；随后活动/菜单又能恢复。不是通过程序硬写focused/dragging锁值冒充真实交互 |
| 菜单抑制 | 打开共享菜单并明确静置4秒，截图菜单和底部栏均仍可见，视频运动图案继续变化。此项为抑制idle窄复验，不重复V8完整菜单测试 |
| 临时sidebar与偏好 | FS进入时侧栏暂隐；通过FS专用命令展开侧栏，原生树显示且second选择保留。再将普通sidebar偏好切false，FS临时展开仍effective=true但preferred=false；Escape退出后侧栏消失，普通偏好false得到恢复。随后显式普通show恢复侧栏，为清理case准备 |
| 普通恢复与媒体 | 退出FS恢复普通窗口、bottom及输入`held`，同session1，186.72秒/frame5599。整个窗态case未调用pause/seek/open；真实视频图案持续变化，公开来源摘要仍同generation/cycle。最后清理case验证旧栏可用时才明确点击Pause |

F11在原生resize后的首次尝试没有可见转换，因此本次只确认菜单方式进入全屏，没有把那次键盘尝试写成成功。跨3秒按住拖动未独立验证；真实输入焦点和菜单抑制已验证。没有声音采集、设备音频连续性或跨平台/缩放结论。

## Owner清理反例的修后闭环

恢复普通sidebar后，将staged底栏 `bar.slint` 写入本目录保存的 `bar.slint.review-fault`（语法错误并缺原导出）。点击Disable sidebar后，原生侧栏消失，226.03秒/frame6780仍播放。原始stderr：

- 20551行有宿主 `host.error`，code=`ui-owner-cleanup`、owner3，包含该实际坏bar文件的Parse error/缺失导出诊断；
- 20552行有真正Node JSONL接收的 `event:"host.error"`；
- 20553行起有消费者 `observed host.error`。

因此这次不仅即时hide，也实际尝试了系统重编并把失败送达订阅端，补足V8/V10未证的路径。旧底栏随后仍能暂停到236.23秒/frame7087。staged原文已恢复；backup/fault保留。逻辑撤销、旧可用UI保留、失败反馈分别有证据，不把“错误已报告”说成“新壳清理重编成功”。

## 保留的诊断与范围

原始stderr启动4/8/9行出现一次 `window-policy` / `invalid viewport or fullscreen-only request outside fullscreen`，事件确被消费者收到。发生在后续有效几何/挂载之前，本次后续没有反复出现；原因未独立确定，不静默删除或宣称全程零错误。[收到错误提取](received-host-errors.json)同时保留这项与预期清理故障。

公开状态提取为[window-states.json](window-states.json)，来源提取为[source-summary.json](source-summary.json)；它们源自原始JSONL，不替代上述原生观察。没有强造旧shell迟到回调、队列压力、窗口销毁期间安装任务或最终产品控件布局等范围。

最后读取新鲜AX Close索引10，单独click(10)，未追加getApp/AX。只读ps确认原88534及本stage所有进程均不存在，无无参重启。没有host退出码句柄，不虚报exit0。GUI已交还；不再启动补测。
