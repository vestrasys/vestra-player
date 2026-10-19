# 第二原生窗口诊断：仍未完成原生input验收

同一冻结二进制，新bundle local.vestra.rebuild.producer-input-a993a78dbe57，原PID16031/Node16069，默认有界300s；runtime正常准备、publicwindow-ready与window-events记录主壳1100×720、canvas1084×642、nativeActive=false/nativeUnavailable=null。此事实不能代替可见屏幕或CUA绑定。root仍按精确ID/path得到cgWindowNotFound；没有发出任何键鼠输入。普通Launcher显前未使用，未用CG/osascript或任何OS输入旁路。

root明确stop，result stopped、receipts0、controls空、source.close completed。最后core9445（含540暖机）、消费7108609、软件欠载208895、设备errors0。单poll批次减少前版开销，但4270次poll中最大间隔591.048ms、state RPC最大538.302ms，不能据均值约58fps宣称无欠载/低延迟/连续AV。若短down/up在同CPU帧前全部应用，Notice通过也不证明游戏读取到按下；TS采样/latch仍需后续真实场景修正。

原实例按315s上限自然退出；未将脚本退出或public逻辑尺寸当原生pass。原始执行脚本/实际consumer、所有日志/结果/窗口事件、依赖/ROM、实际SDK/runtime/ui快照保留；binary只留SHA绑定。

root独立观察：[首轮](../input-native-first-observation.md)、[本轮](../input-native-diagnostic-observation.md)。原第一失败另在../input-native-window-failure/，未覆盖。
