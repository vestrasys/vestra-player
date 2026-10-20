# 示例：面板右键菜单（examples.panel-menu）

侧栏面板「示例列表」，演示：

- 右键条目弹出本插件命令（`panel.showMenu`，单层，`null` 为分隔线），样式同主菜单；
- 面板获得焦点时 Home / Delete 生效（`focus: "panel"`），焦点在别处时不生效；
- 两条命令不进主菜单（省略 `path`，`title` 必填），只出现在面板菜单里；
- 登记为播放序列提供者（`player.provideSequence`）：其它插件的 `player.next()` 在列表里移动选中项；
- `mount` 的 `title` 是分离后浮动窗口的标题（分离用 core-ui 示例的「分离侧栏」）。

只占一个侧栏位置，不要与播放列表同时加载。
