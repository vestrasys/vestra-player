# 双槽组合有限原生操作记录

本轮实验，操作者root；使用CUA操作maintainer构建的`local.vestra.rebuild.composition-evidence`，窗口标题`Vestra composition candidate`。源码为本轮composition模块及独立probe，媒体为本轮合成视频。此例尚非产品插件或M1完整界面能力。

| 操作/观察 | 实际结果 |
| --- | --- |
| 查看真实窗口与截图 | 视频在左、独立侧栏在右、底栏跨两者全宽；session1，11.87秒/frame357 |
| 侧栏Select example row | 显示Selected: example-row；15.33秒/frame460 |
| 底栏Show/hide sidebar隐藏 | 侧栏控件从AX消失；18.63秒/frame560，仍session1 |
| 恢复侧栏 | 仍显示Selected: example-row；21.37秒/frame642 |
| 点击Pause | 按钮变Play，23.87秒/frame720；随后两次观察保持该时钟/帧，截图画面存在 |
| 点击Play | 按钮变Pause，24.25秒/frame730；随后31.65秒/frame952 |
| 关闭窗口 | CUA关闭调用返回超时；随后独立进程检查已无composition实例，不把超时当成功回包 |

截图和AX原始输出在本轮CUA工具会话；未另保存原始CUA PNG到仓库。native进程日志由maintainer保存在本目录。截图说明看到实际视频，不用诊断时钟单独代替像素观察。

此记录支持有限公开Slint组合、owner回调和显隐/暂停/恢复候选。没有测量最大屏幕刷新间隔，不能证明零卡顿；没有验证菜单驻留、原生文件对话框、设备声音、工具窗口、struct/model、首次未知组件失败保旧或全部插件共存。独立产品验收仍待后续完成。
