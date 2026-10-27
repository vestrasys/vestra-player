# Changelog

## 1.0.4

- Save Current Frame, Save Rendered Frame, Copy Current Frame, Copy Rendered Frame and a "current frame" Burst capture the frame shown when the key (or Start) was pressed, like mpv's `screenshot` and PotPlayer. Before, the player froze its current frame only after the snapshot folder was checked and the state read again, up to about 100 ms (several frames) later while playing. The position read at the press is now sent as the capture time, as Save As already did; this costs nothing, since every snapshot is decoded by the export worker anyway. Paused, the frame is the same as before.
- The saved file is named by the time the player reports for the captured frame, which is also the frame the export writes, like mpv's `screenshot-template` `%P`. Before, Save Current / Rendered Frame named the file by the position read at the press while the picture was the later frame (`movie_00.04.400.png` holding the frame at 00:00:04.467). Bursts name each file by its own sample time, as before; Copy has no name; Export Clip uses the times in its dialog.
