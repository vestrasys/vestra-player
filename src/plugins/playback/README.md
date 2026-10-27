# Basic Playback

The main window's control bar, and playback commands such as open, seek and speed.

## Features

- **Control bar**: seek bar, play/pause, stop, back, forward, open, mute and volume, playlist toggle; click the current time to switch between elapsed and remaining time. Button tooltips show the current shortcut.
- **Volume and mute are remembered**: the last volume and mute (set with the bar, the keys or anything else) come back when the player starts again.
- **Open**: open a file, open a link (can fill in a link from the clipboard automatically), reopen the current or last played file; copy the file name or path.
- **Seek**: four seek steps (change them in Seek Steps…), previous/next keyframe, jump to the start, jump to a given time.
- **Jump to keyframes only (0.2.1)**: on by default, set in Seek Steps…. The four seek steps (←/→ and the Ctrl, Shift and Ctrl+Alt steps) then land on a keyframe near the target, forward on or after it and back on or before it, so a press never freezes the picture on files with few keyframes, and each press always moves. The on-screen message shows where the seek landed. Turn it off for exact seeks, as before. The seek bar, Jump to Time, chapters, bookmarks and the A-B loop always seek exactly. Needs Vestra Player 0.5.1 or later.
- **Speed**: 0.5×, 1.5×, 2× and a custom speed.
- **Seek bar hover**: shows the time or a thumbnail at the pointer (thumbnails need ffmpeg).
- **Seek bar thumbnails (0.2.0)**: on by default for local video files, shown on hover and while dragging the seek bar. A few seconds after a file starts playing, a set of thumbnails (up to 200) is made in the background; until then the nearest finished thumbnail is shown. Opening a file is never slowed down by this: nothing runs before the first frame is on screen, at most two ffmpeg processes run at a time, and they pause while you drag or seek. Thumbnails are kept for next time (up to 200 MB in the plugin's data folder, oldest first out). Turn them off with Show Thumbnail on Seek Bar Hover. Network streams show the time only for now.
- **Exact thumbnails for short files (0.2.2)**: a file that is cheap enough to decode whole gets one thumbnail every half second (or 200 over its length, or one per frame for slideshow-like files, whichever is coarser), each the frame on screen at that time, so a phone clip with a keyframe every 5–10 s no longer shows the same picture over a whole stretch of the bar. "Cheap enough" is a budget, not a length: the estimated decoding time — width × height × frame rate × duration at 4 ns per pixel — must stay within about 15 s, which is what the 200 keyframe thumbnails of a long file already cost. That is a 1080p30 file up to 60 s, 1080p60 up to 30 s, 720p30 up to about 2 min; 4K files and longer files keep keyframe thumbnails as before. The pass decodes in software (the computer's video decoder is left to the playing video: sharing it was measured to stall 4K playback) in parts that begin at the file's keyframes, one thread each, two at a time, so nothing is decoded twice and the first thumbnails appear after about two seconds; the same drag and seek pauses apply. If a part takes more than twice as long as estimated, the computer is slower or busier than assumed, and the file gets keyframe thumbnails instead; that is remembered, so the next time the file opens it is not tried again.
- **No screen saver while playing**.

## Where to find it

- Right-click menu: Open File… (Ctrl+O, F3), Open ▸ Open URL… (Ctrl+U), Reopen Current/Last File (Ctrl+Y), Copy to Clipboard.
- Right-click menu Playback ▸ Seek: Previous/Next Keyframe (Ctrl+Shift+←/→), Back/Forward 30 s (Ctrl+←/→), 60 s (Shift+←/→), 300 s (Ctrl+Alt+←/→), Jump to Start (Backspace), Seek Steps…
- Right-click menu Playback: Jump to Time… (G), Speed, Disable Screen Saver While Playing, Show Time / Thumbnail on Seek Bar Hover.

Basic actions such as play/pause, back/forward 5 seconds, volume and mute are provided by the player and keep working when this plugin is turned off (without it, the volume is not remembered).
