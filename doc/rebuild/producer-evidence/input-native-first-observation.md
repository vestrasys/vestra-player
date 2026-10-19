# 原生输入首轮：窗口不可绑定，未执行键鼠验收

2026-10-18。唯一实验目录 `/tmp/vestra-producer-input-native-V7OZ1P`，bundle ID `local.vestra.rebuild.producer-input-d9a9d6c51e06`；维护者报告原宿主 PID15094、Node15124。冻结构建依据见 `input-build/`，本轮未修改共享源码或重新构建。

root 在维护者交还唯一 CUA 时段后实际观察：

- CUA inventory 显示该唯一 bundle 正在运行；`ready.json` 有真实 producer/source 与九个物理键声明。
- `cua.getApp(bundleId)` 和精确现存 `.app` 路径均返回 `cgWindowNotFound`；按显示名称返回 `Invalid app`。没有获取窗口截图，没有发送任何键鼠事件，没有绕过 CUA 使用 OS 输入注入。
- CUA 文档仅暴露按键/组合键，没有独立 keyDown/keyUp；即使窗口恢复，持续按住游戏键同时鼠标开菜单也不能由现有接口直接证明。
- 维护者只读进程栈看到主线程处于 Slint/winit 事件循环；这不证明窗口可见，也未据此定位为 Slint、应用或工具某一方的缺陷。

root 写入本轮自有 `stop` 文件正常停止探针。实际 `result.json`：`outcome=stopped`，`appliedReceipts=0`，`replayedReceipts=0`，`controls=[]`；source close 操作 `completed`。最终核心1654帧（包含540预热帧），音频实际消费888960帧，软件欠载4787072帧，设备错误1，详情为 `epoch=1 paused=false Xrun`。这些数据不支持音频连续性或原生 GUI 通过；不能将正常收尾当成输入验收成功。

下一步应先利用现有公开 window 状态和启动事实区分窗口可见性与工具绑定问题，并检查探针 RPC/日志开销与供帧节奏，再有限重跑。快速按放、双Shift、菜单/失焦撤销、公开暂停恢复和故意未确认输入超时均仍未实测；整个 M4 与共同基线未完成。
