# 首轮菜单原生操作：出现崩溃，未通过

操作者root，CUA绑定`local.vestra.rebuild.shared-ui`，本轮实际宿主及host-ui-demo。截图原始输出在本轮CUA会话，未另存原始CUA PNG。

1. 初始真实窗口显示session1、14.03秒/frame425。
2. 点击Commands，出现同树扁平菜单候选；AX包含文件/播放/视频路径，禁用项及Example reason。截图为17.90秒/frame540。
3. 明确等待11秒后再次截图，菜单仍开，视频图案变化，时钟35.72秒/frame1073。此为有限驻留正向观察，没有测量最大draw间隔或设备声音，不代表完整菜单通过。
4. 点击AX文件/Open native file dialog后CUA超时，再取AX亦超时。随后读取本轮native.stderr发现Slint interpreter `accessing deleted parent (issue #6426)` main panic。没有见到原生文件面板，没有执行选中/取消；不能把点击尝试当文件对话框通过。

失败日志已原样复制到`popup-click-failure/`。maintainer正在核验弹窗先close再访问root回调的生命周期并修复；修后必须另轮实测。旧PID消失后还发现另一无参数bundle进程，来源未独立确认，已交maintainer回收，不继续对旧CUA绑定操作。

另一个可见问题：AX包含中文领域名，但截图中文字缺失，只显示斜杠及英文；日志同时包含ICU4X中文/日文分词模型缺失信息。两者相关原因尚待定位，不能从该提示直接断言字体根因。中文领域呈现待修复及截图复核。
