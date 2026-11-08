# Playlist

The playlist in the right sidebar.

## Features

- **Add**: add files, folders (including subfolders) and links; drag and drop files, folders or list files onto the video area or the list.
- **When opening a file**: open only that file, add similar files from the same folder (the default), or add all files in the folder.
- **Organize**: multiple selection, drag to reorder, move up and down, several ways to sort, search; remove the selected, the unselected, the invalid or all items.
- **Play**: repeat (off, all, one), shuffle; plays the next item when one ends, and Fast Advance to Next can shorten the switch.
- **Opens on the playing item**: each time the list is shown (F6, its tab, detached or attached, after a restart) it is scrolled so that the playing item is in the middle (at the top or bottom near the list's ends); while the list is shown, an item that starts playing out of view is scrolled to the middle, unless you scrolled in the last 2 seconds.
- **List files**: open and save M3U, M3U8, PLS; open DPL.
- **Thumbnails** (on by default): each row shows a small picture with the duration on it, the title and the file's folder. Videos show a keyframe from a tenth of the way in, audio files their cover art, links a placeholder. The button at the right of the bottom bar switches between these rows and compact single-line rows; the choice is remembered. Pictures are made only for the rows on screen and a few around them (needs ffmpeg, at most 2 at a time, paused while a file opens) and kept in the plugin's data folder, so they come back at once.
- Shows each item's duration and the total duration: items that have played use the duration from the player; for items not yet played, durations are read only for the rows on screen (needs ffprobe, at most 2 at a time), so adding many files does not read them one by one; the total has a "+" when some items are unknown. The list can be restored after a restart.

## Where to find it

- F6 (Playlist in the command palette) shows or hides the list; when the sidebar has tabs from other plugins, F6 first switches to the playlist tab, and pressing it again on the playlist tab hides the sidebar. The player's Sidebar item in the right-click menu and the Sidebar button at the right of the control bar show or hide the sidebar at its last tab.
- Right-click menu Open ▸ Open Folder… (F2).
- Command palette (Ctrl+Shift+P): Playlist: Add Files… / Add Link… / Add Folder…, Open List File…, Save List…, Shuffle, Fast Advance to Next, Remember List, Confirm Before Removing All; Repeat: Off / All / One; When Opening a File: the three modes (no longer in the right-click menu).
- The right-click menu inside the list and the ADD, DEL and SORT buttons; when the list has focus, Enter plays, Delete removes, Ctrl+A selects all, Ctrl+S searches, and so on.
- Options window Playback ▸ Playlist.

## Limits

- Fast Advance to Next is not gapless playback; there is still a short pause at the switch.
- Thumbnails are sent to the interface only for the rows on screen and 20 above and below; when you drag the scroll bar far in one go, the pictures at the new position appear a moment later.
- Touch screens are not a target: a tap on a row of a list long enough to scroll may scroll instead of selecting it.
- Links are resolved by the installed site plugins; this plugin does not resolve links. Link rows have no thumbnail of their own (the playlist does not know a site's pictures).
- A thumbnail is made from a keyframe, so it may come from slightly before a tenth of the video; a file changed on disk gets a new one the next time its row is shown.
- Files that other plugins take over by extension (such as emulator ROMs) can be added with Add Files… and play normally, including previous/next; dropping onto the list, scanning folders and "similar files" only recognize audio and video extensions and do not include these files yet.
