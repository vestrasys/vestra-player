# V7 主窗菜单原生崩溃记录

2026-10-17，独立验收者在实际 Slint 主窗操作：侧栏选第二项、底栏暂停、Commands → 播放 → 暂停/播放。最后一步后原宿主 PID 84548 退出。macOS 原始报告确认该实例（bundle ID `local.vestra.rebuild.shared-ui-v7`）发生 EXC_BAD_ACCESS / SIGSEGV，主线程位于 Slint interpreter 的 invoke_callback。不是正常关闭，也不能用随后出现的无参数空白实例代替原实例继续验收。

崩溃前仅确认侧栏 rowId 更新、底栏暂停和层级入口可达。其余挂载、显示隐藏、失败恢复与清理用例未完成，本轮整体不通过。

维护者修复方向：菜单回调只投递携带 shell generation 的内部事件，回调栈返回后的 UI tick 再修改菜单模型、关闭弹窗和路由命令。需要新实例独立复验；此目录保留原始失败，不因后续修复覆盖。
