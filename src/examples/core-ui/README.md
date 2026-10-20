# 示例：核心界面接口（examples.core-ui）

演示 [core-ui-design.md](../../../doc/architecture/core-ui-design.md) 的新接口，不属于产品。右键菜单「示例」子菜单（自成一组，有分隔线）：

| 入口                                     | 演示                                                                                                                                         |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 示例 → 速度 → 0.5/1/2倍                  | `radio` 单选圆点、`order`、`group`；屏幕提示 `osd.show`                                                                                      |
| 示例 → 音量 +5（Ctrl+Alt+↑，按住重复）   | `bindings`、`repeat: true`                                                                                                                   |
| Ctrl+Alt+W                               | 不进主菜单的命令（省略 `path`）                                                                                                              |
| 示例 → 上次速度：N倍                     | 标题会变的菜单项：在子作用域里 `dispose()` 后重新添加（释放即从菜单删除）；速度存进 `plugin.dataDirectory`（先写临时文件再改名），重启后恢复 |
| 示例 → 下一个 / 上一个（Ctrl+Alt+N / P） | `player.next()`、`can.next`；需要播放列表等提供者                                                                                            |
| 示例 → 显示侧栏 / 分离侧栏（Ctrl+Alt+D） | `plugin.panels.sidebar`：任何插件读写侧栏、分离为浮动窗口与附着                                                                              |
| 示例 → 主题控件…                         | `@vestra/theme.slint` 令牌与 `@vestra/widgets.slint` 的 Button、Slider、ToolTipArea                                                          |

运行：`cargo run --bin vestra-host -- --plugin ../plugins/playlist --plugin ../examples/core-ui`（在 `src/host/` 下）。
