# V8 独立原生挂载与菜单有限验证

2026-10-17，操作者 `rebuild_bootstrap_review`，root分配唯一CUA时段；root/维护者均未并发操作GUI。未构建或修改共享源码，未读取旧实现。只绑定维护者已确认真实存活的唯一 `local.vestra.rebuild.shared-ui-v8`，原host PID84720；完整启动信息见[launch.json](launch.json)，原始[stderr](stderr.log)/[stdout](stdout.log)同目录。截图由CUA直接输出在本轮会话，没有声称仓库保存了原始CUA截图。

## 有限结论

**V8 已通过下述实际挂载、菜单动作和失败保旧场景；不能据此宣布完整M1、全部交互规范或共同开发基线通过。** 底栏、侧栏、实际视频和同树菜单在同一原生宿主工作；首次未知编译仍同步占UI线程，准备期间的可接受边界与最终产品要求须后续验证。

| 实际动作 | 独立可见结果 |
| --- | --- |
| 原V7失败步骤原样复验 | 点击Second row显示 `second: Second independent row`；底栏Pause后停在31.95秒/frame964，按钮Play；Commands→播放→Pause/play后恢复Pause按钮，32.32秒/frame973，随后截图35.12秒/frame1058，两个挂载仍在、真实视频色条/运动图案可见 |
| 日常sidebar hide/show | 51.18秒/frame1539隐藏；63.21秒/frame1899恢复，第二行选择结果保留、底栏一直可用；source session始终1 |
| Model更新与rowId | Reverse model rows后顺序变为Third/Second/First；点击新首行显示 `third: Third independent row`，没有把原行索引误当稳定ID |
| 菜单驻留 | Commands菜单持续打开，两次CUA截图之间明确等待10秒：截图诊断82.30秒/frame2472→97.35秒/frame2923，菜单仍开、可见视频运动图案明显改变。声音连续没有独立采集证据 |
| 层级导航与关闭 | 点击根域进入子菜单；Down实际改变高亮，Return进入播放分支，Left回根，Escape关闭。视频右键打开相同五域树。Right未提供分支进入行为，不将其宣称通过；声明快捷键执行未验 |
| Owner正常停用 | 插件菜单Disable sidebar后侧栏消失、底栏和播放继续135.34秒/frame4063；Reload后重新显示，实际仍session1、182.82秒/frame5485 |
| 未知组件真实编译失败 | 在sidebar已空时Load syntax-error specimen；原始日志明确 `ui-prepare` Parse error/Syntax error，耗时157.15ms，非占槽冲突替代。旧底栏仍可暂停163.79秒/frame4915并恢复164.13秒/frame4926，当前来源未重开 |
| 剩余源码损坏时停owner | Reload sidebar后，仅将本次staging的底栏Slint改成保存于 `bar.slint.review-fault` 的坏文本，再Disable sidebar。侧栏立即从原生树消失，202.10秒/frame6062；旧底栏到219.03秒/frame6571仍能暂停。没有修改仓库样例。故障后恢复staged原文，原文备份及注入字节均已保存本目录 |

日常set/show没有触发新的准备事件，当前静态路由也只更新global/property、不编译。此项不宣称具有完整编译计数器或性能剖析证据。首次两个组件准备的作者日志为203.56/204.23ms，不冒称这些数字测量了最长可见停帧。

## 错误可观察性缺口

最后一个清理故障确实观察到死owner侧栏隐藏、旧底栏保用；但本轮例包没有订阅 `host.error`，系统清理的 `ui-owner-cleanup` broadcast没有落入现存stdio。**未独立收到该错误，不能写作错误反馈已通过。** 静态代码会发事件不等于实际订阅端收到了它。维护者已承诺后续让清理错误始终写stderr并补fixture订阅；该单个反馈项留后续必要复验，不为此重跑整套GUI。

未强造旧shell已排队回调在新shell提交后到达；shell_generation过滤目前是静态核对，不借正常操作追认为故障注入通过。未做queue压力实测、真实文件dialog重验、跨系统焦点/缩放、全屏恢复、字幕/NES/任务、设备音频或最终产品菜单布局验收。

## V7崩溃与V8正常结束的区别

V7原PID84548在操作者未触发Close/退出时，于Commands→播放→Pause/play后消失；随后出现无参空白进程。root已独立保存对应SIGSEGV及stage日志到 `../shared-menu-native-crash-v7/`。该失败保持有效，不能用后来的空白窗延续旧场景。

维护者将菜单invoke/back/dismiss/model变更延后到下一UI tick，并携shell_generation；V8按同一步骤复验恢复正常，以上case均在原PID84720完成。

V8最后由操作者明确点击Close。随后同一CUA调用执行getAXState，工具再次返回无参空白窗口；只读ps确认原84720已不存在、新84906指向同staged bundle。原stdio末尾有runtime.stop，未把这次明确Close后的重新拉起当作V7式未请求崩溃。操作者停止App/AX调用并交还GUI，root精确确认84906后TERM回收。没有原host子进程退出码句柄，故不虚报原宿主exit0。交还后未再次启动GUI或补测。

## 后续可追溯更正（保留原观察）

同日[V10独立复核](../independent-native-v10/review.md)在确认staged例包已订阅host.error、宿主有stderr诊断时，仍未实收同形cleanup错误。修前代码发现显式disable后Node EOF再次disable，可能经队列retain/清preparing取消第一次的id0系统重编；维护者已开始修幂等停用，尚待真实回归。

因此本报告原有“V8例包没订阅”的说明只描述观察端局限，不足解释全部调度路径，也不能证明系统重编真的开始或失败。V8原始实证应严格限定为：本轮staged源码确已腐坏、死侧栏即时隐藏、旧底栏仍能操作。正常停用、rowId、菜单驻留等正向结果保持；不删除历史，也不倒推V8这一次必然由与V10相同的根因造成。
