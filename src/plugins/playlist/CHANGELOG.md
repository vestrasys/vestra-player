# Changelog

## 0.2.0

Who must change: nobody. What users see: nothing new; long lists scroll and update more smoothly. Needs player 0.6.0 (row patches, and the higher row limits of large lists P3 for long lists).

- The list is a `ListView` over every shown row (large lists P3) instead of a hand-made window of 200 rows in a scroll view: only the rows in view have elements, and the plugin no longer moves a window as the list scrolls. Each row carries a stable `id`, so on player 0.6.0 a change is sent as row operations (a move, an insertion, a removal, one changed row) instead of rewriting rows; row numbers come from the row's place in the list (and are sent with the rows only while a search filters), so moving a row changes no other row.
- Pressing, dragging (with edge scrolling), releasing and double-clicking are handled once for the whole list, which maps the pointer to a row by the row height and the scroll: a row scrolled out of view during a drag no longer takes the drag with it. One insertion marker is drawn for the list. Rows tell screen readers their place and whether they are selected.
- Each row holds its thumbnail (the placeholder away from the view), so a picture moves with its row; the separate list of pictures is gone.
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
