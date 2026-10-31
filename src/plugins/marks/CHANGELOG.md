# Changelog

## 0.1.5

- The Chapters and Bookmarks window shows a file's chapters as soon as the player has read them, also while it plays. The list comes back from `player.chapters()` after the state event that asked for it; the plugin now calls `plugin.update()` when it arrives. With SDK 1.5.45's precise invalidation the window's rows otherwise waited for the next safety pass (up to about 1 s) instead of about 0.1 s before. The rest of the plugin was checked for the same pattern: the A-B loop's commits (timer, after `await`) already call `plugin.update()`; the menu rebuild (timer) registers new commands with their state; the seek-bar marks and the Clear Bookmarks answer (an awaited window handler) need nothing.
- Tests: the window's chapter rows are there after `settle()` with no wait for timers.
- Nobody needs to change anything.
