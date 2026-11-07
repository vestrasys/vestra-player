# Changelog

## 0.2.6

Who must change: nobody. What users see: in the command palette (Ctrl+Shift+P) playlist commands name what they are about, for example "Repeat: All" or "When Opening a File: Add Similar Files", and say where their control lives (the playlist, or Playlist Settings); lists of keys name the list commands the same way ("Sort: By Size"). Needs player 0.7.6.

- Every command declares a `category` (SDK 1.6.4): Playlist, Repeat, When Opening a File or Sort, the labels of the submenus and popups the commands sit in, with their existing translations.
- Commands whose control is in the playlist panel (the list commands, Add, Open and Save List) declare `home` "playlist.visibility"; the options that are also on the settings page (Repeat, Shuffle, When Opening a File, Fast Advance to Next, Remember List, Confirm Before Removing All) declare `home` "playlist.settings".

## 0.2.5

Who must change: nobody. What users see: the list's header has no close button (✕) any more; right below the window's own close button, a missed click closed the player. Hide the sidebar with F6, the bottom bar's sidebar button or Close List in the list menu. Needs player 0.6.0.

- The header keeps only the detach/attach button, at its right end; the `hide` callback and its "Close list" text are gone.

## 0.2.4

- Code only: the Add Link window handle is named `linkWindow` (was `window`). Who must change: nobody. No behaviour change.

## 0.2.3

Who must change: nobody. What users see: with long lists a click shows a little sooner (20,000 rows: about 40 to 21 ms in the plugin with SDK 1.5.47). Needs player 0.6.0.

- Each row the list is sent is frozen and stays the same object while it does not change, so SDK 1.5.47 reuses its encoding instead of encoding all rows on every update.
- The comparison of a row with the one sent last covers every field of the row; the type check fails if a new field is left out.

## 0.2.2

Who must change: nobody. What users see: turning thumbnails on or off now really keeps the row at the top of the list (0.2.1 could still jump far away). Needs player 0.6.0.

- The list keeps its top row itself when the row height changes; the sidebar no longer sets the scroll position then (that ran before the list's layout, which still used the old height).
- The scroll the list reports after the toggle is no longer taken for the user's (it may differ from the expected one by less than a row), so following the playing row does not pause for 2 seconds.

## 0.2.1

Who must change: nobody. What users see: turning thumbnails on or off keeps the row at the top of the list where it was (before, the list could jump far away); long lists react faster to clicks. Needs player 0.6.0.

- The thumbnail toggle no longer asks the list to scroll: the list keeps its top row itself when the row height changes. The folder and media kind are sent in both row styles, so the toggle changes only the pictures near the view.
- An unchanged row is the same object from one update to the next, the search numbering is rebuilt only when the order changes, and the selection-dependent command states (move, remove unselected, show in folder) are computed once per change instead of on every update.
- The list tells screen readers how many rows it has; a drag keeps its drop position right when rows are added or removed meanwhile.

## 0.2.0

Who must change: nobody. What users see: nothing new; long lists scroll and update more smoothly. Needs player 0.6.0 (row patches, and the higher row limits of large lists P3 for long lists).

- The list is a `ListView` over every shown row (large lists P3) instead of a hand-made window of 200 rows in a scroll view: only the rows in view have elements, and the plugin no longer moves a window as the list scrolls. Each row carries a stable `id`, so on player 0.6.0 a change is sent as row operations (a move, an insertion, a removal, one changed row) instead of rewriting rows; row numbers come from the row's place in the list (and are sent with the rows only while a search filters), so moving a row changes no other row.
- Pressing, dragging (with edge scrolling), releasing and double-clicking are handled once for the whole list, which maps the pointer to a row by the row height and the scroll: a row scrolled out of view during a drag no longer takes the drag with it. One insertion marker is drawn for the list. Rows tell screen readers their place and whether they are selected.
- Each row holds its thumbnail (none away from the view: an empty image, player 0.6.0), so a picture moves with its row; the separate list of pictures and the placeholder picture file are gone.
- The list is saved only when something saved changed, not checked every 200 ms while the list updates.

## 0.1.12

- The drag insertion marker, the playing row's bar, the failed mark, the thumbnail placeholder and duration badge, "No matches", the folded search icon and the search box's hint are now always present and shown or hidden, instead of being created and removed as the pointer moves, the list scrolls or a row changes. Screen readers no longer see the list's accessibility tree rebuilt during a drag. Nothing looks different.

## 0.1.11

- Each row shows a thumbnail by default, like the VestraTube list: a 96×54 picture with the duration on its bottom right, the title on up to two lines and the file's folder (a link's host) below it. Video files show a keyframe from a tenth of the way in, audio files their embedded cover art; links, audio without cover art and files the player cannot read show a placeholder.
- The new button at the right of the bottom bar turns thumbnails off (the compact single-line rows of before) and on; its icon shows the current mode, its tooltip what a click does. The choice is remembered. Add and Delete are narrower to make room.
- Thumbnails are made like the durations: only for the rows on screen and three above and below, at most two ffmpeg processes with one thread each, and never while a file opens or in its first 3 seconds. Rows scrolled away stop their work. Pictures are kept in the plugin's data folder (`thumbs/`, at most 5000, least recently used removed first), so a row shown again, also after a restart, needs no ffmpeg; a file ffmpeg cannot read is not tried again.
- Needs player 0.5.1 (pictures in a list).

## 0.1.10

- The search box's selected text uses the standard selection colours (as the std-widgets LineEdit), legible in both palettes.

## 0.1.9

Earlier versions have no changelog; see the repository history.
