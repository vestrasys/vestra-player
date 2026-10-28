# Changelog

## 1.5.4

- "Load audio files with the same name automatically" now leaves a file to the video whose name it matches more closely, as the Subtitles plugin does for same-name subtitle files (0.4.5). With `Movie.mkv` and `Movie.Extended.mkv` in one folder, `Movie.Extended.ac3` and `Movie.Extended.jpn.mka` load only with `Movie.Extended.mkv`; `Movie.ac3` and `Movie.jpn.mka` still load with `Movie.mkv`. Before, `Movie.mkv` also picked up the other video's audio, since its name starts with `Movie.`. Only files with a video extension in the same folder count as other videos (`.mkv`, `.mp4`, `.avi`, `.mov`, `.webm`, `.ts`, `.m2ts` and the like); names are compared case-insensitively, like the match itself.
- File names are compared in Unicode NFC as well as case-insensitively, as in the Subtitles plugin: macOS lists names in decomposed form (`Cafe` + combining accent), so `Café.ac3` was not found for an opened `Café.mkv` whose path came in composed form, or the other way round. Now it is.
- A file whose name is the video's name followed by two dots and the extension (`movie..ac3` for `movie.mkv`, an empty tag) is no longer loaded, as in the Subtitles plugin (`movie..srt`). `movie.ac3` and tagged names such as `movie.jpn.mka` load as before.
- Everything else about the scan is unchanged: off by default (mpv's `audio-file-auto` is `no` by default; when on, the match is mpv's `exact` plus any dotted tag), only local files, the folder of the video only, audio files sorted by name, at most 8, added without being selected.
