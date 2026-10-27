# Changelog

## 0.1.11

- Each row shows a thumbnail by default, like the VestraTube list: a 96×54 picture with the duration on its bottom right, the title on up to two lines and the file's folder (a link's host) below it. Video files show a keyframe from a tenth of the way in, audio files their embedded cover art; links, audio without cover art and files the player cannot read show a placeholder.
- The new button at the right of the bottom bar turns thumbnails off (the compact single-line rows of before) and on; its icon shows the current mode, its tooltip what a click does. The choice is remembered. Add and Delete are narrower to make room.
- Thumbnails are made like the durations: only for the rows on screen and three above and below, at most two ffmpeg processes with one thread each, and never while a file opens or in its first 3 seconds. Rows scrolled away stop their work. Pictures are kept in the plugin's data folder (`thumbs/`, at most 5000, least recently used removed first), so a row shown again, also after a restart, needs no ffmpeg; a file ffmpeg cannot read is not tried again.
- Needs player 0.5.1 (pictures in a list).

## 0.1.10

- The search box's selected text uses the standard selection colours (as the std-widgets LineEdit), legible in both palettes.

## 0.1.9

Earlier versions have no changelog; see the repository history.
