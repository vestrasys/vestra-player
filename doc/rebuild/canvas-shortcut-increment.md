# development-2026-10-18.1 的画布快捷键增量

2026-10-18。由基础播放真实 usage 驱动，wire 仍为 `vestra.experimental/0.1`；不重开共同工作基线，也不增加 `input.bind` 平台或插件身份判断。

`commands.register` 已有的 `declaration.shortcut` 新增有限执行语义：`Space`、`ArrowLeft`、`ArrowRight` 在主画布聚焦时，将无修饰的首次按下路由到声明它的同一命令。长按重复事件忽略；其它字符串仍只展示。F11/Escape 保留宿主中央处理。播放作者已把早期 `Left`/`Right` usage 改为 `ArrowLeft`/`ArrowRight`，命令分别为 `toggle-pause`、`backward-5`、`forward-5`。

匹配前核对当前 shell generation、画布实际焦点、菜单/对话框及来源输入捕获；文本编辑器、工具窗口和已捕获来源不触发普通画布命令。解析后仍通过当前命令 enabled/busy、owner generation 和同一个命令分发路径。两个命令声明相同受支持快捷键会在注册时明确拒绝（现有 `command` 错误码及 conflict 诊断）。移除 owner 后释放占用。有限试载实例的全局 shortcut 不接管正式实例，隔离菜单入口仍可使用。

已执行：一项受影响 registry 测试通过（当前状态、冲突和 owner 清理）；Clippy 与 feature build 通过，日志 `/tmp/vestra-canvas-shortcut-tests.log`、`/tmp/vestra-canvas-shortcut-clippy.log`、`/tmp/vestra-product-combined-build.log`。这不等于原生按键/编辑器/捕获组合已经交互验收；后续以实际产品 UI 路径记录。播放器的 pause/seek 业务继续由 TS 插件实现。
