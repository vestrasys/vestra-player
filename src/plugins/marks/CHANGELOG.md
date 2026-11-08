# Changelog

## 0.1.7

- Menu sweep (menu redesign §4): Chapter and Bookmark List… (H) moves from Playback ▸ Chapters and Bookmarks ▸ to the top level, right after Playlist (order 801, windows group). Everything else of Chapters and Bookmarks ▸ (Add Bookmark, the previous/next steps, the chapter and bookmark lines, the "More…" lines, Show Marks on Seek Bar) and the whole A-B Loop ▸ with Loop Count ▸ leave the menu and are in the command palette only, with their ids, keys, radio and checked states and categories unchanged (the list window shows the chapters and bookmarks). Who must change: nobody.
- The description names Chapter and Bookmark List… (H) in the menu and the command palette for the rest, in all six languages.

- Every command has a command palette category (SDK 1.6.4 `category`), so the player's palette (Ctrl+Shift+P) names it "Category: Title": "Chapters and Bookmarks" for the list, Add Bookmark, the previous/next steps, each chapter and bookmark line, the "More…" lines and Show Marks on Seek Bar; "A-B Loop" for the A-B points, nudges, range moves, Loop Current Chapter or Bookmark Range and A-B Loop On/Off; "Loop Count" for the six counts. The categories are the submenu names, so every language reads them as the menu does.
- Add Bookmark and each chapter and bookmark line name Chapter and Bookmark List… as their `home`: the window lists them and has an Add Bookmark button.
- Every command that can be disabled already gives a reason, so the palette keeps listing them greyed.
- Needs player 0.7.6 (`minPlayerVersion`; older players refuse the new declaration fields).
- Tests: every command, chapter and bookmark lines included, has a non-empty category; the homes; the categories in Simplified Chinese.
- Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.

## 0.1.6

- Code only: the Chapters and Bookmarks window handle is named `marksWindow` / `#marksWindow` (was `window` / `#window`). Who must change: nobody. No behaviour change.

## 0.1.5

- The Chapters and Bookmarks window shows a file's chapters as soon as the player has read them, also while it plays. The list comes back from `player.chapters()` after the state event that asked for it; the plugin now calls `plugin.update()` when it arrives. With SDK 1.5.45's precise invalidation nothing else refreshed the window's rows at that moment: they usually showed about 0.15 s later, when the 150 ms menu rebuild timer ran an update pass, and at worst only at the SDK's safety pass (about 1 s). The rest of the plugin was checked for the same pattern: the A-B loop's commits (timer, after `await`) already call `plugin.update()`; the menu rebuild (timer) registers new commands with their state; the seek-bar marks and the Clear Bookmarks answer (an awaited window handler) need nothing.
- Tests: the window's chapter rows are there right after the source opens and `settle()`, with no wait for timers and the menu rebuild delay set to 5 s so its pass cannot stand in for the fix (fails without it).
- Nobody needs to change anything.
