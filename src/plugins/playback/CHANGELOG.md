# Changelog

## 0.2.2

- Seek bar thumbnails are exact for short files: when decoding the whole file is estimated to take no more than about 15 s (width × height × frame rate × duration at 4 ns per pixel), the thumbnails are the frames on screen every half second (at most 200, never finer than a frame), instead of the keyframe at or before each slot; a span without frames (a slideshow, a still stretch of a screen recording) repeats the frame still on screen. Measured on 1080p30 H.264 that is any file up to 60 s, on 1080p60 up to 30 s; 4K files and longer files keep the keyframe thumbnails of 0.2.0. The pass decodes in software only (ffmpeg's hardware decoding would share the video engine with the playing video; measured on 4K60 HEVC it stalled playback to 18–31 fps), in parts that start at the file's keyframes (ffprobe lists them), one thread each, two at a time, with the first thumbnails after about two seconds. A pass that fails, or a part that takes more than twice its estimate, falls back to keyframe thumbnails, and the file's cache entry remembers it so later opens do not try again. Exact sprites are cached under their own key; keyframe sprites made by 0.2.0 and 0.2.1 are kept.

## 0.2.1

- Jump to keyframes only (on by default, set in Seek Steps…): the four seek steps land on a keyframe near the target. Needs Vestra Player 0.5.1 or later.

## 0.2.0

- Seek bar thumbnails: a keyframe sprite of each local video file, made in the background after the first frame and shown on hover and while dragging; on by default; cached across sessions.
