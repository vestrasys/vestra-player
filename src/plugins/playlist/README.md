# Playlist

The playlist in the right sidebar.

## Features

- **Add**: add files, folders (including subfolders) and links; drag and drop files, folders or list files onto the video area or the list.
- **When opening a file**: open only that file, add similar files from the same folder (the default), or add all files in the folder.
- **Organize**: multiple selection, drag to reorder, move up and down, several ways to sort, search; remove the selected, the unselected, the invalid or all items.
- **Play**: repeat (off, all, one), shuffle; plays the next item when one ends, and Fast Advance to Next can shorten the switch.
- **Opens on the playing item**: each time the list is shown (F6, its tab, detached or attached, after a restart) it is scrolled so that the playing item is in the middle (at the top or bottom near the list's ends); while the list is shown, an item that starts playing out of view is scrolled to the middle, unless you scrolled in the last 2 seconds.
- **List files**: open and save M3U, M3U8, PLS; open DPL.
- Shows each item's duration and the total duration: items that have played use the duration from the player; for items not yet played, durations are read only for the rows on screen (needs ffprobe, at most 2 at a time), so adding many files does not read them one by one; the total has a "+" when some items are unknown. The list can be restored after a restart.

## Where to find it

- Right-click menu Playlist (F6) shows or hides the list; when the sidebar has tabs from other plugins, F6 first switches to the playlist tab, and pressing it again on the playlist tab hides the sidebar. The control bar also has a toggle on the right.
- Right-click menu Open ▸ Open Folder… (F2).
- Right-click menu Playback ▸ Playlist: add, repeat, shuffle, list files and the settings.
- The right-click menu inside the list and the ADD, DEL and SORT buttons; when the list has focus, Enter plays, Delete removes, Ctrl+A selects all, Ctrl+S searches, and so on.
- Options window Playback ▸ Playlist.

## Limits

- Fast Advance to Next is not gapless playback; there is still a short pause at the switch.
- The list gives the interface only the 200 rows around the view at a time and updates them as you scroll; when you drag the scroll bar far in one go, the rows at the new position appear a moment later.
- Links are resolved by the installed site plugins; this plugin does not resolve links.
- Files that other plugins take over by extension (such as emulator ROMs) can be added with Add Files… and play normally, including previous/next; dropping onto the list, scanning folders and "similar files" only recognize audio and video extensions and do not include these files yet.
