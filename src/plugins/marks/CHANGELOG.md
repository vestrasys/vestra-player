# Changelog

## 0.1.8

- A-B Loop moved to the Playback panel (user ruling 2026-11-08): the A-B loop, its 18 commands (`ab.set-a` … `ab.toggle`, the six `ab.count.*`) and the Loop Count setting are now the Basic Playback plugin's (`vestra.playback` 0.2.18), which shows them in its Playback panel (Alt+4) and keeps the same local ids, titles, keys, categories and checked states. This plugin keeps chapters, bookmarks, the Chapter and Bookmark List… window (H), their commands and their seek-bar marks; the `A` marker a lone start point puts on the seek bar now comes from the playback plugin. Who must change: nobody, with the player release that carries the binding migration (keys a user rebound for `vestra.marks/ab.*` follow to `vestra.playback/ab.*`). The Loop Count saved in `marks.json` (`abCount`) is left in that file untouched; the playback plugin starts at Endless.
- The plugin is now called **Chapters and Bookmarks** (was "Chapters, Bookmarks and A-B Loop"); the punchline, description and tags follow, in all six languages (the name reuses the existing "Chapters and Bookmarks" translations; the new punchline is machine-translated, `x-machine`, in de, es, fr, ja and ko). The A-B Loop texts are removed from the catalogs.
- Code: `loop.ts`, `addLoopCommands`, `segmentAt`, `Actions.times` and the loop count in `store.ts` are gone; `seekbarMarks` no longer takes A and B.
- Tests: the B23 tests moved to the playback plugin; this plugin's tests check that it declares no `ab.*` command and publishes no `A` marker.
- Menu (user ruling 2026-11-11): Chapter and Bookmark List… (H) moves from the top level of the right-click menu into Tools ▸ (`["@tools", "Chapter and Bookmark List…"]`, SDK 1.7.12), after Snapshot ▸; order 801 stays, its group is now `marks` (was `windows`) so it sits between separators of its own. The description says "Tools ▸ Chapter and Bookmark List…" in all six languages (the player's word for Tools). Who must change: nobody.

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
