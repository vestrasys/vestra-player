# Changelog

## 0.1.6

- Code only: the Chapters and Bookmarks window handle is named `marksWindow` / `#marksWindow` (was `window` / `#window`). Who must change: nobody. No behaviour change.

## 0.1.5

- The Chapters and Bookmarks window shows a file's chapters as soon as the player has read them, also while it plays. The list comes back from `player.chapters()` after the state event that asked for it; the plugin now calls `plugin.update()` when it arrives. With SDK 1.5.45's precise invalidation nothing else refreshed the window's rows at that moment: they usually showed about 0.15 s later, when the 150 ms menu rebuild timer ran an update pass, and at worst only at the SDK's safety pass (about 1 s). The rest of the plugin was checked for the same pattern: the A-B loop's commits (timer, after `await`) already call `plugin.update()`; the menu rebuild (timer) registers new commands with their state; the seek-bar marks and the Clear Bookmarks answer (an awaited window handler) need nothing.
- Tests: the window's chapter rows are there right after the source opens and `settle()`, with no wait for timers and the menu rebuild delay set to 5 s so its pass cannot stand in for the fix (fails without it).
- Nobody needs to change anything.
