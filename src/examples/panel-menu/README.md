# Example: panel context menu (examples.panel-menu)

A sidebar panel 示例列表 (Example List) that demonstrates:

- right-clicking an entry pops up this plugin's commands (`panel.showMenu`, one level, `null` is a separator), styled like the main menu;
- Home / Delete work while the panel has focus (`focus: "panel"`), and not when the focus is elsewhere;
- the two commands are not in the main menu (`path` omitted, `title` required) and appear only in the panel menu;
- it registers as a play sequence provider (`player.provideSequence`): another plugin's `player.next()` moves the selection in the list;
- the `title` of `mount` is the title of the floating window after detaching (detach with 分离侧栏 (Detach Sidebar) from the core-ui example).

It takes the only sidebar slot; do not load it together with the playlist.
