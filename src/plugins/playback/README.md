# Basic Playback

The main window's control bar, and playback commands such as open, seek and speed.

## Features

- **Control bar**: seek bar, play/pause, stop, back, forward, open, mute and volume, playlist toggle; click the current time to switch between elapsed and remaining time. Button tooltips show the current shortcut.
- **Volume and mute are remembered**: the last volume and mute (set with the bar, the keys or anything else) come back when the player starts again.
- **Open**: open a file, open a link (can fill in a link from the clipboard automatically), reopen the current or last played file; copy the file name or path.
- **Seek**: four seek steps (change them in Seek Steps…), previous/next keyframe, jump to the start, jump to a given time.
- **Speed**: 0.5×, 1.5×, 2× and a custom speed.
- **Seek bar hover**: shows the time or a thumbnail at the pointer (thumbnails need ffmpeg).
- **Seek bar thumbnails (0.2.0)**: on by default for local video files, shown on hover and while dragging the seek bar. A few seconds after a file starts playing, a set of keyframe thumbnails (up to 200) is made in the background; until then the nearest finished thumbnail is shown. Opening a file is never slowed down by this: nothing runs before the first frame is on screen, at most two ffmpeg processes run at a time, and they pause while you drag or seek. Thumbnails are kept for next time (up to 200 MB in the plugin's data folder, oldest first out). Turn them off with Show Thumbnail on Seek Bar Hover. Network streams show the time only for now.
- **No screen saver while playing**.

## Where to find it

- Right-click menu: Open File… (Ctrl+O, F3), Open ▸ Open URL… (Ctrl+U), Reopen Current/Last File (Ctrl+Y), Copy to Clipboard.
- Right-click menu Playback ▸ Seek: Previous/Next Keyframe (Ctrl+Shift+←/→), Back/Forward 30 s (Ctrl+←/→), 60 s (Shift+←/→), 300 s (Ctrl+Alt+←/→), Jump to Start (Backspace), Seek Steps…
- Right-click menu Playback: Jump to Time… (G), Speed, Disable Screen Saver While Playing, Show Time / Thumbnail on Seek Bar Hover.

Basic actions such as play/pause, back/forward 5 seconds, volume and mute are provided by the player and keep working when this plugin is turned off (without it, the volume is not remembered).
