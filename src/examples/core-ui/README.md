# Example: core UI APIs (examples.core-ui)

Demonstrates the new APIs from [core-ui-design.md](../../../doc/architecture/core-ui-design.md); not part of the product. The 示例 (Examples) submenu in the right-click menu (a group of its own, with a separator). The menu titles below are the example's actual (Chinese) strings:

| Entry                                     | Demonstrates                                                                                                                                         |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 示例 → 速度 → 0.5/1/2倍 (Examples → Speed → 0.5/1/2×)                  | `radio` radio dots, `order`, `group`; on-screen message `osd.show`                                                                                      |
| 示例 → 音量 +5 (Examples → Volume +5) (Ctrl+Alt+↑, repeats while held)   | `bindings`, `repeat: true`                                                                                                                   |
| Ctrl+Alt+W                               | A command that is not in the main menu (`path` omitted)                                                                                                              |
| 示例 → 上次速度：N倍 (Examples → Last speed: N×)                     | A menu item whose title changes: `dispose()` it in a child scope and add it again (disposing removes it from the menu); the speed is stored in `plugin.dataDirectory` (written to a temporary file first, then renamed) and restored after a restart |
| 示例 → 下一个 / 上一个 (Examples → Next / Previous) (Ctrl+Alt+N / P) | `player.next()`, `can.next`; needs a provider such as the playlist                                                                                            |
| 示例 → 显示侧栏 / 分离侧栏 (Examples → Show Sidebar / Detach Sidebar) (Ctrl+Alt+D) | `plugin.panels.sidebar`: any plugin reads and writes the sidebar, detaches it into a floating window and attaches it again                                                                              |
| 示例 → 主题控件… (Examples → Theme Widgets…)                         | `@vestra/theme.slint` tokens and Button, Slider, ToolTipArea from `@vestra/widgets.slint`                                                          |

Run: `cargo run --bin vestra-host -- --plugin ../plugins/playlist --plugin ../examples/core-ui` (in `src/host/`).
