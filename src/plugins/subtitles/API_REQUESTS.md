# 0.1.2 公共 API 使用

本轮不需要共享接口新增。

- commands.register/update/onExecute：完整中文字幕菜单leaf和busy状态。
- events.subscribe(source.state/playback.backend)：来源变化及刷新。
- ui.open/set/show/onCallback：独立普通Slint Window；callbacks仅实际声明的12项。
- dialogs.open(openFile)：用户选择SRT；可信进程用Node fs有界读取最多8MiB，并验证UTF-8。
- subtitles.state/select/remove：当前轨道和本次运行资源操作。
- subtitles.begin/append/commit/discard：32KiB分块，SHA-256，generation与expected selection stamp；commit提交后不再自动discard。
- subtitles.style：带operationId、revision、expected stamp的样式/offsetMs，null撤销本owner层。
- subtitles.operation：RPC timeout查询同operationId，不重放写操作。

Manifest capabilities为subtitles.read、subtitles.control、commands、events、ui.window、ui.dialog；不使用playback/source控制。Host和字幕接口均type-only import，生产JS仅依赖Node fs/crypto。

当前边界：普通native Slint编译/激活和真实媒体效果仍由维护者统一验收；本地6项编排测试不充当这些验收。本轮0.1.1是从当前作者上下文的新revision，不是旧dataless包同字节恢复。

0.1.2仅修正三处ComboBox.selected参数：该信号参数为string，业务回调需要int，因此改传self.current-index。不请求共享API或Compiler放宽类型。0.1.1真实Compiler失败证据保留在README。
