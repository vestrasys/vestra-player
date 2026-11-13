# Feature requirements: one chapter per plugin

Date: 2026-10-20. Source: the items in the [feature catalog](feature-catalog.md) that are not implemented (✗) or partly implemented (◐); after the cut review, 78 items remain. Interface language (H14) is handled separately and is out of scope here.

One chapter per plugin. Each chapter gives, in order:
- scope and boundaries;
- feature overview;
- menus and shortcuts;
- requirements item by item (requirement, entry points, behavior details, interface, persistence, core dependencies, acceptance, notes);
- the chapter's core API requests.

The core API summary for the whole document is in [Appendix A](#appendix-a-core-api-request-summary); conventions shared by all chapters are in [Appendix B](#appendix-b-common-conventions).

**Hard constraints** (apply to the whole document): plugins do not communicate with each other; the core provides only host capabilities that plugins cannot provide themselves; cross-platform (macOS and Windows, see community workflow §1.5) — the macOS commands and behavior described here are the first implementation and will later be made platform-independent; control sizes follow the [UI spec](ui-spec.md).

## Plugin overview

| Plugin | Location | Type | Features this round |
| --- | --- | --- | --- |
| Playback control `playback` | `src/plugins/playback/` | Existing | 10 (plus "Basic playback actions handed over to the core") |
| Playlist `playlist` | `src/plugins/playlist/` | Existing | 11 |
| Subtitles `subtitles` | `src/plugins/subtitles/` | Existing | 11 |
| Picture adjustment `video` | `src/plugins/video/` | Existing | 13 |
| Audio workbench `audio` | `src/plugins/audio/` | Existing | 7 |
| Capture `capture` | `src/plugins/capture/` | Existing | 4 |
| Playback info `info` | `src/plugins/info/` | Existing | 3 |
| Window `window` | `src/plugins/window/` | New plugin, to be created | 5 |
| Playback history `history` | `src/plugins/history/` | New plugin, to be created | 2 |
| Chapters, bookmarks and ranges `marks` | `src/plugins/marks/` | New plugin, to be created | 3 |
| Shortcut settings `hotkeys` | `src/plugins/hotkeys/` | New plugin, to be created | 3 |
| Preferences `preferences` | `src/plugins/preferences/` | New plugin, to be created | 2 |
| Plugin management `plugins` | `src/plugins/plugins/` | Removed 2026-10-26: managed in the store (`src/plugins/store/`) | 1 |
| Album art and idle screen `visualizer` | `src/plugins/visualizer/` | New plugin, to be created | 2 |
| Online subtitles `subtitle-online` | `src/plugins/subtitle-online/` | New plugin, to be created | 1 |

`subtitle-online` is a plugin of its own and is not merged into `subtitles`, for three reasons: it depends on an external network service and an API key, so it needs its own network and credential settings; it can be disabled on its own; and it does not affect local subtitle features.

## ID → plugin

| ID | Feature | Plugin |
| --- | --- | --- |
| A02 | Open URL (generic URL) | `playback` |
| A03 | Open folder | `playlist` |
| A08 | Secondary subtitles (two subtitles shown at once) | `subtitles` |
| A09 | Reload subtitles (read the file again) | `subtitles` |
| A10 | Reopen current / last file | `playback` |
| A11 | Load external audio track | `audio` |
| A12 | Recent playback history | `history` |
| A13 | Add same-folder / similar files when opening a file | `playlist` |
| A15 | Drag files onto the window to open | `playlist` |
| B06 | Configurable seek step seconds | `playback` |
| B07 | Previous frame / next frame | `playback` |
| B08 | Previous keyframe / next keyframe | `playback` |
| B09 | Jump to start | `playback` |
| B11 | Previous / next subtitle line | `subtitles` |
| B21 | Chapter / bookmark menu and jumping | `marks` |
| B22 | Chapter / bookmark markers on the seek bar | `marks` |
| B23 | A-B range loop | `marks` |
| B25 | Remember playback position | `history` |
| B26 | Fast advance to the next item (formerly "gapless playback", downgraded by R-P5) | `playlist` |
| B29 | Prevent screensaver / sleep during playback | `playback` |
| B30 | Hide the mouse pointer during playback | `window` |
| C04 | Add folder | `playlist` |
| C07 | Remove unselected / remove invalid items | `playlist` |
| C11 | More sort orders | `playlist` |
| C17 | Restore the list after restart | `playlist` |
| C18 | Open / save M3U, M3U8, PLS lists | `playlist` |
| C21 | Search the list | `playlist` |
| C23 | Show in Folder | `playlist` |
| D02 | Cycle subtitle tracks | `subtitles` |
| D05 | Enter a sync value manually | `subtitles` |
| D06 | Font size shortcuts | `subtitles` |
| D07 | Subtitle position shortcuts | `subtitles` |
| D09 | Bold | `subtitles` |
| D11 | Shadow, background box | `subtitles` |
| D16 | Online subtitle search and download | `subtitle-online` |
| D17 | External ASS subtitles (keep effects) | `subtitles` |
| D18 | External subtitle formats: ASS, VTT, etc. | `subtitles` |
| E02 | Hue | `video` |
| E04 | Sharpen (including edge sharpening) | `video` |
| E05 | Denoise (3D / temporal / progressive) | `video` |
| E06 | Deblock / deband | `video` |
| E08 | Vertical flip / horizontal flip | `video` |
| E09 | Rotate 90 / 180 / 270 | `video` |
| E10 | Deinterlace (auto / on / off) | `video` |
| E11 | Aspect ratio and display mode | `video` |
| E12 | Stretch (add borders) / crop to a given ratio | `video` |
| E13 | Picture zoom and pan (frame position) | `video` |
| E16 | HDR correction / tone mapping | `video` |
| E17 | Pixel shaders (custom GLSL) | `video` |
| E18 | Frame doubling / motion compensation | `video` |
| F03 | Copy the current frame to the clipboard | `capture` |
| F04 | Single screenshot with effects and subtitles | `capture` |
| F07 | Screenshot format JPEG / BMP, quality | `capture` |
| F09 | Open the screenshot folder | `capture` |
| G03 | Select audio track | `audio` |
| G04 | Cycle audio tracks | `audio` |
| G05 | Channel selection (stereo / left / right / mono mix) | `audio` |
| G09 | Audio sync shortcuts | `audio` |
| G11 | Crystalizer shortcuts | `audio` |
| G16 | Audio output device selection | `audio` |
| H03 | Window at 0.5 / 1 / 1.5 / 2× video size | `window` |
| H04 | Fit to video aspect ratio | `window` |
| H05 | On top: always / while playing / while fullscreen | `window` |
| H07 | Picture-in-picture (replaces mini size) | `window` |
| H10 | Theme / color scheme switching | `preferences` |
| H12 | Show the logo when idle | `visualizer` |
| I03 | Brief info | `info` |
| I05 | Playback properties dialog | `info` |
| I06 | Show time on seek bar hover | `playback` |
| I07 | Show thumbnail on seek bar hover | `playback` |
| I08 | Copy file name / path | `playback` |
| I09 | About | `info` |
| J01 | Shortcut settings | `hotkeys` |
| J02 | Mouse action settings | `hotkeys` |
| J03 | Ignore default shortcuts | `hotkeys` |
| J04 | Main preferences window | `preferences` |
| J07 | Plugin management | `plugins` |
| J11 | Show album art for audio-only media | `visualizer` |
## Cut log

The 2026-10-20 cut review removed the following 47 items because few people use them and the mainstream players (VLC, IINA, mpv, MPC-HC) rarely offer them. The planned new plugins `files`, `scenes` and `timer` were dropped with them:

- **Open**: A04 remote server browsing, A05 preferred open method, A06 quick file search, A14 favorites
- **Playback**: B10 jump to middle / before the end, B13 jump by frame number, B14 keyframe seek mode, B15 scene grid, B18 speed-change pitch mode, B19 skip intro and outro, B20 skip chapter (covered by the chapter jumping in B21), B24 loop subtitle line, B27 action after playback ends, B28 timed quit
- **List**: C08 move to Trash, C13 invert selection, C19 multiple list tabs, C20 play queue, C22 rename, C24 copy and paste list items, C25 thumbnail view, C26 list file info
- **Subtitles**: D08 left/right alignment, D12 subtitle browser, D13 copy current subtitle, D14 save subtitles, D15 frame rate conversion, D19 subtitles shown on the black bars, D21 subtitle translation
- **Picture**: E07 levels, E14 bottom margin, E15 select video stream
- **Capture**: F06 thumbnail contact sheet, F08 file name template, F10 delete latest screenshot, F12 live recording
- **Audio**: G06 channel swap (merged into the options of G05), G07 equalizer expanded to 10 bands, G12 noise reduction, G13 reverb, G14 vocal removal
- **Window and info**: H02 fullscreen stretch, H06 move to a screen corner, H11 window opacity, I04 miscellaneous info
- **Settings**: J05 settings import / export, J10 check for updates (J10 restored by the user ruling of 2026-11-01, see [feature catalog](feature-catalog.md) J10)

Two further changes:
- J11 keeps only the album art; the audio animation is removed.
- H07 mini size is replaced by macOS picture-in-picture.

---

## Playback control (`playback`)

**Location**: `src/plugins/playback/` (existing plugin)

### Scope

playback owns the bottom bar (seek bar row 22px + button row 40px), the "Open / Close / Reopen" entry points, seeking and speed under "Playback ▸", and the on-screen messages for these operations. This round adds 10 items to it: Open URL, reopen the last file, four adjustable seek steps, frame and keyframe stepping, jump to start, prevent the screensaver during playback, time and thumbnail on seek bar hover, and copy file name / path.

Boundaries:
- Window size, on-top and pointer hiding belong to the new plugin `window`. The two plugins do not call each other; both only read the core's `player.state`.
- The playlist's "Add Link…" (playlist) adds a link to the list; this plugin's "Open URL…" replaces the current source directly and does not go into the list. Both use the same small-window spec UI-DLG-02, each with its own implementation.
- When a YouTube / Bilibili link is opened from "Open URL…", the core hands it, by host name, to the resolver registered by the matching plugin (`resolvers.add`); this plugin does not recognize sites.
- Recent playback history (A12) and remembering the playback position (B25) are not in this chapter. A10 only records the last source.
- Once "Basic playback actions handed over to the core" (the last section of this chapter) is done, play / pause, ±5 seconds, previous frame / next frame, previous / next, volume, mute, speed ± and stop are registered by the core, and playback only owns the bottom bar and the extended commands. The sections below are written for the state "after the hand-over", and also say what to do before it.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| A02 | Open URL (generic URL) | ◐ | No |
| A10 | Reopen current / last file | ◐ | No |
| B06 | Configurable seek step seconds | ✗ | After the hand-over (`player.setStepSeek`) |
| B07 | Previous frame / next frame | ✗ | Yes (core actions `frame-back` / `frame-forward`, `player.step`) |
| B08 | Previous keyframe / next keyframe | ✗ | Yes (`player.step("keyframe", …)`) |
| B09 | Jump to start | ✗ | No |
| B29 | Prevent screensaver / sleep during playback | ✗ | Yes (general capability `plugin.power`) |
| I06 | Show time on seek bar hover | ✗ | No |
| I07 | Show thumbnail on seek bar hover | ✗ | Yes (panel property carries an image) |
| I08 | Copy file name / path | ✗ | Yes (general capability `plugin.clipboard`) |
| — | Basic playback actions handed over to the core | — | Yes (core-registered actions + `plugin.host.run`) |

### Menus and shortcuts

Only new and changed items are listed; everything else stays as in `doc/product/menus.md`. `order` is the value at the level the item sits on; a submenu's position is the smallest `order` inside it. "Core" means the core registers it after the hand-over; the menu position does not change.

| Menu path | Registered by | order / group | Default shortcut | Notes |
| --- | --- | --- | --- | --- |
| Open ▸ Open URL… | playback | 110 / open | Ctrl+U | New; the `Open ▸` submenu as a whole stays between "Open File…" (100) and "Close" (120) |
| Reopen Current/Last File | playback | 121 / open | Ctrl+Y | The former "Reopen", renamed and available in more cases |
| Copy to Clipboard ▸ File Name · Folder Path · Full Path | playback | 125–127 / open | — | New |
| Playback ▸ Seek ▸ Previous Frame · Next Frame | Core (playback before the hand-over) | 204–205 / frame | D · F | New, repeats while held |
| Playback ▸ Seek ▸ Previous Keyframe · Next Keyframe | playback | 206–207 / frame | Ctrl+Shift+← · Ctrl+Shift+→ | New, repeats while held |
| Playback ▸ Seek ▸ Back/Forward N seconds (4 steps × 2) | Step 1 core, steps 2–4 playback | 210–217 / step | Unchanged | N in the title follows the B06 setting |
| Playback ▸ Seek ▸ Jump to Start | playback | 219 / points | Backspace | New |
| Playback ▸ Seek ▸ Seek Steps… | playback | 223 / jump-settings | — | New |
| Playback ▸ Disable Screen Saver While Playing (checkbox) | playback | 240 / playback-options | — | New, placed after "Playlist ▸" (230–235) |
| Playback ▸ Hide Pointer While Playing (checkbox) | window | 241 / playback-options | — | See B30 in the window chapter |
| Playback ▸ Show Time on Seek Bar Hover · Show Thumbnail on Seek Bar Hover (checkboxes) | playback | 242–243 / playback-options | — | New |

Conflict check (against menus.md and each plugin's `bindings`): D, F, Backspace, Ctrl+Shift+←/→, Ctrl+U and Ctrl+Y are not used by any command now. On macOS, `Ctrl+X` is triggered by both ⌃X and ⌘X:
- By default the system reserves ⌃←/⌃→ for Mission Control space switching (the existing "±30 seconds" is affected too); users can press ⌘⇧←/→.
- Ctrl+U and Ctrl+Y can also be triggered with ⌘U and ⌘Y and do not conflict with the system.

---

### A02 Open URL (generic URL)

Current state: there are only the dedicated YouTube / Bilibili windows and the playlist's "Add Link…"; there is no entry point that opens an arbitrary link directly and replaces the current source.

**Requirement**
- The user enters a network address (or an absolute local path), which immediately replaces the current source and starts playing.
- Supported addresses: `http`, `https`, `rtsp`, `rtmp`, `rtmps`, `mms`, `ftp`; `file://` and absolute paths starting with `/` are opened as local files. Any other address is reported as invalid.
- Option "Use an address from the clipboard by default", on by default: when the window opens and the first line of the clipboard is a supported address, it is filled into the input box and fully selected.

**Entry point**: menu "Open ▸ Open URL…", Ctrl+U.

**Behavior details**
1. When the window opens, the input box content is decided in this order: clipboard (option on and first line valid) → the text last entered during this run → empty. The clipboard is read with `plugin.clipboard.readText()`; no text or a failed read both count as no content, with no message.
2. On confirm, take the first non-empty line and trim leading and trailing whitespace.
   - With several lines, only the first line is opened, and the window shows "Only the first line is opened; for several links use the playlist's "Add Link…"" (a hint only; it does not block).
   - Invalid address: the window shows "Not a valid link" and stays open.
3. Call `player.open(url)`.
   - Until it returns, the "Open" button shows "Opening…" and is disabled; further Return presses or button clicks are ignored.
   - When `open` resolves, the window closes; when it rejects, the window shows "Open failed: <reason>".
   - If it was accepted but loading fails later (`phase: error`), the existing one-time on-screen message "Open failed: …" is used.
4. Links that match a resolver (youtube.com, bilibili.com, etc.) are handed to the resolver by the core; when the resolver is not installed, they go to mpv as an ordinary URL.
5. The window does not belong to a source: if another plugin changes the source while the window is open, the address the user entered is still opened, because this is an open the user started.
6. Opening a link does not add it to the playlist; the playlist's previous / next still follow its own cursor.
7. "Process subtitles (open subtitles at the same time)" is not done, because subtitles belong to the subtitles plugin and plugins cannot call each other.

**Interface** (the shared UI-DLG-02 "enter a link" spec; no history list)
- Tool window `ui/open-url.slint`, title "Open URL", width 464px, height follows the content (about 200px).
- First line of explanation "Enter the address of the file to play below."; second line, a grey example "e.g. https://example.com/video.mp4".
- Multi-line input box 397×68.
- Checkbox "Use an address from the clipboard by default".
- Message line, `text-dim`, wraps automatically.
- Bottom buttons [Open] [Cancel], button height 21px.
- Esc and "Cancel" close the window; Return in the input box equals "Open", Shift+Return inserts a line break.

**Persistence**: `openUrl.pasteFromClipboard` in `plugin.dataDirectory/settings.json` (boolean, default `true`). The input is not saved to disk.

**Core dependencies**: `player.open`, `windows.create`, `plugin.clipboard` (general capability, round 2).

**Acceptance**
- With `https://…/a.mp4` on the clipboard, press Ctrl+U: the input box is filled and fully selected; pressing Return starts playback and closes the window.
- Enter `abc`: the message "Not a valid link" appears and the window stays open.
- Enter a YouTube link: it is played through the youtube plugin's resolver.
- Turn the clipboard option off and open again: the input box shows the text entered last time.
- Unreachable address: the on-screen message "Open failed: …" appears only once.

**Notes**: following UI-DLG-02, there is no history list and no "save entered address", and no "open subtitles at the same time".

### A10 Reopen current / last file

Current state: "Reopen" (order 121) is available only after a failed load; it has no shortcut; nothing can be done when there is no current source.

**Requirement**
- When there is a current source, reload it.
- When there is no source (after startup, after close), open the source played last, across launches.

**Entry point**: top-level "Reopen Current/Last File", Ctrl+Y (the former "Reopen", renamed).

**Behavior details**
1. Record the "last source": listen to `player.onChange`.
   - When `kind === "media"`, `source` is non-empty and `phase` first becomes `ready`, write `last.json`.
   - Plugin simulator streams (`kind: "stream"`) are not recorded, because they cannot be reopened.
   - The source can have been opened by any plugin (playlist, YouTube, etc.).
2. When there is a current source (`phase` is `loading`, `ready`, `ended` or `error`):
   - First note `position` and `paused`, then call `player.open(source, { paused })`.
   - Once ready, if the old position is greater than 0 and less than the new duration, seek back to it. This lets a file that is still being written continue after it is reloaded.
   - On-screen message "Reopened: <file name or link>".
3. When there is no source: read `source` from `last.json` and open it from the start; on-screen message "Reopened the last file: <file name>". Resuming from the last position belongs to B25, not this section.
4. When unavailable: if `last.json` does not exist or is corrupt and there is no current source, the command is disabled with the reason "No file to reopen".
5. Local file deleted, link expired: try to open as usual; a failure goes through the existing "Open failed: …" message. Do not check in advance whether the file exists, because a network drive may be temporarily unreachable.
6. The reopened source is opened (`open`) by this plugin. The playlist sees it as "replaced by another plugin", and its cursor does not change (the same as opening from the YouTube window).
7. Pressing Ctrl+Y repeatedly: a press before the previous `open` has returned is ignored (the command is `busy`).

**Persistence**: `plugin.dataDirectory/last.json`: `{ "source": string, "openedAt": ISO time }`. Write a temporary file first, then rename it.

**Core dependencies**: `player.open`, `player.seek`, `media.ready()`, `player.onChange`. No new core API needed.

**Acceptance**
- Play a.mp4 to 1:00 and press Ctrl+Y: it reloads and returns to about 1:00.
- After closing, press Ctrl+Y: a.mp4 plays from the start.
- Restart the player and, without opening any file, press Ctrl+Y: a.mp4 plays.
- Delete a.mp4 and press Ctrl+Y: an open failure is reported.
- On the first run the menu item is grey, with the reason "No file to reopen".

### B06 Configurable seek step seconds

Current state: the four seek steps are fixed at 5 / 30 / 60 / 300 seconds.

**Requirement**
- The user sets the seconds of each of the four seek steps.
  - Step 1: ← / →, default 5 seconds.
  - Step 2: Ctrl + ← / →, default 30 seconds.
  - Step 3: Shift + ← / →, default 60 seconds.
  - Step 4: Ctrl+Alt + ← / →, default 300 seconds.
- Range 0.1–3600 seconds, at most one decimal place.

**Entry point**: "Playback ▸ Seek ▸ Seek Steps…".

**Behavior details**
1. Takes effect immediately after OK:
   - The menu titles become "Back N seconds / Forward N seconds", with trailing zeros removed from N (`2.5`, `10`). A title cannot be changed in place, so the eight commands live in a child scope, which is released and re-added with the new values; `order` 210–217 and the bindings stay the same.
   - The on-screen message text changes accordingly: "Back 10 s: 00:37 / 01:40".
2. Step 1 is handled in two ways:
   - Before the hand-over: playback's own `backward-5` / `forward-5` commands (renamed to `backward-1` / `forward-1`) run with the setting.
   - After the hand-over: ←/→ are core basic actions, and playback calls `player.setStepSeek({ seconds })` to hand the step to the core. When playback is disabled or uninstalled, the core goes back to 5 seconds.
3. Invalid input (empty, not a number, out of range, two or more decimal places): the right side of that row turns red, the message line shows "Range 0.1–3600 s, at most one decimal place", and "OK" neither closes the window nor saves.
4. Each row has a "Default" button that restores only that row.
5. The window does not belong to a source: it can be opened and saved even with no media.

**Interface**
- Tool window `ui/time-jump.slint`, title "Seek Steps", about 357×230.
- Explanation at the top: "Set how far each seek key moves during playback."
- Four rows, each: label → input box (width 60) → "s" → [Default].
  - Row 1 label "Left/Right arrow keys:"
  - Row 2 label "Ctrl + Left/Right arrow keys:"
  - Row 3 label "Shift + Left/Right arrow keys:"
  - Row 4 label "Ctrl + Alt + Left/Right arrow keys:"
  - These labels show the default keys; after the user rebinds, the labels do not follow, because the plugin cannot read the binding table.
- Bottom: [OK] [Cancel].

**Persistence**: `seekSteps: [number, number, number, number]` in `settings.json`, default `[5, 30, 60, 300]`. If the array length is wrong or a value is out of range, the whole set falls back to the defaults.

**Core dependencies**: `commands.add`, child scope rebuild, `windows.create`. [New core API] (needed only after the hand-over, see the last section):

```ts
player.setStepSeek(options: { seconds: number }): Promise<void>
```

**Acceptance**
- Change step 1 to 10: pressing → shows "Forward 10 s: …", and the menu shows "Forward 10 s".
- Enter 4000 for step 3: a range error is shown and the window stays open.
- The settings persist after a restart.
- Click "Default" on row 2: that row goes back to 30.

**Notes**: there is no frame step setting; frame stepping is always one frame. "Use keyframes as the seek unit" is not done (removed from the requirements).

### B07 Previous frame / next frame

**Requirement**: each press moves the picture one frame forward or back and keeps playback paused, for frame-by-frame viewing.

**Entry point**: "Playback ▸ Seek ▸ Previous Frame (D) / Next Frame (F)"; repeats with the system key repeat while held.

**Behavior details**
1. First press while playing: pause and show the next or previous frame; stay paused afterwards. Space resumes playback.
2. Next frame uses mpv `frame-step`; previous frame uses `frame-back-step`. Previous frame has to decode again from the preceding keyframe, which can take a few hundred milliseconds on sources with a long GOP.
3. Only one frame-step request is in flight at a time. Requests triggered by key repeat while one is in flight are dropped, not queued, so stepping does not continue after the key is released.
4. On-screen message "Seek: next frame 00:01:23.456" / "Seek: previous frame …", with the time in milliseconds (the `position` after completion).
5. Edge cases:
   - Next frame on the last frame: nothing moves; message "Already at the last frame".
   - Previous frame on the first frame: nothing moves; message "Already at the first frame".
   - No video track (audio only): the command is disabled with the reason "No video".
   - Plugin simulator streams and live streams that cannot seek: previous frame is disabled with the reason "The current source cannot step back a frame". Live streams can still step to the next frame (mpv can decode the next frame); this is decided by `can.frameStep`.
   - Playback finished (`phase: ended`): previous frame is available, goes to just before the last frame and stays paused; next frame is disabled. Other seeks after the end (seek bar, ← →, `player.seek`) play on (interaction spec IA-PB-09).
6. A frame step triggered while a seek bar drag gesture is in progress: end the gesture first (consistent with the existing rule "any external change ends the gesture").

**Core dependencies**: core actions `frame-back` / `frame-forward` (reference §5.5), which call `player.step("frame", …)` internally (ruling R-P1, accepted after rewrite: no `can.*` added). After the hand-over these two commands are registered and run entirely by the core; playback does not register them, and the bottom bar has no buttons for them.

**Acceptance**
- Press F while playing: playback pauses, the picture moves one frame forward, and the time in the message grows by about 1/fps.
- After pressing D five times, the time goes back by about 5/fps.
- Hold F and release: stepping stops after release.
- In an audio-only file the menu items are grey.

### B08 Previous keyframe / next keyframe

**Requirement**: jump to the nearest keyframe before or after the current picture, for quick rough browsing.

**Entry point**: "Playback ▸ Seek ▸ Previous Keyframe (Ctrl+Shift+←) / Next Keyframe (Ctrl+Shift+→)", repeats while held.

**Behavior details**
1. "Previous" jumps to the nearest keyframe strictly earlier than the currently displayed time. When the picture is exactly on a keyframe, it jumps to the one before that, so repeated presses keep going back. "Next" works the same way.
2. The paused or playing state is kept.
3. One request in flight at a time; repeated triggers are dropped.
4. On-screen message "Seek: next keyframe 00:12:03 / 01:40:00".
5. Edge cases:
   - Sources that cannot seek: the command is disabled with the existing reason "Not supported by the current source".
   - Audio only (downgraded): the command is disabled with the reason "No video". `player.step` returns `unsupported` for sources without an active video; whether there is video is found by playback calling `stats()` once when each source first becomes ready, and cached (ruling R-P1). Plugin simulator streams are disabled the same way.
   - "Previous" while already at the first keyframe: jump to 0; message "Jumped to the start".
   - "Next" after the last keyframe: nothing moves; message "Already at the last keyframe".
   - Playback finished (`phase: ended`): "Previous" goes to the last keyframe before the position and stays paused; "Next" does not move ("Already at the last keyframe"); interaction spec IA-PB-09.

**Core dependencies**: `player.step("keyframe", direction, { ifCurrent })` (ruling R-P2 merged into R-P1; the core uses mpv `seek ±0.001 relative+keyframes`, which strictly passes the current keyframe in both directions).

**Acceptance**: on an H.264 source, press Ctrl+Shift+→ ten times: the time grows each time, and the intervals match the GOP (check against the keyframe times listed with `ffprobe -skip_frame nokey`); pressing Ctrl+Shift+← on a keyframe goes to the previous keyframe and does not stay in place.

### B09 Jump to start

**Requirement**: return to 0:00 with one key.

**Entry point**: "Playback ▸ Seek ▸ Jump to Start", Backspace.

**Behavior details**
1. `phase: ready`: `player.seek(0)`, keeping the paused or playing state.
2. `phase: ended`: `player.restart()`, consistent with the existing "press Space after the end to replay".
3. Cannot seek (live streams, simulator streams): disabled with the reason "Not supported by the current source".
4. On-screen message "Jumped to the start".
5. When an input box has focus, the input box takes Backspace and the command is not triggered (binding rule).

**Core dependencies**: `player.seek`, `player.restart`. No new core API needed.

**Acceptance**: play to 10:00 and press Backspace: playback returns to 0:00 and continues; press Backspace after the end: playback starts again; Backspace in the input box of the Open URL window only deletes characters.

### B29 Prevent screensaver / sleep during playback

**Requirement**: a toggle, on by default. During playback the screensaver does not start and the display does not sleep from idleness; after pause or stop, the system settings apply again.

**Entry point**: "Playback ▸ Disable Screen Saver While Playing" (checkbox).

**Behavior details**
1. "Playing" means `phase === "ready" && !paused`, including plugin simulator streams.
2. When the toggle is on, playback calls `plugin.power.preventSleep("playing")`, and `"off"` when it is off (called once at startup with the saved value). The assertion is held by the player, with arguments depending on whether there is video:
   - Video frames are shown (including plugin simulator streams): `caffeinate -d -i -w <player pid>`. This prevents both display sleep (so the screensaver does not start) and system idle sleep.
   - Audio only: `-i -w <player pid>`. Only system sleep is prevented; the display can turn off.
3. On pause, end, close, error or source change, the player releases or re-evaluates it; turning the toggle off or disabling playback withdraws its request. The assertion is tied to the player process and is not left behind.
4. On toggle, message "Disable screen saver while playing: on / off".
5. This is only a power assertion; it does not stop the user from locking the screen manually, nor sleep when the lid is closed.
6. (Downgraded) A failure to start `caffeinate` is only written to the player log; the plugin does not receive it and no message is shown (failure semantics in reference §16.2). The former item 5 "Cannot prevent the screensaver: <reason>" has been removed.

**Persistence**: `preventSleep: boolean` in `settings.json`, default `true`.

**Core dependencies**: `plugin.power.preventSleep` (general capability, round 2). mpv's `stop-screensaver` only works for its own video output; we use `vo=libmpv` with the host rendering, so it has no effect. Instead the player holds a `caffeinate` assertion according to the playback state, and plugins no longer start child processes for this themselves.

**Acceptance**
- While a video plays, `pmset -g assertions` shows caffeinate's `PreventUserIdleDisplaySleep` and `PreventUserIdleSystemSleep`.
- With the screensaver set to 1 minute, it does not start during 3 minutes of playback; after pausing, it starts after 1 minute.
- During audio-only playback there is only `PreventUserIdleSystemSleep`.
- After turning the toggle off, the assertion disappears.
- After disabling playback, no caffeinate process is left.

**Notes**: audio only is treated separately and only prevents system sleep.

### I06 Show time on seek bar hover

> The default has been superseded by IA-PB-01 in the [interaction spec](interaction-spec.md): off by default; users who have already saved a setting keep their value.

**Requirement**: a toggle, off by default. When the mouse rests on the seek bar, the time for that position is shown above the pointer.

**Entry point**: "Playback ▸ Show Time on Seek Bar Hover" (checkbox).

**Behavior details**
1. Shown when: the toggle is on, `can-seek` is true (duration known), and the pointer is inside the seek bar's 10px hit area.
2. Time = pointer fraction × duration, in the same format as the bottom bar time text (`MM:SS` under 1 hour, otherwise `H:MM:SS`).
3. While dragging, the time of the drag target is shown, matching the pointer position.
4. The tooltip is centered horizontally on the pointer, keeps 2px on each side and does not go past the window edges; its bottom edge is 4px above the progress line, drawn over the video area.
5. It disappears when the pointer leaves, another button is pressed, the bottom bar auto-hides (fullscreen), or the source changes. It shows immediately on hover, without the 600 ms wait of `ToolTipArea`.
6. The time is computed in Slint from `duration` and the pointer position, without a round trip through the plugin, so mouse movement has no lag. The plugin only provides `duration` (seconds) and the toggle.

**Interface**
- The bridge global gains `in-out property <float> duration` and `in-out property <bool> hover-time`.
- Tooltip: `Theme.bg-panel` background, 1px `Theme.line` border, padding 5×2, font size `font-body` 11px, `Theme.text`.
- The tooltip is an ordinary element in the bottom bar component, drawn with a negative `y` into the video area above the bottom bar, with its position updated on every move (ruling R-U4: the host composition does not clip the bottom bar, reference §6.3). No `PopupWindow`.

**Persistence**: `hoverTime: boolean` in `settings.json`, default `false` (IA-PB-01).

**Core dependencies**: `panels.mount`, `bind`. No new core API expected.

**Acceptance**
- 100-minute source, pointer resting in the middle of the seek bar: about `50:00` is shown, and the text follows when moving left and right.
- With the pointer at the far left or far right end, the tooltip does not go past the window.
- Not shown on live streams.
- Not shown after the toggle is turned off.

### I07 Show thumbnail on seek bar hover

**Requirement**: a toggle, off by default (needs ffmpeg and uses CPU). On hover, a 160×90 thumbnail of the picture at that position is shown above the time tooltip.

**Entry point**: "Playback ▸ Show Thumbnail on Seek Bar Hover" (checkbox).

**Behavior details**
1. Only for local files: `source` is an absolute path and `stats().video` is non-empty. Network links, simulator streams and audio only show just the time.
2. Slint sends `hover-changed(fraction)` when the hover position changes, at most once every 50 ms; on leaving it sends `hover-ended()`.
3. The plugin quantizes the time to 0.5% of the duration (at least 2 seconds) as the cache key, then:
   - Cache hit: show immediately.
   - Cache miss: generate after the pointer has rested for 80 ms: `ffmpeg -v error -ss <t> -i <path> -frames:v 1 -vf scale=160:90:force_original_aspect_ratio=decrease -q:v 5 <cache>/<key>.jpg`.
   - Generation rules: `-ss` goes before `-i` for fast keyframe seeking; no shell; at most 1 process at a time, 3-second timeout; when a new hover target appears, the old result is discarded — a running process finishes, but its result only goes into the cache.
4. While generating, the previous thumbnail (if any) is shown, otherwise a black box of the same size; the time text is shown as usual.
5. Cache: `plugin.dataDirectory/thumbs/<hash of source realpath + mtime>/`. At most 200 images per source; on source change the old source's directory is deleted; `thumbs/` is emptied when the plugin starts.
6. ffmpeg executable: `VESTRA_FFMPEG`, otherwise `ffmpeg` on PATH, the same rule as playlist's ffprobe. When not found, the menu item is disabled with the reason "Needs ffmpeg", without a popup message.
7. When generating one thumbnail fails, only the time is shown, with no message. After 5 consecutive failures, no more attempts are made for this source.
8. When the plugin is disabled, a running ffmpeg is terminated.

**Interface**: the same popup as I06, with a 160×90 image on top (`image-fit: contain`, `Theme.bg-video` background: plugin Slint does not hard-code colors) and the time text below. The bridge global gains `in-out property <bool> hover-thumbnail`, `in-out property <bool> has-thumbnail` (only the background box is drawn while there is no image yet), `in-out property <image> thumbnail`, `callback hover-changed(float)` and `callback hover-ended()`. When the toggle is on but the current source cannot produce thumbnails (network, audio only, 5 consecutive failures), `hover-thumbnail` is false and the popup shows only the time.

**Persistence**: `hoverThumbnail: boolean` in `settings.json`, default `false`.

**Core dependencies**: the existing `panels.mount`, `bind`, `player.stats()`, `tools.run("ffmpeg")`. [New core API] (provided in round 3b, reference §6.1):

```ts
// Wraps an image file under this plugin's dataDirectory or package directory as an image reference, passed as a panel / window property value; the host reads the file and converts it to a Slint image
plugin.ui.image(path: string): ImageRef   // absolute path, PNG/JPEG, ≤ 16 MB and ≤ 4096×4096, under this plugin's dataDirectory or package directory (thumbnails go in dataDirectory)
type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
type PropValue = Json | ImageRef;         // ImageRef is an opaque value that only plugin.ui.image can create
```

Reason: the UI bridge only accepts JSON (ui_values.rs explicitly rejects image), so plugins cannot hand images to Slint. The plugin generates thumbnails itself with ffmpeg; the core only "loads an image by path". `media.capture` + `export` is not used: it is designed for screenshots and exports full-size PNG files through an offline job, which is too slow for hover previews.

**Acceptance**
- With the toggle on, move over the seek bar of a 1080p local mkv: a thumbnail appears within 200 ms, matching the picture at that time.
- Hovering at the same position a second time shows it immediately (cache hit).
- On a YouTube source only the time is shown.
- After removing ffmpeg the menu item is grey.
- Moving quickly back and forth for 10 seconds: `ps` shows at most 1 ffmpeg at a time.

### I08 Copy file name / path

**Requirement**: copy the current source's file name, folder path or full path to the clipboard.

**Entry point**: "Copy to Clipboard ▸ File Name · Folder Path · Full Path".

**Behavior details**
1. Local file (`source` starts with `/`):
   - File name: `basename`, with extension.
   - Folder path: `dirname`.
   - Full path: as is.
2. Network source: only "Full Path" is available and copies the original link. "File Name" and "Folder Path" are disabled with the reason "A network source has no file name".
3. No source, or a generated stream (`kind` is `"stream"`, e.g. a simulator; its `source` is `producer://<n>` or the path of the opened ROM and is not treated as a file path): all three are disabled with the reason "No open file".
4. Write to the clipboard: `plugin.clipboard.writeText(text)` (written to the system clipboard as UTF-8 text, so Chinese paths are not garbled).
5. On-screen message "Copied: <text>". Text longer than 60 characters is shortened in the middle (keeping the first 20 and the last 37 characters), within the OSD limit of 200 characters. On failure the message is "Copy failed: <reason>".

**Core dependencies**: `player.state.source`, `plugin.clipboard` (general capability, round 2).

**Acceptance**: while playing "/Users/x/电影/a b.mkv", the three items paste "a b.mkv", "/Users/x/电影" and "/Users/x/电影/a b.mkv" respectively, with correct Chinese; with a YouTube source only the full path is available; after closing the source all three are grey.

**Notes**: there is no "Other" menu; following the file domain, these go into the top-level open group.

### Basic playback actions handed over to the core

Based on the last section of the [feature catalog](feature-catalog.md#about-the-shortcut-settings-plugin-j01j03), "About the shortcut settings plugin": the shortcut settings plugin (J01) must still be able to bind basic operations after playback is uninstalled or replaced, so these operations have to be registered by the core, like "Fullscreen" and "Quit" today.

**Basic actions registered by the core**

Menu paths, order and default bindings are the same as now; the menu the user sees does not change:

| Core action id | Menu path | order | Default binding | playback command it replaces |
| --- | --- | --- | --- | --- |
| `play-pause` | Playback ▸ Play/Pause | 201 | Space | `toggle-pause` |
| `previous` | Playback ▸ Previous | 202 | PageUp, Mouse:X2 | `previous` |
| `next` | Playback ▸ Next | 203 | PageDown, Mouse:X1 | `next` |
| `frame-back` / `frame-forward` | Playback ▸ Seek ▸ Previous Frame / Next Frame | 204 / 205 | D / F | None (new in B07) |
| `seek-backward` / `seek-forward` | Playback ▸ Seek ▸ Back/Forward N seconds (step 1) | 210 / 211 | ← / → (repeat) | `backward-5` / `forward-5` |
| `rate-down` / `rate-up` | Playback ▸ Speed ▸ Slower 0.1 / Faster 0.1 | 221 / 222 | X / C | `rate-down` / `rate-up` |
| `volume-up` / `volume-down` | Audio ▸ Volume + / Volume − | 501 / 502 | ↑, wheel up / ↓, wheel down (repeat) | `volume-up` / `volume-down` |
| `mute` | Audio ▸ Mute (checkbox) | 503 | M | `mute` |
| `stop` | Close | 120 | F4 | `close-source` |

We suggest also handing over `rate-10` (Speed ▸ Normal, Z): it belongs to the same group of basic keys as X / C, and after the hand-over the shortcut plugin can bind it too. The original analysis only said "speed ±"; this one is left to the maintainer to decide.

**Existing behavior the core must take over** (moved over from `controller.ts`, behavior unchanged)
- Play / pause: with `phase: ended` it replays instead; message "Paused" / "Play".
- Relative seek: keeps adding from the latest unfinished target (repeated → presses do not lose steps); clamped between 0 and the duration; message "Forward 5 s: 00:37 / 01:40"; uses the step set with `setStepSeek`.
- Seeking after the end (2026-11-09, IA-PB-09): ← → and the seek bar work at `phase: ended`; a seek that leaves the end plays on, unless it is previous frame or the end was reached while paused.
- Speed: ±0.1, rounded to 0.01, clamped to 0.25–4; message "Speed: 1.1x".
- Volume: ±5, clamped to 0–200, based on the latest draft value; set the volume first, then unmute; messages "Volume: 85%" and "Mute: on / off".
- Previous / next: handed to the playback sequence provider; when there is none, disabled with the reason "No previous item / No next item".
- Each action's `enabled` / `busy` / `reason` is the same as now; the on-screen messages are shown by the core with the same text as now.

**Changes in playback**
- Remove the 11 commands in the "replaces" column of the table above (`toggle-pause`, `previous`, `next`, `backward-5`, `forward-5`, `rate-down`, `rate-up`, `volume-up`, `volume-down`, `mute`, `close-source`). Keep registering: `open`, `reopen` (renamed by A10), seek steps 2–4 (`backward-30` etc., IDs change to `backward-2…4` with B06), `jump`, `rate-10` (if not handed over), `rate-05/15/20`, `rate-custom`, and the commands added in this chapter.
- Bottom bar buttons call the core actions instead, so buttons, menus and shortcuts share one behavior and one message:
  - Play / pause → `play-pause`; stop → `stop`.
  - Back / forward → `seek-backward` / `seek-forward`; right-click → `previous` / `next`.
  - Mute icon → `mute`.
  - The volume slider needs continuous values and still uses `player.setOutput` directly, with the same rules as the core's `volume-up/down`.
  - Seek bar dragging still uses `player.scrub`.
- The enabled state of the bottom bar buttons is still derived from `player.state` (`can.*`, `phase`); there is no need to read the state of core commands.
- B06's step 1 is handed to the core through `player.setStepSeek`.
- Existing bindings for `vestra.playback/<old id>` in the user overlay: when the core is upgraded, they are mapped once to the new core action ids. There is no rebinding interface yet, so the actual impact is small, but the mapping has to be written into the upgrade logic.

**Core dependencies** [New core API]:

```ts
type HostAction =
  | "play-pause" | "stop" | "previous" | "next"
  | "frame-back" | "frame-forward"
  | "seek-backward" | "seek-forward"
  | "rate-down" | "rate-up" | "rate-normal"
  | "volume-up" | "volume-down" | "mute";
plugin.host.run(action: HostAction): Promise<void>       // needs commands; fails with unavailable when the action is disabled
player.setStepSeek(options: { seconds: number }): Promise<void>
// seconds 0.1–3600, default 5; back to the default after the calling plugin is disabled
```

Reasons:
- "Running a core action" is not communication between plugins: the core is a party that is always there and does not depend on any plugin.
- Without `host.run`, the bottom bar would have to re-implement the actions itself, and buttons and shortcuts would end up with two sets of behavior and messages (violating interaction requirements §1 "shared state and behavior").
- `setStepSeek` keeps step 1 configurable (B06), while the setting is still saved by playback.

**Acceptance**
- After the hand-over, uninstall playback: Space, ←/→, D/F, ↑/↓, M, PageUp/PageDown, X/C and F4 still work, and the message text is unchanged.
- Reinstall playback: the effect and messages of the bottom bar buttons and the shortcuts are exactly the same.
- Set step 1 to 10 seconds: ← goes back 10 seconds; after disabling playback, ← goes back to 5 seconds.
- The menu snapshot is the same as before the hand-over (only with the items added in this chapter).

### Chapter core API request summary

| Signature | Reason | Source |
| --- | --- | --- |
| ~~`player.stepFrame(…)`, `can.frameStep` / `can.frameBackStep`~~ → merged into `player.step(unit, direction, opts?)`, no `can.*` added (ruling R-P1) | Frame stepping is an mpv command that plugins cannot reach | B07 |
| ~~`player.stepKeyframe(…)`~~ → `player.step("keyframe", …)` (ruling R-P2) | "Strictly pass the current keyframe" can only be done in the core with mpv's keyframes-direction seek | B08 |
| `plugin.ui.image(path: string): ImageRef`, property values may be `ImageRef` (PNG/JPEG under dataDirectory) | The UI bridge only accepts JSON; plugins cannot show images they generate | I07 |
| `plugin.host.run(action: HostAction): Promise<void>` | Bottom bar buttons call core basic actions, so buttons and shortcuts share one behavior | Hand-over |
| `player.setStepSeek({ seconds }): Promise<void>` | Step 1 ←/→ stays configurable after it is handed to the core | B06, hand-over |
| ~~(conditional) a `PopupWindow` inside a panel that can extend beyond the panel and follow the pointer~~ | Not needed: the bottom bar is not clipped, and the tooltip is drawn into the video area with a negative `y` (ruling R-U4, reference §6.3) | I06, I07 |

The hand-over of the basic actions itself (the core registering the 14 actions above) is a change of core responsibilities; we suggest combining it with the shortcut interface (J01) into one round, done top-down.

---

## Playlist (`playlist`)

**Location**: `src/plugins/playlist/` (existing plugin)

### Scope

playlist is the sidebar playlist and the playback sequence provider (`player.provideSequence`). It already has: adding several files, adding links, multi-select, drag to reorder, move to top / up / down / bottom, sort by name / path / reversed, loop, shuffle, ffprobe durations, and the list context menu. This round completes it with the everyday features of a full playlist: adding entries from folders, list files and drag and drop; more sort orders, remove unselected / invalid items; search, show in folder; restore the list after restart; "add same-folder / similar files when opening a file"; fast advance to the next item (B26, the former "gapless playback", downgraded by ruling R-P5). The list is still a single list (no multiple tabs).

Boundaries: playlist cooperates only through the core. It does not call history or playback commands and does not read their data. For sources opened by other plugins, playlist can only see `player.state` (path, `sourceId`, phase). So "add same-folder files when opening a file" is done by playlist itself by watching source changes (A13), and history and other plugins do not need to know that playlist exists. The only new core API needed is the window drop event (A15); preloading the next source (the original B26 plan) was rejected by ruling R-P5. Folder scanning and list file parsing are done by the plugin with Node; ffprobe, Finder and the settings file use SDK general capabilities (`plugin.tools`, `plugin.system`, `plugin.storage`, migrated in round 2).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| A03 | Open folder (add and play) | ✗ | No |
| A13 | Add same-folder / similar files when opening a file | ✗ | No |
| A15 | Drag files onto the window to open | ✗ | **Yes**: window drop events (`drops.provide`, `Panel.onDrop`) |
| B26 | Fast advance to the next item (formerly "gapless playback", downgraded by R-P5) | ✗ | No (benefits from the host's "push phase changes immediately") |
| C04 | Add folder | ✗ | No |
| C07 | Remove unselected / remove invalid items | ✗ | No |
| C11 | More sort orders | ◐ (name / path / reversed) | No |
| C17 | Restore the list after restart | ✗ | No (`plugin.dataDirectory`) |
| C18 | Open / save M3U, M3U8, PLS | ✗ | No |
| C21 | Search the list | ✗ | No |
| C23 | Show in Folder | ✗ | No |

### Menus and shortcuts

**Main menu** (new rows in bold; `order` is within playlist's range 230–249 in the allocation table; "Open Folder" takes 110 in `Open ▸`):

| Menu path | order | group | Default binding | Notes |
| --- | --- | --- | --- | --- |
| **Open ▸ Open Folder…** | 110 | open | F2 | A03; the `Open ▸` submenu is therefore placed at 110 |
| Playback ▸ Playlist ▸ Add Files… | 230 | domain | | Existing |
| Playback ▸ Playlist ▸ Add Link… | 231 | domain | | Existing |
| **Playback ▸ Playlist ▸ Add Folder…** | 232 | domain | | C04 (the former 232–235 move down) |
| Playback ▸ Playlist ▸ Loop ▸ Off / All / One | 233–235 | domain | | Existing, radio |
| Playback ▸ Playlist ▸ Shuffle | 236 | domain | | Existing |
| **Playback ▸ Playlist ▸ Open List File…** | 237 | list | | C18 |
| **Playback ▸ Playlist ▸ Save List…** | 238 | list | | C18 |
| **Playback ▸ Playlist ▸ When Opening a File ▸ Open Only That File / Add Similar Files / Add All Files in the Folder** | 239–241 | open-mode | | A13, radio, default "Add Similar Files" (IA-PL-01) |
| **Playback ▸ Playlist ▸ Fast Advance to Next** | 242 | options | | B26, checkbox, off by default |
| **Playback ▸ Playlist ▸ Remember List** | 243 | options | | C17, checkbox, on by default |
| Playlist (checkbox) | 800 | windows | F6 | Existing |

**List commands** (not in the main menu; they appear in the list context menu / ADD / SORT; a key works only while the list or its search box has focus, `focus: "panel"`). Coordinator ruling 2026-11-08 (playlist 0.2.6): no list key is a global key; the commands whose keys collided (P, Ctrl+I, Ctrl+O, Ctrl+U, Backspace, Ctrl+T, Ctrl+Home, F2, F3, F5, F6) have none and are reached from the list menu and the palette. See [menus](menus.md#when-the-playlist-has-focus) for the focus rules and the navigation keys the list handles itself:

| Command | Binding | Appears in | ID |
| --- | --- | --- | --- |
| Play | Enter | Context menu | Existing |
| Add Files… | (no key; Ctrl+I stays Deinterlace: Cycle) | Context menu, ADD | C03 |
| **Add Folder…** | (no key; Ctrl+O stays Open File…) | Context menu, ADD | C04 |
| Add Link… | (no key; Ctrl+U stays Open URL…) | Context menu, ADD | C03 |
| Remove Selected | Delete (Backspace stays Jump to Start) | Context menu | Existing |
| **Remove Unselected** | Ctrl+Delete | Context menu | C07 |
| **Remove Invalid Items** | (no key; Ctrl+T stays Always on Top: Cycle Mode) | Context menu | C07 |
| Remove All | **Alt+Delete** | Context menu | Existing command, key added |
| **Show in Folder** | Ctrl+F | Context menu | C23 |
| Select All | Ctrl+A | Context menu | Existing |
| **Search** | Ctrl+S | Context menu | C21 |
| **Open List File…** | (no key; F3 stays Open File…) | Context menu | C18 |
| **Save List…** | (no key; F2 stays Open Folder…) | Context menu | C18 |
| **Ascending / Descending** (radio) | Ctrl+1 / Ctrl+2 | SORT | C11 |
| **By Title / File Name / Extension / Size / Duration / Date / Random Order / Folder** | Ctrl+3 … Ctrl+0 | SORT | C11 |
| Sort by Path | | SORT | Existing |
| Reverse Current Order (the former "Sort Reversed", renamed) | | SORT | Existing |
| Close List | (no key; global F6, Playlist, hides the list too) | Context menu | Existing |
| Playlist Settings… | (no key; F5 stays Options…) | Palette only | interaction-spec IA-PL-02: opens "Options ▸ Playback ▸ Playlist" |

**Conflicts and trade-offs**:

1. With list focus, `focus: "panel"` is looked up before global, but no list binding is a global key, so no global command is shadowed by a list command. While the list has focus the list itself (in `sidebar.slint`) takes only the standard list keys: ↑ ↓ PgUp PgDn Home End move the selection (covering Volume ±, Previous/Next and Previous/Next Dialogue), Enter plays, Esc gives the keys back to the video (covering Toggle Fullscreen). Every other key, Space and ← → included, reaches the player. The list gets the focus only from a single click on a row; playing (double-click, Enter), Esc and a click on the video give it back to the video; the wheel, the buttons and a drag leave the focus unchanged.
2. **Remove Unselected is bound only to Ctrl+Delete** (forward delete), not Ctrl+Backspace: the latter equals ⌘⌫, which is "Move to Trash" in Finder, and users could easily think it deletes files from disk. ⌘⌫ is not bound in the list.
3. The list context menu is single-level with at most 32 items (reference §6.3). Following the table above, the context menu has 13 commands + 6 separators = 19 items, and the sort items are in the SORT popup menu (12 items); both are within the limit.

List context menu order: Play | Add Files…, Add Folder…, Add Link… | Remove Selected, Remove Unselected, Remove Invalid Items, Remove All | Show in Folder | Select All, Search | Open List File…, Save List… | Close List. ADD popup menu: Add Files…, Add Folder…, Add Link….

### A03 Open folder

**Requirement**: the user picks a folder; the media files in it (including subfolders) are added to the current list in natural order, and the first added file starts playing immediately.

**Entry point**: context menu "Open ▸ Open Folder…", global F2.

**Behavior details**:

1. Show `dialogs.openDirectory({ title: "Open Folder" })`; on cancel nothing happens.
2. Scan: recursive, maximum depth 8 levels, collecting at most 5000 media files (the rest are dropped, and the OSD gets ", limit of 5000 reached" at the end). Only media extensions are collected, not list files or subtitles. The order is "by folder" (compare the parent directory path first, then the file name, both in natural order), so a subfolder's contents come after the folder's own files, ordered by subfolder name.
3. By default the list is cleared first (no confirmation; not cleared while another source is being opened), then the files are added; when "Append when opening or dropping; don't clear the list" is on, they are added to the end of the list, with the same de-duplication rule as "Add Files" (same `realpath` means duplicate).
4. Playback: play the first **newly added** row; if all are duplicates, play the existing row that matches the first file in the folder. While another open is in progress (`player.busy()`), files are only added, not played, and the OSD says so.
5. While scanning, the command shows busy (`busy`) and further triggers are ignored. The scan runs asynchronously in Node and yields to the event loop every 200 directory entries; if it has not finished after 1 second, the OSD shows "Scanning folder…". When the plugin is disabled, the scan is aborted.
6. The clearing in item 3 has been rewritten by item 4 of IA-PL-01 in interaction-spec.md: the former "append by default, optional `clearOnOpenFolder`" is void; `clearOnOpenFolder` is merged into "Append when opening or dropping; don't clear the list" (off by default), and the old field is ignored when read.
7. When the sidebar is hidden, files are added and played as usual; the sidebar is not forced to show.

OSD:
- Success: "Added N items" (the existing wording, followed by ", M duplicates skipped" and ", K subfolders could not be read").
- No media: "No playable files in the folder".
- The folder itself cannot be read: "Cannot read the folder: <reason>". When macOS privacy protection refuses (`EPERM`), the reason reads "Access to this location is not allowed (System Settings ▸ Privacy & Security ▸ Files and Folders)".

**Core dependencies**: the existing `dialogs.openDirectory`, `player.open`. Nothing new.

**Acceptance**:
- Pick a directory with 3 videos, 1 `.txt` and 1 subfolder (2 mp3s): the list gains 5 rows, in the order 3 videos (natural order) followed by 2 mp3s, and the first row starts playing.
- Pick the same directory again: the list is replaced by the same 5 rows and the first row plays; with the "append" setting on, pick it again: OSD "Added 0 items, 5 duplicates skipped", and the row matching the first file plays.
- Empty directory: OSD "No playable files in the folder"; the list does not change.
- Press F2 again during the scan: no second dialog appears.
- A directory with a symbolic link loop: the scan ends, nothing is added twice.

### A13 Add same-folder / similar files when opening a file

> Rewritten by IA-PL-01 in the [interaction spec](interaction-spec.md) (user ruling): the default is "Add Similar Files"; the list is cleared first on open; "similar" and "all" are decided by the rules in the "same-kind file rules" subsection of IA-PL-01. The text below has been rewritten to the new rules.

**Requirement**: when the user opens a local file through another entry point (the bottom bar's "Open File", recent playback, site plugins, etc.), the playlist is cleared first, then this file is added together with the "similar files" or "all files in the same group" from the same folder; the opened file becomes the current row, and next / automatic advance work afterwards.

**Entry point**: "Playback ▸ Playlist ▸ When Opening a File ▸", one of three (radio); also on the settings page "Options ▸ Playback ▸ Playlist":
- **Open Only That File**: after clearing, the list contains only this file, and the plugin adopts it.
- **Add Similar Files** (default)
- **Add All Files in the Folder**

When the setting "Append when opening or dropping; don't clear the list" (off by default; the same setting also covers A03 "Open Folder" and A15 drops on the video area) is on: the list is not cleared, files are appended with de-duplication (see items 4 and 6), and with "Open Only That File" the list does not change and the file is not adopted (UI-PL-20).

**Behavior details**:

1. Trigger: `player.onChange` sees a new `sourceId`, `kind === "media"`, `source` is an absolute local path (no `://`), and this source was **not opened by this plugin** (every `Media` returned by this plugin's `player.open` is recorded). URLs and simulator streams do not trigger it.
2. Deciding "similar files" and "all files": follow the "same-kind file rules" of IA-PL-01 in interaction-spec.md (`media.ts` `siblings` / `similarStep`). In short: same folder, not recursive; video only matches video, audio only matches audio; "similar" requires the opened file name to contain a space, `-`, `_` or an ASCII digit, then four comparison steps S1–S4; "all" only collects files of the same group; files starting with `.` are skipped (macOS `._*`).
3. No separate adjustable limit; when more than 5000 files match, 5000 are taken centered on the opened file, and the OSD ends with "limit of 5000 reached".
4. How files are added: by default the list is cleared first (no confirmation), then files are added in natural order, including the opened file itself and the files before it. With the "append" setting on, they are appended to the end of the list instead, skipping files already in the list (`realpath`).
5. **Adopt**: the row of the opened file is marked as the current row (blue). Because it was not opened by this plugin, there is no `Media` handle; while adopted:
   - Current row rule: as long as `player.state.sourceId` equals the `sourceId` at adoption, the row counts as still playing; when `sourceId` changes, the adoption ends (the same as the existing "switched to a source outside the list", with no message).
   - Natural end: when `onChange` sees `phase === "ended"` and `endReason === "natural"` for the same `sourceId`, advance by the existing advance rules (loop, shuffle) with `player.open(next, { ifCurrent: endedState })`; `stale` means someone else has changed the source, and advancing is abandoned.
   - "Loop one": `player.restart({ ifCurrent: endedState })`.
   - `canNext`/`canPrevious` are computed with the adopted row as the anchor, so the bottom bar's previous / next and PgUp/PgDn work immediately.
6. "Append" setting on and the file already in the list: no new row is added, and that row is adopted directly; if the mode is "Open Only That File", it is **not adopted** (UI-PL-20). In clear mode, old rows are not reused. When the plugin starts (or is re-enabled) and the file playing is already in the restored list, it is only adopted, without clearing.
7. Reading the folder fails (permissions, etc.): only this one file is added and adopted (in clear mode the list contains only it), with no message.
8. Each open is handled only once; a replay (`restart`) of the same `sourceId` is not handled again.
9. OSD: when new rows besides the opened file were added, "Added N files from the same folder" (N does not include the opened file; in append mode it is the number of newly added rows); otherwise no message.
10. Relation to A15 and C18: drops on the video area are handled by playlist itself (A15); dropping a single media file is expanded with the same mode and same-kind file rules as this item (difference item 9 of IA-PL-01), and not expanded when Option is held; this rule only applies to sources opened by other plugins. Other plugins open only one source at a time, so the rule "multi-select opens are not expanded" is met automatically.

**Persistence**: the mode is stored in `settings.openMode` (`"only" | "similar" | "folder"`, see C17). An old file with `settings.version` below 2 has `openMode` and `appendOnOpen` migrated once to the new defaults (similar files, no append) when read; the other settings are kept. It is then saved as version 2, and values the user changes later are saved as usual.

**Core dependencies**: the existing `player.onChange`, `player.state` (`sourceId`, `source`, `phase`, `endReason`), `player.open({ ifCurrent })`, `player.restart`, `provideSequence`. Nothing new: adoption only relies on public state and does not need another plugin's `Media`.

**Acceptance**:
- The list has 3 items, default mode, the folder has `A01.mkv A02.mkv A03.mkv B.mkv song.mp3`; open `A02.mkv` with the bottom bar's "Open File": the list becomes `A01 A02 A03`, `A02` is the blue current row; after it ends, `A03` plays automatically.
- Mode "Add All Files in the Folder", same as above: the list has 4 rows `A01 A02 A03 B` (audio not included).
- Mode "Open Only That File": the list contains only the opened file, as the current row.
- "Append" setting on, mode "Open Only That File": the list does not change, and PgDn does not advance (`canNext` is false unless the list itself has a current row).
- The 17 same-kind file cases of IA-PL-01 (`tests/similar.test.ts`).
- After adopting `A02`, open another URL from outside: `A02` is no longer the current row, with no message.
- Open a URL: the list does not change.

**Notes**: files starting with `.` are skipped; extensions are matched as whole extensions against the set in Appendix B.5; sorting uses `Intl.Collator("zh", { numeric: true })` (natural order); more than 5000 matches are truncated.

### A15 Drag files onto the window to open

> Rewritten in round 3b by ruling R-U2: `kinds` → `extensions`; there is no `onDragOver` and no insertion line is drawn; items are inserted before the row under the drop point; for the same extension, the first registration wins; with no handler, the core does nothing.
>
> Behavior item 1 has been superseded by G-11 in the [interaction spec](interaction-spec.md) (dropping on the video area clears the list first, then adds and plays); behavior item 2 and acceptance item 4 have been superseded by G-12 (dropping on the list appends to the end by default; "insert at the drop position" is an option, off by default). The text below has been rewritten to the new rules.

**Requirement**: dragging files, folders or list files from the file manager (Finder on macOS, File Explorer on Windows) onto the Vestra Player main window (video area) or the playlist sidebar (attached or floating) opens or adds them.

**Entry point**: drag and drop. No menu, no shortcut.

**Behavior details**:

1. **Drop on the video area** (the core groups by extension and hands each group to its registrant; playlist registers the media extensions (Appendix B.5), the list files `m3u m3u8 pls dpl`, `webloc` and the reserved name `"folder"` with `drops.provide`):
   - Folders are expanded by the A03 rules (recursive, limit 5000; macOS bundles such as `.app` are not expanded and are skipped); list files are parsed into entries by C18; media files are taken directly. When several items are dropped, they are taken in the order the file manager gives, each expanded and then concatenated.
   - When only **one** local media file is dropped, it is expanded by the A13 "When Opening a File" mode (see IA-PL-01 in interaction-spec.md): "similar" adds the similar files of the same folder, "all" adds all files of the same group, "Open Only That File" does not expand; the dropped file is the one played. Several files, folders or list files are not expanded.
   - When at least one item can be added, **the list is cleared first** (without the "Confirm Before Removing All" prompt), then items are added in drop order and **the first entry plays** (G-11). When the setting "Append when opening or dropping; don't clear the list" (playlist settings page, off by default) is on: they are added to the end of the list (de-duplicated), and the first new entry plays.
   - Released with Option held (Alt on Windows; `modifiers.alt`): only appended to the end, without clearing or playing (OSD: "Added N items").
2. **Drop on the playlist** (`Panel.onDrop`; the whole group of paths goes to playlist, not split by extension):
   - No playback, no clearing; items are only added. By default they are **appended to the end** (G-12). When the setting "Insert at the drop position when dropping on the list" (off by default) is on: they are inserted **before** the row under the release point; dropping on empty space (below the last row), the tab strip or the bottom button bar still appends to the end. There is no insertion line while dragging (the core does not provide positions during the drag).
   - The drop point is in panel coordinates (used only when "insert at the drop position" is on); playlist converts it to a row with the scroll position and list area height reported back by the sidebar (the `list-area` callback). While a search filter is active, the conversion uses the displayed rows, and items are inserted before that row's position in the full list.
   - Files in the group that playlist does not accept (subtitles, etc.) are skipped, and the OSD ends with "N unsupported items skipped".
3. Subtitle files (`srt ass ssa vtt …`), images and other types are not registered by playlist; the core hands them to the plugin that registered the matching extension (for example the subtitles plugin), and does nothing when there is no handler (no message). playlist does not handle subtitles and does not call the subtitles plugin.
4. All dropped entries are duplicates (only possible when appending): a drop on the video area plays the existing row matching the first one; OSD "Added 0 items, N duplicates skipped".
5. Web links: the core only reports file paths, not URLs dragged from a browser; for `.webloc` files dragged out of Safari/Chrome, the `URL` field of their plist (XML or binary `bplist00`; binary is parsed by the plugin in pure JS without calling system tools) is used as a link row (only HTTP(S) is accepted; anything else is skipped).
6. While the player is opening another source (`busy`), a drop on the video area does not clear and only appends; the OSD says "…; opening, added to the list".
7. When the sidebar is hidden for lack of space (`reason: "space"`), dropping on the video area still works.
8. None of the dropped content can be added (all unsupported, folder unreadable): the list does not change; OSD "Nothing to add: …". Several drops are handled one after another in arrival order.

**Interface**: `ui/sidebar.slint` gains `callback list-area(float, float)` (scroll distance and list area height, in logical points), reported when the list area is initialized, changes height, and scrolls. No drop insertion line is drawn.

**Core dependencies**: window drag and drop (round 3b batch 2, capability `ui.drop`, reference §6.7). Why the plugin cannot do it: system drag and drop only reaches the host's native window; plugin Slint components do not receive system drops; and the video area belongs to no plugin, so the host has to dispatch by drop point and extension.

```ts
interface DropModifiers { alt: boolean; shift: boolean; cmd: boolean }
drops.provide(handler: { extensions: string[] /* lower case, no dot; "folder" = folders */; drop(e: { paths: string[]; modifiers: DropModifiers }): unknown }): Disposable;
Panel.onDrop(listener: (e: { paths: string[]; at: { x: number; y: number } /* panel coordinates */; modifiers: DropModifiers }) => unknown): Disposable;
```

Host rules: on release over the video area, `paths` are grouped by extension (keeping the order the file manager gives), and each group goes to the matching provider in one call; when several providers register the same extension, the first registration wins; files nobody claims are ignored by the core. On release over a panel, the whole group goes to that panel's `onDrop`; with no listener, the video area rules apply. Drag and drop does not pause or change playback.

**Acceptance**:
- The list has 5 items; drop 1 mkv on the video area: the list contains only this item and it plays; with "Append when opening or dropping" on, drop again: it is appended as item 6 and plays.
- Drop 1 folder on the video area: it is expanded by the A03 rules and the first item plays.
- The folder has `A01 A02 A03 B`; drop `A02.mkv` on the video area: the list is `A01 A02 A03` and `A02` plays; drop `B` and `A02` together: the list has only these 2 items.
- Drop 3 files on the video area with Option held (Alt on Windows): 3 rows are added, nothing plays.
- Drop 2 files on row 4 of the list: appended to the end, nothing plays; with "Insert at the drop position when dropping on the list" on, drop on row 4 again: the two rows are inserted before the former row 4; dropping on empty space appends to the end.
- Drop a `.m3u8` on the video area: the list is replaced by the parsed entries and the first item plays.
- Drop a `.srt` on the video area with no plugin registered for srt: the list does not change, no message.
- With the sidebar floating, drop on the floating window: the same as when attached; with the sidebar hidden for lack of space, dropping on the video area still works.

**Notes**: no "play / add" choice popup appears on drop; the Option key (Alt on Windows) makes the distinction; dropped files are not sorted again and keep the order the file manager gives.

### B26 Fast advance to the next item

> Downgraded in round 2 (ruling R-P5): the original "gapless playback (preload the next item)" needed `player.preload`, which was rejected. Measurements (negotiation record R-P5) on the current host: the switch from natural end → opening the next item takes about 5–115 ms for the same format and about 60–165 ms when the sample rate changes; the host now pushes source phase changes immediately, removing the 0–100 ms periodic wait from that. This item is changed to make the plugin's side of the advance path as short as possible, without promising no gap: even a 5–10 ms break in a continuous waveform is audible as a click, and "true gapless" for live albums and classical music is given up this time. Raising it again later needs evidence from recorded waveforms.

**Requirement**: when enabled, a natural advance of the list to the next item starts playing as soon as possible (consecutive album tracks, segmented videos); the switch between files of the same format is < 150 ms.

**Entry point**: "Playback ▸ Playlist ▸ Fast Advance to Next" checkbox, off by default.

**Behavior details**:

1. When enabled, after the current row is opened by this plugin (or adopted by A13), playlist immediately computes "the next item after a natural end" (by the same advance rules: loop / shuffle / in order), runs `realpath` and `fs.stat` in advance for local files, and records it as the planned next item.
2. In shuffle mode the next item is drawn in advance; as long as it is still eligible (not played in this pass, still in the list), it is used when advancing, without drawing again.
3. When the "next item" changes (reorder, remove, switching loop / shuffle, editing the list), it is recomputed and `stat`ed again at the next interface refresh; when it has not changed, the disk is not read again.
4. Advance: in the `media.onEnd` callback, if the planned next item is still the advance target and its file check is complete, `player.open(next, { ifCurrent: ended })` is called once directly, without reading the disk again (the precomputed `realpath` result and duplicate check are used). If the plan is not ready or has expired, the ordinary advance is used (`realpath` first, then `open`).
5. The advance check finds the file no longer exists: it is not opened on advance; on-screen message "Cannot open <title>: the file does not exist"; the row is marked as failed (warning icon), and the advance goes to the item after it (ordinary advance). A file deleted after the check is handled as an ordinary open failure.
6. With "Loop one", or at the end of the list with "Loop off", there is no planned next item; the advance rules do not change (replay / "End of the list"). Link rows have no file check, and advancing to them is also a single `open`.
7. The user's manual "Next" or a double click on another row: an ordinary open; after the new row starts, the planned next item is recomputed.
8. Turning the toggle off: the plan is discarded immediately, and ordinary advances are used afterwards.
9. The plan does not change the current playback or `player.state`; it only reads the file metadata once.

**Persistence**: `settings.gapless` (boolean, see C17; the key name is kept from the original item).

**Core dependencies**: none. Benefits from the round 2 host behavior "push source phase changes immediately" (`ready → ended` no longer waits for the 100 ms cycle).

**Acceptance**:
- Enabled, play two test audio files of the same format (same sample rate) (e.g. two 44.1 kHz flac files): the switch from natural end to the start of the next item is < 150 ms. Measurement: start the host with `VESTRA_PLAYLIST_TIMING=1`; for each automatic advance stderr prints the milliseconds for "end → open accepted" and "end → new source ready"; take "end → ready". Or compare the last frame of the previous track with the first frame of the next in a screen or audio recording.
- The same test with the toggle off as a control (one more `realpath`; the difference is in milliseconds).
- Move the next row elsewhere during playback: the advance goes to the new next row.
- The next item's file is deleted before the current item starts: it is skipped on advance with an on-screen message, and the row shows a warning icon.
- Open a URL from outside during playback: no advance, no error (`ifCurrent` expired).

**Notes**: there is no preloading and no gapless switching (R-P5), only a shorter advance path; "merge several files into one playback" and "number of simultaneous reads" are not done.

### C04 Add folder

**Requirement**: add the media files in a folder (including subfolders) to the current list without playing.

**Entry point**: "Playback ▸ Playlist ▸ Add Folder…"; the list's ADD popup menu and context menu; the palette (no key: Ctrl+O stays Open File…, coordinator ruling 2026-11-08).

**Behavior details**: scan rules, limit, de-duplication, OSD and busy state are exactly the same as A03 (one shared scan function). Differences: nothing plays; when the list was empty, the first added row is selected (consistent with "Add Files"). Dialog title "Add Folder to Playlist". While scanning, both "Add Folder…" and "Open Folder…" show busy.

**Core dependencies**: the existing `dialogs.openDirectory`.

**Acceptance**: choose "Add Folder…" from the ADD popup menu and pick a directory: rows are added at the end, nothing plays; the same de-duplication and limit behavior as A03.

### C07 Remove unselected / remove invalid items

**Requirement**: with one command, keep the selection and delete the other rows; with one command, delete the rows whose files no longer exist. Only the list changes; nothing on disk is touched.

**Entry point**: list context menu "Remove Unselected" (Ctrl+Delete), "Remove Invalid Items" (no key; Ctrl+T stays Always on Top: Cycle Mode).

**Behavior details**:

1. Remove unselected: delete all rows in the list that are not selected. Disabled when nothing is selected or everything is selected (reason "Nothing to act on"). When the row currently playing is deleted, the behavior is the same as "Remove Selected": the media keeps playing, the binding is released, OSD "The current media was removed from the list". Running ffprobe jobs are aborted. OSD: "Removed N items".
2. Remove invalid items: run `fs.stat` on all local file rows in the list, 8 at a time; rows that do not exist (`ENOENT`/`ENOTDIR`) or are not regular files are deleted; permission errors (`EACCES`/`EPERM`) are **not** deleted (the file is still there and may become readable later). Link rows are not checked or deleted. The command is busy during the check. OSD: "Removed N invalid items" / "No invalid items".
3. While a search filter is active (C21), both only act on visible rows: "Remove Unselected" deletes visible rows that are not selected, and hidden rows are left alone.

**Core dependencies**: none.

**Acceptance**: select 2 of 5 rows and press Ctrl+Delete: 2 rows remain; delete a file in the file manager and press Ctrl+T: its row disappears and link rows stay; with no invalid items, OSD "No invalid items".

### C11 More sort orders

**Current state**: the SORT popup menu only has sort by name, sort by path and sort reversed.

**Requirement**: add the common sort keys and ascending / descending order.

**Entry point**: the bottom bar's SORT popup menu; Ctrl+1 … Ctrl+0 with list focus (see the menu table).

**Behavior details**:

| Key | Comparison | When there is no value |
| --- | --- | --- |
| By Title (Ctrl+3) | Displayed title (the title given by the list file, otherwise the file name), natural order | — |
| By File Name (Ctrl+4) | The basename of the path; for links, the last segment of the URL (decoded) | — |
| By Extension (Ctrl+5) | Extension, then file name when equal | No extension sorts first |
| By Size (Ctrl+6) | `stat.size` | Links and unreadable files sort last |
| By Duration (Ctrl+7) | Known duration (the player's report first, otherwise ffprobe) | Unknown sorts last |
| By Date (Ctrl+8) | File modification time `mtimeMs` | Links and unreadable files sort last |
| Random Order (Ctrl+9) | Fisher–Yates shuffle, not affected by ascending / descending | — |
| By Folder (Ctrl+0) | Parent directory path, then file name | Links sort last |
| By Path | Full path / URL (existing) | — |

1. Ascending (Ctrl+1) / Descending (Ctrl+2) is a radio state, ascending by default. Choosing a direction immediately re-sorts by "the last sort key"; if the list has never been sorted, by title. Choosing a sort key sorts in the current direction. Rows "with no value" still sort last in descending order.
2. Stable sort: equal items keep their original relative order.
3. The former "Sort Reversed" is renamed "Reverse Current Order" and keeps its behavior (reverse directly, independent of any sort key).
4. Keys that need `stat` (size, date): read asynchronously, 8 at a time; the SORT menu items are busy while reading; the results are discarded after this sort (not cached, files may change).
5. Sorting applies to all rows of the list (even with a search filter active) and does not change the selection or the current row; with B26 on, the next item is prepared again in the new order.
6. Disabled when the list has fewer than 2 rows (existing rule).

**Persistence**: the last sort key and direction (`list.sort`, see C17).

**Core dependencies**: none.

**Acceptance**: build 5 files that differ in size, date, duration and extension, run each sort key in turn and compare with the expected order; after Ctrl+2 the order is fully reversed (items with no value still last); two random orders give different results (the probability assertion injects a fixed random seed in tests).

### C17 Restore the list after restart

**Requirement**: after quitting and starting again, the rows of the list (order, titles), the selected rows, loop / shuffle and all settings of this chapter are restored; nothing plays automatically.

**Entry point**: "Playback ▸ Playlist ▸ Remember List" checkbox, on by default.

**Behavior details**:

1. When to save: after the list or settings change, writes are merged into one write after 1 second; when the plugin is disabled (`onDispose`), one synchronous write. Write method per reference §3.4: write a temporary file, then `rename`.
2. What is saved (`playlists.json`):
   ```json
   { "version": 1,
     "settings": { "version": 2, "remember": true, "repeat": "none", "shuffle": false, "openMode": "similar",
                   "gapless": false, "confirmClear": true,
                   "appendOnOpen": false, "dropAtPointer": false },
     "list": { "sort": { "key": "title", "descending": false },
               "cursor": 3, "selected": [3],
               "rows": [ { "kind": "file", "source": "/Users/…/a.mkv", "title": null, "duration": 1432.5 } ] } }
   ```
   `title` `null` means the file name is used; a non-empty value comes from a list file (`#EXTINF` / `TitleN` in C18); `duration` is the last known duration (shown immediately after restore and updated when the ffprobe result arrives).
3. Not saved: the binding of the row currently playing, the played (grey) marks, failure states, the search text.
4. Restore: read at startup; when the file is missing or fails to parse, the default (empty list) is used, and the bad file is renamed to `playlists.json.bad-<timestamp>` to keep a copy. Restored rows are not checked for existence (a large list does not read the disk at startup); missing files are handled by the existing failure path when played, or cleaned up with C07. After restore, local rows are queued for ffprobe again (cache hits are immediate).
5. Turning "Remember List" off: the list in `playlists.json` is deleted immediately (only `settings` is kept), and only settings are saved afterwards. OSD: "The list will no longer be remembered" / "The list will be remembered".
6. Capacity: at most 20 000 rows; beyond that, further additions are refused with the OSD "The list is full at 20000 items".
7. Restoring does not open any media and does not change the player state; if the player already has a source at startup (for example opened from the command line), it is handled by the A13 mode.

**Persistence**: `<plugin.dataDirectory>/playlists.json`; ffprobe duration cache `probe-cache.json` (realpath + mtime → seconds, at most 20 000 entries, LRU).

**Core dependencies**: the existing `plugin.dataDirectory`.

**Acceptance**:
- Add some rows to the list (some from an m3u8 with titles), set loop to "All", quit and restart: rows, titles and order are all restored, and nothing plays automatically.
- Turn "Remember List" off and restart: the list is empty, and the loop setting is still "All".
- Corrupt `playlists.json` by hand: startup gives an empty list, and a `.bad-` backup appears in the directory.
- Test: pass the same `dataDirectory` to `startPlugin` twice to verify the restore.

### C18 Open / save M3U, M3U8, PLS lists

**Requirement**: read `.m3u` `.m3u8` `.pls` list files into the playlist; save the current list in one of these three formats.

**Entry point**: "Playback ▸ Playlist ▸ Open List File… / Save List…"; the list context menu; the palette (no key: F3 / F2 stay Open File… / Open Folder…); drag and drop (A15).

**Behavior details (open)**:

1. `dialogs.openFile({ title: "Open Playlist File" })`. The system dialog has no type filter, so the plugin decides by extension; when it is not `m3u m3u8 pls dpl`, OSD "Unsupported list format".
2. The result **replaces** the current list contents: the media playing keeps playing, and its binding to the list row is released (as with "Remove All"). Nothing plays automatically (a drop on the video area replaces the list and plays the first item per A15 / G-11; a drop on the list appends or inserts per A15 / G-12, without replacing). When the list is not empty, a confirmation comes first: "Replace the current list with <list file name>?" [Replace] [Cancel].
3. Parsing:
   - **M3U/M3U8**: line by line; `#EXTM3U` is optional; `#EXTINF:<seconds>,<title>` gives the title and duration of the next entry (seconds ≤ 0 means unknown); other `#` lines are ignored; empty lines are ignored. Entries can be absolute paths, relative paths (relative to the list file's directory), `file://` URLs (decoded to paths), or `http(s)://` URLs. Other protocols (`rtsp://`, `ftp://`, etc.) are added as link rows as they are (playback goes to mpv; the Add Link dialog's restriction to HTTP(S) does not apply to list files).
   - Encoding: `.m3u8` is read as UTF-8; `.m3u` is first decoded strictly as UTF-8, then as GB18030 on failure (`TextDecoder("gb18030")`), then as Latin-1. The UTF-8 BOM is removed.
   - **PLS**: INI format `[playlist]`, `FileN`, `TitleN`, `LengthN` (-1 = unknown), in ascending N; `NumberOfEntries` is only used for validation.
   - **DPL** (read only): the first line is the DPL file identification header, followed by `N*file*<path>`, `N*title*<title>`, `N*duration2*<milliseconds>`.
4. De-duplication: within one list, local files with the same `realpath` are kept only once; links are de-duplicated by their literal URL. Local files that do not exist are **kept** (the row shows as pending retry, with the hover hint "File not found"), so that users can deal with path migrations themselves when a list was made on another machine.
5. Limits: 20 000 entries per list; file size limit 16 MB. OSD: "Opened list: <name> (N items)"; skipped items are counted separately as ", M skipped".

**Behavior details (save)**:

1. Disabled when the list is empty. `dialogs.saveFile({ title: "Save Playlist", fileName: "<name of the last opened list file, otherwise "Playlist">.m3u8" })`.
2. The format is chosen by the extension the user enters: `.m3u8` (UTF-8), `.m3u` (UTF-8 with BOM so that Windows players recognize it), `.pls`; any other extension or none gets `.m3u8` appended.
3. M3U format: first line `#EXTM3U`, each entry `#EXTINF:<whole seconds or -1>,<displayed title>` + path. PLS: `[playlist]`, `FileN/TitleN/LengthN`, `NumberOfEntries`, `Version=2`.
4. Paths: media in the list file's directory or a subdirectory of it are written as relative paths, otherwise as absolute paths; links are written as they are.
5. Write a temporary file, then rename; when the target exists, the system dialog has already confirmed the overwrite, so it is overwritten directly. OSD "Saved list: <file name>"; on failure "Cannot save the list: <reason>".

**Core dependencies**: the existing `dialogs.openFile`, `dialogs.saveFile`.

**Acceptance**:
- Open an m3u8 exported by VLC/foobar (with relative paths and `#EXTINF` titles): titles and order match.
- Chinese paths in a GBK-encoded m3u are parsed correctly.
- Save as `.pls` and open again: entries, titles and durations match.
- Enter `list` (no extension) when saving: `list.m3u8` is created.
- Round-trip test: save → open → save again; the two files have the same content.

**Notes**: ASX is not done; DPL is read only, not written.

### C21 Search the list

**Requirement**: type keywords at the bottom of the list to filter it live, showing only the rows whose title or file name matches.

**Entry point**: the bottom bar search box; the list context menu "Search" or Ctrl+S with list focus (focuses the search box and selects the existing text).

**Behavior details**:

1. The search box sits in the bottom button bar after SORT and before the total duration (UI-PL-09 spec: about 24px high, `#282828` background), takes the remaining width, minimum 60px; when the sidebar is too narrow, the total duration hides first. Placeholder text "Search".
2. Matching: the input is split on whitespace into words; a row matches only when all words appear in "displayed title + file name + link URL"; case-insensitive and full-width/half-width-insensitive (NFKC normalization). 150 ms debounce after input.
3. While filtering: the row numbers still show the original numbers; drag to reorder and move to top / up / down / bottom are disabled (reason "The order cannot change while searching"); Ctrl+A and Remove Unselected only act on visible rows; when the filter changes, hidden rows are deselected automatically. The total duration shows the sum of the visible rows, prefixed with "Filtered ".
4. In the search box: Enter plays the first matching row (the cursor row if something is selected); ↓ moves focus back to the list and selects the first matching row; Esc clears the box and gives focus back to the list. With no match, the list area shows a centered grey "No matches".
5. When the search box has focus, `panel.setInteraction({ focused: true })` is called so the bottom bar does not auto-hide; letters typed in the input box do not trigger list keys such as P and Q (reference §4.1: a focused control takes the keys).

**Core dependencies**: none (plugin Slint interface).

**Acceptance**: in 100 rows, typing "s01 1080" leaves only the rows that contain both words; Esc restores all; while filtering, Ctrl+A then Delete only deletes visible rows; typing "p" in the search box does not start playback.

**Notes**: for the look of the search box, see the bottom bar in [UI spec](ui-spec.md) §5.2 (about 24 high, #282828).

### C23 Show in Folder

**Requirement**: open the folder of the selected file in the system file manager (Finder on macOS, File Explorer on Windows) and select the file. The menu title is "Show in Folder", the same on every system.

**Entry point**: list context menu "Show in Folder", Ctrl+F with list focus.

**Behavior details**:

1. Call `plugin.system.reveal(paths)` (round 2 general capability, takes an array of paths) for the selected local file rows (at most 20); files in the same folder go in one call and are all highlighted, different folders in separate calls. Run by the player (reference §16.5): on macOS one Finder window selects all of them; on Windows one File Explorer window per folder; on other systems `unsupported`.
2. File no longer exists: show its folder (`plugin.system.openPath(dir)`); if the folder does not exist either, OSD "File not found: <name>".
3. Disabled when only link rows are selected (reason "Links have no local location"). With more than 20 selected, only the first 20 are handled, OSD "Only the first 20 files are shown".

**Core dependencies**: `plugin.system.reveal`, `plugin.system.openPath` (round 2 general capabilities).

**Acceptance**: select a file and press Ctrl+F: the file manager (Finder on macOS, File Explorer on Windows) opens its directory and highlights the file; run it on the row of a deleted file: the file manager opens its directory.

### Chapter core API request summary

| Interface (TS draft) | Reason |
| --- | --- |
| `plugin.drops.provide({ extensions: string[]; drop(e): unknown }): Disposable` (capability `ui.drop`; `"folder"` matches folders) | System drag and drop only reaches the host's native window, and the video area belongs to no plugin, so the host has to dispatch by extension (A15) |
| `Panel.onDrop(listener: (e: { paths; at /* panel coordinates */; modifiers }) => unknown): Disposable` | Drops on a panel need the drop point to insert at a given position; plugin Slint components do not receive system drops (A15) |
| `interface DropEvent { paths: string[]; at: { x; y }; modifiers: { alt; shift; cmd } }` | Event data for the two items above (`at` only in `Panel.onDrop`; ruling R-U2 removed `onDragOver`) |
| ~~`player.preload(source: string, options: { after: Media; signal?: AbortSignal }): Preload`~~ | Rejected by ruling R-P5 (measured same-format switches are below the 150 ms threshold); B26 is downgraded to "fast advance to the next item" and needs no new core API |

---

## Subtitles (`subtitles`)

**Location**: `src/plugins/subtitles/` (existing plugin)

### Scope

The subtitles plugin owns every "at hand" operation in the subtitle domain: track selection, display, style, sync, subtitle file reading and format conversion. This round adds: secondary subtitles, reload, jumping by subtitle line, cycling tracks, numeric sync input and sync shortcut changes, style shortcuts and new style fields (bold, shadow, background box, original ASS styles), and loading external ASS/SSA/VTT/SMI/MicroDVD files.

Boundaries:
- Subtitle **rendering** (libass, style properties, secondary subtitles, render size) belongs to the core. The plugin only reads and writes state through `plugin.subtitles`.
- Parsing and encoding conversion of subtitle **files** are done in the plugin with Node, not in the core. The core only accepts the bytes of a few more formats.
- Reading the line-by-line content of **embedded** subtitles needs the core (the host already has embedded subtitle extraction, `native_subtitle.rs`).
- "Jump by subtitle line" uses the existing `player.seek` and does not call playback's commands.
- Online subtitle search is another plugin (`subtitle-online`), and the two do not communicate: that plugin loads its subtitle track itself, and this plugin sees the new track through `subtitles.onChange` and treats it as an ordinary track (`owned: false`, cannot be removed).

The manifest needs the new capabilities `playback.read` (read position, frame rate, source path) and `playback.control` (`seek`).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| A08 | Secondary subtitles (two subtitles shown at once) | ✗ | Yes: `role` in `subtitles.select`, `state.secondary`, `select: "secondary"` in `load` (ruling R-T3: round 3a) |
| A09 | Reload subtitles (read the file again) | ◐ | No (tracks loaded by this plugin are uploaded again by the plugin) |
| B11 | Previous / next subtitle line, current subtitle start | ✗ | No (ruling R-T5 rejected `subtitles.cues`: the plugin parses itself; embedded tracks use `tools.run("ffmpeg")` and `SubtitleTrack.streamIndex`) |
| D02 | Cycle subtitle tracks | ✗ | No |
| D05 | Enter a sync value manually | ◐ | `offsetMs` range widened to ±600 000 (R-T8, provided in round 2) |
| D06 | Font size shortcuts | ◐ | No |
| D07 | Subtitle position shortcuts | ◐ | No |
| D09 | Bold | ✗ | Yes: style field `bold` |
| D11 | Shadow, background box | ✗ | Yes: style fields `shadow`, `box` (one number field = opacity, `null` for no box; R-T7) |
| D17 | External ASS subtitles (keep effects) | ◐ | Yes: `format` in `load`; style field `assOverride` |
| D18 | External formats such as ASS/VTT | ✗ | Yes: `format: "srt" \| "ass"` and `title` in `load`; `SubtitleTrack.codec/text` (VTT is converted to SRT in the plugin, R-T4) |

### Menus and shortcuts

The top-level "Subtitles ▸" uses 300–399 (`doc/product/menus.md`). Existing items are renumbered; `subtitle-online` takes 360–369.

| Menu path | order | Group | Default shortcut | ID |
| --- | --- | --- | --- | --- |
| Open ▸ Open Subtitle… | 113 | open | Alt+O (existing) | D17/D18 extend the formats |
| Open ▸ Add Secondary Subtitle… | 114 | open | — | A08 |
| Playback ▸ Seek ▸ Previous Dialogue | 217 | jump-subtitle | Home | B11 |
| Playback ▸ Seek ▸ Next Dialogue | 217 | jump-subtitle | End | B11 |
| Playback ▸ Seek ▸ Current Subtitle Start | 217 | jump-subtitle | Ctrl+Home | B11 |
| Subtitles ▸ Show Subtitles (checkbox) | 301 | domain | Alt+H (existing) | |
| Subtitles ▸ Subtitle Track ▸ Off / each track (radio, at most 8) | 302–310 | subtitle-track | — | Existing |
| Subtitles ▸ Subtitle Track ▸ Cycle | 311 | subtitle-cycle | Alt+L | D02 |
| Subtitles ▸ Secondary Subtitle ▸ Off / each track (radio, at most 8) | 312–320 | subtitle-second | — | A08 |
| Subtitles ▸ Secondary Subtitle ▸ Position ▸ Top / Above Main Subtitle (radio) | 321 | subtitle-second-pos | — | A08 |
| Subtitles ▸ Subtitle Style ▸ Bold (checkbox) | 324 | style-font | Alt+B | D09 |
| Subtitles ▸ Subtitle Style ▸ Size + / Size − | 325 / 326 | style-font | Alt+PageUp / Alt+PageDown | D06 |
| Subtitles ▸ Subtitle Style ▸ Move Up / Move Down | 330–331 | style-move | none (user ruling 2026-10-27: Alt+↑/↓ pan the video; bind them in Settings ▸ Keyboard Shortcuts) | D07 |
| Reset ▸ Subtitle Position (moved to Reset ▸ 2026-11-01, see menus.md) | 330 | reset-subtitles | Alt+Home | D07 |
| Subtitles ▸ Subtitle Style ▸ Shadow (checkbox) | 335 | style-box | — | D11 |
| Subtitles ▸ Subtitle Style ▸ Opaque Box (checkbox) | 336 | style-box | — | D11 |
| Subtitles ▸ Original ASS/SSA Styles (checkbox) | 337 | subtitle-render | — | D17 |
| Subtitles ▸ Sync ▸ Earlier / Later; Reset ▸ Subtitle Sync (moved to Reset ▸ 2026-11-01, see menus.md) | 339–340; 331 | subtitle-sync; reset-subtitles | `.` `,` `/` (now `>` `<` `/`; `>` `<` go to audio sync, see the conflict check) | |
| Subtitles ▸ Sync ▸ Adjust Sync… | 342 | subtitle-sync-input | — | D05 |
| (360–369 used by `subtitle-online`) | | | | |
| Subtitles ▸ Reload Subtitles | 370 | subtitle-tools | Ctrl+Alt+Y | A09 |
| Subtitles ▸ Refresh Subtitle Tracks | 371 | subtitle-tools | — (existing) | |
| Subtitles ▸ Subtitle Tuning… | 372 | subtitle-tools | — (existing) | |
| Subtitles ▸ Subtitle Settings… | 373 | subtitle-tools | Alt+F | D10 existing window, shortcut added |

Conflict check (against the current bindings in menus.md):
- Home / End / Ctrl+Home are not used by any other global command. While the playlist has focus (only after a single click on a row) the list itself uses Home / End (and ↑ ↓ PgUp PgDn) to move the selection, as list views do; Ctrl+Home (Move to Top) has no list key (coordinator ruling 2026-11-08), Ctrl+End stays Move to Bottom (panel-only). Esc, playing a row or a click on the video gives the keys back to the video.
- Alt+F is not used. Alt+P (enter subtitles manually) is out of scope this round and is not taken.
- **Subtitle sync keys changed**: on a US keyboard Shift+. = `>`, Shift+, = `<`, Shift+/ = `?`. Audio sync uses `>` `<` `?` (G09 in the audio chapter), so subtitle sync changes to `.` earlier, `,` later, `/` reset, and `>` `<` go to audio. `.` and `,` have no other bindings now. The core does not migrate old user overlay bindings (there is no rebinding interface yet).

### A08 Secondary subtitles (two subtitles shown at once)

> **Schedule (round 2 ruling)**: secondary subtitles as a whole move to round 3a (R-T3, `select: "secondary"` in `load`, `secondaryPosition`). The text below follows the 3a interface; round 2 does not show "Open ▸ Add Secondary Subtitle…" or "Subtitles ▸ Secondary Subtitle ▸".

**Requirement**: show two subtitle tracks at once: the main subtitle at the bottom, the secondary subtitle at the top or right above the main subtitle. The secondary subtitle can be any existing text track (embedded or external), or added from a file.

**Entry points**:
- `Open ▸ Add Secondary Subtitle…`: pick a file → loaded as an external track and set as the secondary subtitle (the main subtitle does not change).
- `Subtitles ▸ Secondary Subtitle ▸ Off / <track name>` (radio, at most 8 tracks; the rest are chosen in Subtitle Tuning).
- `Subtitles ▸ Secondary Subtitle ▸ Position ▸ Top (default) / Above Main Subtitle`.
- The Subtitle Tuning window gains a "Secondary Subtitle" drop-down row (subtitles off + each track).

**Behavior details**:
1. The secondary subtitle cannot be the same track as the main subtitle: in the secondary subtitle menu, the item of the current main subtitle is disabled with the reason "Already the primary subtitle", and the other way round. When the user picks the current secondary track in the main subtitle menu, the secondary subtitle is set to "Off" automatically, OSD "Secondary subtitle off (it is now the main subtitle)".
2. Only text tracks are accepted (`track.text === true`). Image tracks (PGS, VobSub) are disabled in the secondary subtitle menu with the reason "Only text subtitles can be secondary".
3. "Show Subtitles" Alt+H only toggles the main subtitle; the secondary subtitle keeps its setting.
4. After the source changes, the secondary subtitle resets to "Off" (the core state is cleared with the source).
5. Secondary subtitle style: font, color, outline, bold and shadow are the same as the main subtitle; the font size is the same as the main subtitle's (mpv has no separate size for the secondary subtitle); the position is set by the "Position" radio. Sync (`offsetMs`) applies to both. Reason: mpv's secondary subtitle has only a few independent properties (`secondary-sub-pos`, `secondary-sub-delay`, `secondary-sub-ass-override`), so there is no independent style this round.
6. Adding a secondary subtitle uses the same file reading as D18 (format, encoding, size checks). If the upload succeeds but setting it as the secondary subtitle fails, the track stays in the list, OSD "Subtitle loaded, but it cannot be the secondary subtitle".
7. OSD: `Secondary subtitle: <track name>`, `Secondary subtitle: off`.
8. With no media or no text track, the secondary subtitle submenu has only "Off" (selected); `Add Secondary Subtitle…` is disabled with the reason "No media to play".

**Interface**: the Subtitle Tuning window gets a "Secondary Subtitle" drop-down in its second row (the same width as the track drop-down, no button on the right). The window is still 301 wide, total height ≤ 280 (UI-TOOL-08).

**Persistence**: `settings.json` stores `secondaryPosition` (`"top" | "aboveMain"`). Which track is selected is not saved across sources.

**Core dependencies**:
- Existing: `subtitles.state/onChange/refresh/select/load`, `dialogs.openFile`.
- [New core API]
  ```ts
  interface SubtitlesState { readonly secondary: SubtitleTrack | null;
                             readonly secondaryConfirmation: "waiting" | "applied" | "uncertain"; }
  subtitles.selectSecondary(track: SubtitleTrack | null, opts?: { ifCurrent?: SubtitlesState }): Promise<SubtitlesState>
  subtitles.load(bytes, opts?: { …; select?: "primary" | "secondary" | "none" })   // default "primary", same as now
  type SubtitleStyle = { …; secondaryPosition?: "top" | "aboveMain" }
  ```
  Reason: mpv's `secondary-sid` can only be set by the host, and plugins cannot compose two tracks between them. The host has to confirm that its libmpv supports `secondary-sub-pos` (mpv ≥ 0.38); otherwise "Above Main Subtitle" falls back to top, explained in `styleReason`.

**Acceptance**:
- With two embedded text tracks, main = 1 and secondary = 2: two lines appear on the picture at once; `state.secondary.id` equals track 2.
- The menu item that would make the secondary subtitle the same track as the main subtitle is disabled; after changing the main subtitle to the current secondary track, the secondary subtitle turns off automatically with an OSD.
- Image tracks are disabled in the secondary subtitle menu.
- After changing files, `state.secondary === null` and the menu is back to "Off".
- `Add Secondary Subtitle…` with an SRT: the main subtitle does not change, and the secondary subtitle is the new track.

**Notes**: this round does not turn on secondary subtitles automatically by language.

### A09 Reload subtitles (read the file again)

**Current state**: there is only "Refresh Subtitle Tracks", which re-reads the host's track list but not the file.

**Requirement**: after the user edits a subtitle file in an external editor, pressing Ctrl+Alt+Y makes the player read the file on disk again and show the new content immediately, keeping the current selection and sync.

**Entry point**: `Subtitles ▸ Reload Subtitles` (Ctrl+Alt+Y).

**Behavior details**:
1. Applies to: the external tracks **loaded by this plugin** (`owned`) among the current main and secondary subtitles. The plugin remembers the origin of each track it loaded: `{ trackId, path, format, encoding, title, role }`, in memory only.
2. For each track: read the file again (the D18 reading flow) → `load(bytes, { select: <original role> })` → on success `remove(<old track>)`. The new track takes the old track's place in the menu (the menu is rebuilt by title).
3. When the file content has not changed (same SHA-256), nothing is uploaded again, OSD "Subtitle unchanged".
4. The file has been deleted or cannot be read: the old track is kept, OSD "Cannot reload: subtitle file <file name> not found".
5. Neither the current main nor the secondary subtitle was loaded by this plugin: for embedded tracks OSD "Embedded subtitles need no reload"; for external tracks loaded by other plugins OSD "This subtitle was not loaded by the subtitles plugin and cannot be reloaded".
6. Subtitle style and sync do not change (they are the plugin's style layer and do not belong to the track).
7. While reloading, the menu shows busy (the existing serial queue is used); it is cancelled when the source changes.
8. Setting "Reload automatically when the subtitle file changes" (off by default): when on, loaded files are watched with `fs.watch`, and 500 ms after changes stop, the same flow runs automatically, OSD "Subtitle reloaded automatically".
9. Success OSD: `Reloaded subtitle <file name>`.

**Persistence**: `autoReload: boolean` in `settings.json`.

**Core dependencies**: the existing `subtitles.load`, `remove`, `select`. No new core API needed (the host's `sub-reload` only makes sense for files mpv opened itself, while our external tracks are all uploaded temporary files).

**Acceptance**:
- Load a.srt, change the text of line 1 externally, press Ctrl+Alt+Y: the picture shows the new text; in `state.tracks` the old track is gone and the new track is selected.
- The sync value is the same before and after reload.
- Reload after deleting the file: the old track is still there, and the OSD wording is correct.
- When the content has not changed, `host.sent("subtitles.load")` has no new entry.
- With automatic reload on, rewriting the file takes effect within 1 second.

### B11 Previous / next subtitle line

**Requirement**: Home jumps to the start of the previous subtitle line, End to the start of the next subtitle line, and Ctrl+Home back to the start of the current subtitle line (repeat). For language learning and finding lines.

**Entry points**: `Playback ▸ Seek ▸ Previous Dialogue` (Home), `Next Dialogue` (End), `Current Subtitle Start` (Ctrl+Home). They go in playback's Seek submenu (order 217, a group of their own, with a separator), because they are seek operations; the commands still belong to the subtitles plugin.

**Behavior details**:
1. Based on the line timetable of the current **main subtitle** track, which the plugin gets itself (ruling R-T5: the core does not provide `cues`): for external tracks loaded by this plugin, parse the uploaded bytes (SRT or ASS); for embedded text tracks, extract SRT with `plugin.tools.run("ffmpeg", ["-v","error","-nostdin","-i",<source>,"-map","0:<streamIndex>","-f","srt","pipe:1"])` and parse it (embedded ASS tracks can be extracted the same way). Display time = line time + sync value (`style.offsetMs`, positive = later).
2. The timetable is cached per track: read on the first key press and reused for the same track afterwards; invalidated by a track change, source change or reload. While ffmpeg is reading, the menu shows busy and key presses are ignored; the timeout is about 1 second per 100 MB of source size, at least 15 seconds; after 0.5 seconds, OSD "Reading subtitles…". Without ffmpeg, OSD "ffmpeg is needed to read embedded subtitles"; on timeout "Reading embedded subtitles timed out".
3. Previous line: the last line whose display start < current position − 0.5 seconds. This way pressing Home within 0.5 seconds of the start of a line goes to the previous line instead of repeating the same one.
4. Next line: the first line whose display start > current position + 0.05 seconds.
5. Current subtitle start: the start of the line containing the current position (start ≤ position < end); when not inside any line, the most recent line that has ended.
6. The jump uses `player.seek(t)`, with `t` = line start − 0.05 seconds (so decoding does not land on a frame in the second half of the line), minimum 0.
7. No line to jump to: OSD "No previous dialogue" / "No next dialogue".
8. No subtitle selected, the main subtitle is an image track, or the timetable is empty: the command is disabled with the reason "No text subtitles available". It is also disabled in these cases (the plugin cannot get a timetable): an embedded track's `streamIndex` is `null` (not read by lavf), network sources, external tracks loaded by other plugins (such as tracks loaded by subtitle-online).
9. No media, or `can.seek` is false (live stream): disabled with the reason "The current media cannot seek".
10. Held down: `repeat: true`, jumping line after line. Jumps run one at a time, each counted from where the previous jump landed (the position read again after the jump completes), so it does not stop on the same line; when keys come faster than jumps, at most one is queued and the rest are dropped.
11. OSD: `Seek: previous dialogue 00:12:03.450`, `Seek: next dialogue …`, `Seek: start of current dialogue …`.
12. When `seek` fails with `stale` (someone changed the source meanwhile), there is no retry and no message.

**Core dependencies**:
- Existing: `player.seek`, `player.refresh`, `subtitles.state.style.offsetMs`; `SubtitleTrack.text` and `streamIndex` (R-T6) and `plugin.tools.run`, added in round 2.
- No new core interface (ruling R-T5: `subtitles.cues` rejected; the alternative is ffmpeg extraction in the plugin). The manifest adds `playback.read` and `playback.control`.

**Acceptance**:
- With 3 lines (10–12 s, 20–22 s, 30–32 s) and sync 0: at 25 s, End → 30 s; Home → 20 s; at 20.3 s, Home → 10 s.
- An embedded text track (local mkv) calls ffmpeg once on the first key press, and not again for the same track afterwards; network sources are disabled.
- With sync +1000 ms, End jumps from 25 s to 31 s.
- With no subtitles, the command is disabled and shows the reason; disabled on live streams.
- Holding End jumps line after line without stopping on the same line.

### D02 Cycle subtitle tracks

**Requirement**: Alt+L cycles through the subtitle tracks: track 1 → track 2 → … → track N → off → track 1.

**Entry point**: `Subtitles ▸ Subtitle Track ▸ Cycle` (Alt+L).

**Behavior details**:
1. The order follows the list order of `state.tracks`, including all tracks (not limited to the 8 in the menu), with "off" after the last one.
2. The track that is currently the secondary subtitle is skipped.
3. With only one track, it toggles between "track 1 / off"; with no tracks, it is disabled with the reason "No subtitle tracks".
4. Fast repeated presses: the last request wins (requests not yet started that have been superseded are dropped from the serial queue), and the OSD shows the final result.
5. OSD: `Subtitles: <track name> (2/5)`, `Subtitles: off`.
6. Switch failures (`stale`, etc.) use the existing error wording.

**Core dependencies**: the existing `subtitles.select`.

**Acceptance**: with 3 tracks, press Alt+L 4 times: the selection goes 2, 3, off, 1 (starting from 1); the secondary track is skipped; disabled with no tracks.

### D05 Enter a sync value manually

**Current state**: sync can only move in 0.5-second steps (menu "Earlier / Later / Reset", slower / faster in the tuning window); no value can be entered; the step is fixed.

**Requirement**:
1. Enter any sync value (milliseconds), applied immediately.
2. Set the sync step.

**Entry points**: `Subtitles ▸ Sync ▸ Adjust Sync…` (opens a small window); clicking the sync number in the Subtitle Tuning window opens the same window; the Subtitle Settings window gains a "Sync step" row.

**Behavior details**:
1. The input box accepts whole milliseconds with an optional sign; positive values make subtitles later, negative values earlier (as now; the hint text says so). Range −600 000 to 600 000 (widened in core round 2, R-T8).
2. When the window opens, the current value is filled in. Enter or "Apply" applies it; out of range, the input box keeps the old value and the status line shows "The sync value must be between −600000 and 600000 ms".
3. When the input box has focus, the up and down arrow keys add or subtract one step (global shortcuts are not triggered; the focused control takes the keys).
4. After applying, OSD `Subtitle sync: 1.25 s later` (the existing `syncText`); the window stays open.
5. No subtitle selected, or the track does not support styles: the input box and button are disabled and show the reason (the existing `styleBlocked`).
6. Sync step: 0.05 / 0.1 / 0.25 / 0.5 / 1 / 5 seconds, default 0.5 seconds. The "Earlier / Later" menu titles change accordingly to "Earlier 0.1 s" etc. (title change → the two commands are rebuilt in a child scope, `order` unchanged).

**Interface**: the "Adjust Sync" window is 350×121: one row with the right-aligned label "Sync value (ms):" 102px, input box 86×21, "Apply" 75×23, spacing 7px, vertically centered; one line of status text (11px) below.

**Persistence**: `syncStepMs` in `settings.json`. The sync value itself is not persisted (following the existing style layer behavior).

**Core dependencies**:
- Existing: `subtitles.setStyle({ offsetMs })`.
- [Provided in core round 2] `SubtitleStyle.offsetMs` range −600 000–600 000 (R-T8). Reason: intros of different releases of a source commonly differ by 1–3 minutes, so ±10 seconds is not enough; mpv's `sub-delay` itself has no such limit.

**Acceptance**: enter 1250 and apply → `style.offsetMs === 1250`, with the correct OSD; entering abc or 99999999 has no effect and shows a status message; after changing the step to 0.1 seconds, each press of `.` is −100 ms and the menu title becomes "Earlier 0.1 s"; pressing `>` no longer changes the subtitle sync (it is now used by audio sync).

### D06 Font size shortcuts

**Current state**: the Subtitle Tuning window has font size ±, with no shortcuts and no menu items.

**Requirement**: Alt+PageUp font size +2, Alt+PageDown font size −2, range 16–72 (the host style range), default 36.

**Entry point**: `Subtitles ▸ Subtitle Style ▸ Size + / Size −`.

**Behavior details**:
1. Shares one `styleChange` with the tuning window buttons (submits merged over 100 ms), so both have the same state.
2. `repeat: true`. At the upper or lower limit it does not change any more, and the OSD still shows the current value.
3. OSD: `Subtitle size: larger (38)`, `Subtitle size: smaller (34)`.
4. When the style cannot be adjusted, the OSD shows the reason (such as "Select a subtitle track first.").

**Core dependencies**: the existing `subtitles.setStyle`.

**Acceptance**: after `host.press("Alt+PageUp")`, `style.fontSize` is 38; pressing again at 72 stays at 72; with no subtitle selected, the OSD is "Select a subtitle track first.".

### D07 Subtitle position shortcuts

**Current state**: the Subtitle Tuning window has position up / down, with no shortcuts.

**Requirement**: Move Up and Move Down (no default key, user ruling 2026-10-27: Alt+↑/↓ pan the video) move the subtitles 1% per step (bottom margin `marginPercent`), range 2–15, default 5; Alt+Home resets the position.

**Entry point**: `Subtitles ▸ Subtitle Style ▸ Move Up / Move Down`, and `Reset ▸ Subtitle Position` (Subtitle Style ▸ Reset Position until 2026-11-01).

**Behavior details**:
1. Shares state with the tuning window; `repeat: true`.
2. Reset: `marginPercent = 5`.
3. OSD: `Subtitle position: up (6%)`, `Subtitle position: down (4%)`, `Subtitle position: default (5%)`.

**Core dependencies**: the existing `subtitles.setStyle`.

**Acceptance**: run Move Up 3 times → `marginPercent` 8; Alt+Home → 5, with the correct OSD wording.

**Notes**: there is no "top / bottom margin ±"; the subtitle position already is the bottom margin.

### D09 Bold

**Requirement**: a subtitle bold toggle, off by default.

**Entry points**: `Subtitles ▸ Subtitle Style ▸ Bold` (checkbox, Alt+B); a "Bold" checkbox below the "Font" row of the Subtitle Settings window.

**Behavior details**:
1. Checked state = `bold` of the style currently in effect.
2. OSD: `Bold subtitles` / `Bold subtitles off`.
3. When the bundled font has no bold glyphs, the renderer synthesizes bold (libass default behavior), without an error.
4. When the style cannot be adjusted, the OSD shows the reason.

**Persistence**: new `style.json`: saves all style fields except `offsetMs` (font, font size, color, outline, margin, bold, shadow, background box, original ASS styles, secondary subtitle position), restored with `setStyle` the first time a track with adjustable style is selected after the plugin starts. Reason: if the new toggles were not remembered, users would have to set them again on every launch. A corrupt file or out-of-range fields fall back to the defaults.

**Core dependencies**: [New core API] `SubtitleStyle.bold?: boolean` (mpv `sub-bold`).

**Acceptance**: after Alt+B, `style.bold === true`, the subtitles on the picture become bold and the menu item is checked; still bold after restarting the player.

### D11 Shadow, background box

**Requirement**:
1. Shadow: a toggle + shadow distance 0–4 (default 0 = off; 2 by default when turned on), color fixed at black 50% opacity.
2. Background box (opaque box): a toggle, off by default; opacity 0–100%, default 50%, step 5%; box color black. When on, the outline setting has no effect (the box replaces the outline), and the interface says so.

**Entry points**: `Subtitles ▸ Subtitle Style ▸ Shadow` (checkbox), `Opaque Box` (checkbox); the Subtitle Settings window gains a "Shadow" slider row (0–4 + default) and a "Background box" checkbox + "Opacity" slider row (+ default).

**Behavior details**:
1. Checking "Shadow" in the menu sets the distance to the last non-zero value (2 the first time); unchecking sets it to 0.
2. OSD: `Subtitle shadow: on (2)` / `Subtitle shadow: off`; `Opaque subtitle box: on` / `off`.
3. Slider drags use the existing 100 ms merged submit, and the model value is pushed back on release.

**Persistence**: `style.json` (see D09).

**Core dependencies**: [Provided in core round 2, rewritten by R-T7] `SubtitleStyle.shadow?: number` (0–4, mpv `sub-shadow-offset` scaled to 1080p), `SubtitleStyle.box?: number | null` (one field: opacity 0–100, `null` for no box; the host uses `background-box` + `sub-back-color`). With the toggle and opacity merged into one field, the plugin itself remembers the opacity before the box was turned off (`boxOpacity` in `style.json`) and the last shadow distance (`shadowLast`). The host's background box and shadow share `sub-shadow-offset`, so **the shadow has no effect while the box is on**; the shadow slider in the settings window turns grey accordingly, and the checkbox reads "Background box (replaces outline and shadow)".

**Acceptance**: with the background box on, a semi-transparent black box appears under the subtitles; at opacity 100 the box is fully black; at shadow 4 the shadow is clearly visible; the menu and window states agree.

### D17 External ASS subtitles (keep effects)

**Current state**: embedded ASS is rendered by mpv, but the host fixes `sub-ass-override=force`, so the ASS file's own fonts, colors, positioning and effects are overridden by the plugin style; external files only accept SRT.

**Requirement**:
1. External `.ass` / `.ssa` files can be loaded.
2. "Original ASS/SSA Styles" toggle (on by default): when on, ASS tracks are shown with the file's own styles and effects (positioning, movement, karaoke, fades); the plugin style only applies font size scaling and the sync value to them. When off, the plugin style is forced as now.

**Entry points**: `Open ▸ Open Subtitle…` (Alt+O, ass/ssa added to the file filter); `Subtitles ▸ Original ASS/SSA Styles` (checkbox); the "ASS style" drop-down in the Subtitle Settings window (Keep original styles / Use subtitle settings).

**Behavior details**:
1. The reading flow is the same as D18 (encoding, size). The `[Fonts]` and `[Graphics]` sections of an ASS file (embedded fonts and images) are removed before upload, because the host only uses bundled fonts (`sub-font-provider=none`); this data is useless and easily exceeds the 2 MiB limit. After removal the status line says "Fonts embedded in the subtitle were ignored".
2. When a font named by the ASS file is not in the host's font list, the host falls back to the default font (libass behavior), without an error; for ASS tracks the "Font" row of the Subtitle Settings window shows "As in the subtitle file (missing fonts use the default font)".
3. With original styles on, for ASS tracks: the font size uses "subtitle font size ÷ 36" as the scale factor; color, outline, bold, shadow, background box and position have no effect (these controls stay available for ASS tracks but carry the side note "No effect while original ASS styles are on"). SRT/VTT tracks use all styles as usual.
4. Switching the toggle takes effect immediately, OSD: `ASS/SSA animated subtitles: on` / `off`.
5. ASS with syntax errors: the host rejects it (`invalid`), OSD "The player rejected this subtitle (the file is too large or its content is invalid)." (existing wording).

**Persistence**: `assOverride` in `style.json`.

**Core dependencies**:
- [Provided in core round 2, R-T4] `subtitles.load(bytes, { format: "ass" })` (SSA also uses `"ass"`; the host does bounded validation of ASS: UTF-8, ≤ 2 MiB, has `[Script Info]` and `[Events]`, ≤ 20 000 Dialogue lines). The plugin converts UTF-16 and other encodings to UTF-8 before upload. External ASS tracks cannot be included in "with subtitles" screenshots (host limitation).
- [New core API] `SubtitleStyle.assOverride?: "keep" | "force"` (default keep; mpv `sub-ass-override=scale` / `force`). Reason: the override policy is a global mpv property that only the host can set; the current force is hard-coded in the host.

**Acceptance**: load an ASS with `\pos` and `\fad`: with the toggle on, the subtitle appears at the given position and fades in; when off it becomes white text at the bottom; embedded ASS is controlled by the toggle too; a 3 MiB file with a [Fonts] section can be loaded.

### D18 External subtitle formats: ASS, VTT, etc.

**Requirement**: "Open Subtitle" accepts the common text subtitle formats and common encodings, without requiring the user to convert to UTF-8 SRT first.

| Format | Extension | Handling |
| --- | --- | --- |
| SubRip | .srt | Uploaded as is (existing) |
| ASS / SSA | .ass, .ssa | Uploaded with `format: "ass"` (D17; SSA also uses `"ass"`) |
| WebVTT | .vtt | The plugin converts to SRT (ruling R-T4: no `"vtt"`): remove `STYLE`, `REGION` and `NOTE` blocks and cue settings, remove `<c>`, `<v>`, `<lang>`, `<ruby>` and karaoke timestamp tags, keep `<b>` `<i>` `<u>`; subtitles and subtitle-online each carry a copy of the same conversion code |
| SAMI | .smi, .sami | The plugin converts to SRT (split into lines by `<SYNC Start=>`; when a file contains several language classes, one track per language, with the language in the title) |
| MicroDVD | .sub (text, `{start}{end}` lines) | The plugin converts to SRT using the video frame rate; when the frame rate is unknown, use the `{1}{1}23.976` declaration on the file's first line, otherwise 23.976 with a message |
| Plain text timeline | .txt | Not supported, message "Unknown subtitle format" |
| Image subtitles | .sup, .idx/.sub (VobSub) | Not supported this round, message "Image subtitle files are not supported yet" |

**Behavior details**:
1. The format is recognized by content; the extension is only a hint (`.sub` is first checked for MicroDVD text; binary means VobSub).
2. Encoding: a BOM (UTF-8 / UTF-16LE / UTF-16BE) comes first; otherwise try strict UTF-8; on failure decode with the "Default encoding" setting (default GB18030; Big5, Shift_JIS, EUC-KR, Windows-1252, Windows-1251, ISO-8859-1 are available), and convert everything to UTF-8 before upload. Node's built-in ICU supports these encodings. When the share of replacement characters after decoding is > 1%, OSD "The subtitle encoding may be wrong; you can change the default encoding in Subtitle Settings".
3. Size: the file read is ≤ 8 MiB (existing); after conversion and removal of ASS embedded fonts ≤ 2 MiB (the host limit), otherwise "The subtitle file is larger than 2 MiB".
4. The open dialog's file type hint is "Subtitle files (*.srt *.ass *.ssa *.vtt *.smi *.sub)" (if the system dialog does not support filters, nothing is restricted and the content decides).
5. Load success OSD: `Opened subtitle: <file name>`; for multi-language SAMI `Opened subtitle: <file name> (2 tracks)`.
6. Track titles use the file name (through `title` in `load`), no longer the host's temporary name.
7. The settings window gains a "Default encoding" drop-down.

**Persistence**: `fallbackEncoding` in `settings.json`.

**Core dependencies**:
- [Provided in core round 2, rewritten by R-T4] `format` (`"srt" | "ass"`) and `title` in `subtitles.load`. VTT, SAMI and MicroDVD are converted to SRT in the plugin; no core formats are added.
- [New core API] `SubtitleTrack.codec: string | null`, `SubtitleTrack.text: boolean` (the host already reads `codec` from mpv and only needs to expose it). Used to tell text tracks from image tracks (needed by A08 and B11).

**Acceptance**: a GB18030-encoded SRT, a UTF-16LE ASS, a VTT, an SMI with Chinese and English classes and a MicroDVD file each load and show the correct text; VobSub gives a clear message; the new track's title in `state.tracks` is the file name.

### Chapter core API request summary

> **Ruling result (round 2)**: `format` (`"srt" | "ass"`), `title` and `select: "primary" | "none"` in `load`, `SubtitleTrack.codec/text/streamIndex`, the style fields `bold`, `shadow`, `box` (a single number field) and `assOverride`, and `offsetMs` ±600 000 are provided; `subtitles.cues` is rejected (replaced by ffmpeg in the plugin); secondary subtitles (`role` in `select`, `select: "secondary"`, `secondaryPosition`) are scheduled for round 3a; no `"vtt"` or `"ssa"` formats are added. The table below keeps the original requests.

| Interface | Reason | Used by |
| --- | --- | --- |
| `subtitles.selectSecondary(track: SubtitleTrack \| null, opts?: { ifCurrent?: SubtitlesState }): Promise<SubtitlesState>`; `SubtitlesState.secondary: SubtitleTrack \| null`, `secondaryConfirmation` | mpv `secondary-sid` can only be set by the host | A08 |
| `subtitles.load(bytes: Uint8Array, opts?: { format?: "srt" \| "ass" \| "ssa" \| "vtt"; title?: string; select?: "primary" \| "secondary" \| "none"; ifCurrent?; signal? }): Promise<SubtitleTrack>` | The host only validates SRT; the track title and whether it is selected need the host to decide | A08, A09, D17, D18 |
| `subtitles.cues(track: SubtitleTrack, opts?: { signal?: AbortSignal }): Promise<SubtitleCue[]>`; `SubtitleCue { start; end; text }` | Only the host can reliably extract the line-by-line content of embedded subtitles (the host already has an extraction implementation) | B11 |
| `SubtitleTrack.codec: string \| null`, `SubtitleTrack.text: boolean` | Tell text and image tracks apart; the host already reads codec and only needs to expose it | A08, B11, D18 |
| New optional `SubtitleStyle` fields: `bold?: boolean`, `shadow?: number` (0–4), `box?: boolean`, `boxOpacity?: number` (0–100), `assOverride?: "keep" \| "force"`, `secondaryPosition?: "top" \| "aboveMain"` | mpv style properties can only be written by the host; optional fields keep existing calls compatible | D09, D11, D17, A08 |
| `SubtitleStyle.offsetMs` range widened to ±600 000 (optional) | ±10 seconds is not enough to correct intro differences; mpv has no such limit | D05 |

---

## Picture adjustment (`video`)

**Location**: `src/plugins/video/` (existing plugin, manifest id `vestra.video`)

**Scope**

This plugin owns every feature in the "Video" menu that changes how the picture looks: color, image processing filters, geometric transforms (flip, rotate, crop/stretch), display mode (aspect ratio, framing zoom and pan), deinterlacing, and the GPU-rendering-dependent HDR tone mapping, pixel shaders and motion interpolation. Today it has only two processing nodes, eq (brightness / contrast / saturation / gamma) and Gaussian blur, and a single-page window 301 wide. This round adds 13 items, E02, E04–E06, E08–E13 and E16–E18, and the window gets four tabs. Capture (`capture`), subtitles (`subtitles`) and playback info (`info`) do not belong to this plugin.

The boundary with the core falls into three classes, and each feature states which class it belongs to:

1. **Pixel processing** (hue, sharpen, denoise, deblock, deband, flip, rotate, deinterlace, crop/stretch, frame doubling): continues to go through this plugin's own processing chain `plugin.video.filter("picture")`; the core only needs to add node types to `VideoNode` (`VideoNode` in `src/host/src/playback/effects.rs` currently has only `Eq` and `Gblur`). These effects follow the source and are reapplied by the SDK after a source change; screenshots "with effects" and offline exports include them as well (comment in `effects.rs`: the same description produces both the live picture and the offline picture), so every new node has to work on both paths.
2. **Display mode** (aspect ratio, display mode, zoom and pan): does not change pixels, only how the picture is placed in the video area, matching mpv's vo-level properties (`video-aspect-override`, `keepaspect`, `panscan`, `video-zoom`, `video-scale-x/y`, `video-pan-x/y`). The processing chain cannot express these, so the core needs a new separate interface `video.view()`; for correct results it also needs the host behavior change **C-SIZE: render size follows the video area** (see E11). Screenshots do not include these effects.
3. **GPU render options** (HDR tone mapping, GLSL shaders, motion interpolation): mpv's `tone-mapping`, `glsl-shaders` and `interpolation` only work in the GPU renderer. In the render round (2026-10-21) the host completed **C-GPU**: macOS uses libmpv's OpenGL rendering by default and provides `plugin.video.render()` (reference §8.2, single owner); software rendering is still the fallback (headless mode, `VESTRA_SOFTWARE_RENDER=1`, Windows and Linux, OpenGL initialization failure), and then `render()` rejects with `unsupported`. This plugin enables or disables these three items (E16–E18) according to the state of `render()`. They are not in the processing chain, and screenshots and exports do not include them.

This plugin does not communicate with other plugins. All the external data it needs is read from the core: playback state (`player.state`) and video parameters (`video.width/height/fps` of `player.stats()`).

Manifest capabilities added to the existing ones: `ui.dialog` (E17 picking shader files).

**Feature overview**

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| E02 | Hue ±1% | ✗ | Yes: `hue` node |
| E04 | Sharpen, edge sharpen | ✗ | Yes: `sharpen` node (small / large, ruling R-V2, no `cas`) |
| E05 | Denoise (3D / temporal / progressive) | ✗ | Yes: `denoise` node (hqdn3d; the three kinds are presets in the plugin, ruling R-V3) |
| E06 | Deblock / deband | ✗ | Yes: `deblock`, `deband` nodes |
| E08 | Vertical / horizontal flip | ✗ | Yes: `transform` node (ruling R-V5) |
| E09 | Rotate 90 / 180 / 270 | ✗ | Yes: `transform` node (ruling R-V5) |
| E10 | Deinterlace (auto / on / off) | ✗ | Yes: `deinterlace` node (fixed BWDIF, ruling R-V6) |
| E11 | Aspect ratio and display mode | ✗ | Yes: `video.view()`; C-SIZE (round 4a, ruling R-V9) |
| E12 | Stretch (add borders) / crop to a given ratio | ✗ | Yes: `reframe` node (ruling R-V7) |
| E13 | Picture zoom and pan (framing) | ✗ | Yes: `video.view()` (round 4a, ruling R-V9); numpad binding names (provided in round 2) |
| E16 | Automatic HDR correction / tone mapping | ✗ | Yes: C-GPU + `video.render()`, `stats().video.transfer` (provided in the render round) |
| E17 | Pixel shaders (custom GLSL) | ✗ | Yes: C-GPU + `video.render()` (provided in the render round) |
| E18 | Frame doubling / motion interpolation | ✗ | Frame doubling: `frameBlend` node (round 4a, ruling R-V8); motion interpolation: C-GPU + `video.render()` (provided in the render round) |

**Menus and shortcuts**

Menu order: Pixel Shaders → Deinterlace → Aspect → Crop/Stretch → Framing → Image Processing → Rotation → Snapshot → Picture Adjustments and Reset → Brightness…Hue. "Aspect" and "Framing" go under "Video ▸" by the domain rule (interaction-requirements.md §1: video parameters all belong to the video domain).

**Changes to existing entry points**: `Picture Adjustments…`, `Reset` and the six brightness / contrast / saturation items move from order 401–408 to 440–447, freeing 401–409 for the new submenus, so `Snapshot ▸` (capture, 420) ends up in the middle. `Gaussian Blur` (Ctrl+B) moves from the first level of "Video" to "Video ▸ Image Processing ▸"; the command id and binding do not change. Items inside a submenu all use the same order (a submenu's position is its smallest order) and are ordered by declaration (reference §4: equal order follows the declaration order of the same plugin). After the change, `doc/product/menus.md` has to be updated to match.

| Menu path | order | group | Default shortcut | ID |
| --- | --- | --- | --- | --- |
| Video ▸ Pixel Shaders ▸ Use Shaders (checkbox) · Shader List… · Open Shader Folder · Reload Shaders · —— · HDR Tone Mapping ▸ (radio: Auto / Off / BT.2390 / Hable / Mobius / Reinhard / Clip · —— · Dynamic Peak Detection (checkbox)) · HDR Settings… | 402 | video-source (the two HDR items video-hdr, peak detection video-hdr-peak) | Use Shaders Shift+P | E16, E17 |
| Video ▸ Deinterlace ▸ Cycle · Auto · Always · Never (radio) · —— · Output ▸ By Frame / By Field (Double Rate) (radio) | 403 | video-source | Cycle Ctrl+I | E10 |
| Video ▸ Aspect ▸ Keep Aspect Ratio · Ignore Aspect Ratio · Fill Window (Crop Excess) (radio) · —— · Cycle Aspect Ratio · Default (Source) · 4:3 · 16:9 · 16:10 · 1.85:1 · 2.35:1 · Custom (x:y) (radio) · Custom Aspect Ratio… | 404 | video-display | Cycle Aspect Ratio Shift+F6 (Ctrl+F6 is taken by macOS) | E11 |
| Video ▸ Crop/Stretch ▸ None · Stretch to 4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1 · Crop to 4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1 (radio) · —— · Move Left · Move Right · Move Up · Move Down (the reset: Reset ▸ Crop Position, 442, reset-video) | 405 | video-display | None (not Ctrl+Q, see below) | E12 |
| Video ▸ Framing ▸ Zoom In · Zoom Out · —— · Pan → · Pan ← · Pan ↑ · Pan ↓ · Center Frame (the reset: Reset ▸ Framing, 441, reset-video) | 406 | video-display | See the table below | E13 |
| Video ▸ Image Processing ▸ Flip Vertically · Flip Horizontally · —— · Gaussian Blur · Sharpen · Edge Sharpen · —— · Deblock · Deband · 3D Denoise · Gradual Denoise · Temporal Denoise · —— · Frame Doubling · Motion Interpolation (all checkboxes) | 408 | video-effects | Ctrl+P · Ctrl+Z · Ctrl+B · Ctrl+R · — · Ctrl+H · — · Ctrl+N · — · Ctrl+M | E04–E06, E08, E18 |
| Video ▸ Rotation ▸ Cycle · No Rotation · 90° · 180° · 270° (radio) | 409 | video-effects | Cycle Alt+K | E09 |
| Video ▸ Snapshot ▸ … (capture, unchanged) | 420 | — | — | — |
| Video ▸ Picture Adjustments… | 440 | video-color | | Existing |
| Reset ▸ Picture (moved to Reset ▸ 2026-11-01, see menus.md) | 440 | reset-video | Q | Existing |
| Video ▸ Brightness −1% / +1%, contrast, saturation | 442–447 | video-color | W/E, R/T, Y/U | Existing |
| Video ▸ Hue −1% / +1% | 448 / 449 | video-color | I / O (`repeat`) | E02 |

> **Round 2 implementation scope**: "Pixel Shaders ▸" has only the two items "Use Shaders" (Shift+P) and "HDR Tone Mapping", both disabled (reason "Needs GPU rendering, not supported in this version"); "Image Processing ▸ Frame Doubling" and "Motion Interpolation" are shown disabled with a reason (frame doubling in round 4a, motion interpolation with C-GPU); "Aspect ▸" (E11) and "Framing ▸" (E13) appear only after `video.view()` is provided in round 4a. The other rows are implemented as in the table above.

> **Round 4a implementation scope**: "Aspect ▸" (404) and "Framing ▸" (406) appear as in the table above, with the framing shortcuts as in the table below (numpad and fallback keys bound together; zoom and pan `repeat`); "Aspect ▸ Custom (x:y)" is disabled until a custom ratio has been entered, after which its title becomes "Custom (21:9)". "Image Processing ▸ Frame Doubling" is available. The two "Pixel Shaders ▸" items and "Motion Interpolation" are still disabled, with the reason changed to "Waiting for the GPU rendering prototype result" (round 4a §7.2).

> **Render round implementation scope (2026-10-21)**: "Pixel Shaders ▸" appears as in the table above: Use Shaders (Shift+P), Shader List…, Open Shader Folder, Reload Shaders; HDR Tone Mapping ▸ with seven radio items + Dynamic Peak Detection; HDR Settings…. The former placeholder command `video.shader.hdr` is replaced by `video.hdr.<algorithm>`, `video.hdr.peak` and `video.hdr.settings`; `video.shader.list`, `video.shader.folder` and `video.shader.reload` are added. "Image Processing ▸ Motion Interpolation" is available (checkbox). With software rendering (`render()` is `failed` / `unsupported`) everything except "Shader List…" and "Open Shader Folder" is disabled with the reason "Needs GPU rendering (currently software rendering)"; when the player falls back to software rendering while running, they are disabled accordingly. Another plugin holding the render options (`conflict`) does not count as disabled: the commands can be chosen as usual, and when the user acts the on-screen message is "Render options unavailable: another plugin controls them"; after the other plugin releases them, acting again (or clicking "Retry" in the window) takes effect. The manifest adds `ui.dialog`.

Framing shortcuts (E13). The binding syntax has no numpad key names now (the input in reference §4.1 only has `0–9`); the framing keys use the numpad, so the core needs to add `Numpad0`–`Numpad9`. Mac laptops have no numpad, so a set of mpv-style fallback keys is bound as well:

| Command | Numpad key (needs core Numpad names) | Fallback key (existing syntax) |
| --- | --- | --- |
| Reset Framing (Reset ▸ Framing) | Numpad5 | Alt+Backspace |
| Zoom In / Out | Numpad9 / Numpad1 | Alt+Plus / Alt+- |
| Pan → ← ↑ ↓ | Ctrl+Numpad6 / 4 / 8 / 2 | Alt+ArrowRight / Left / Up / Down |
| Center Frame | Ctrl+Numpad5 | — |

Conflict check (against `doc/product/menus.md` and all bindings in the existing plugin code):

- I, O, Ctrl+R, Ctrl+N, Ctrl+M, Ctrl+H, Ctrl+P, Ctrl+Z, Ctrl+I, Shift+F6, Alt+K, Shift+P, Alt+Plus, Alt+-, Alt+arrow keys and Alt+Backspace are not used by anyone now (this check missed subtitles Move Up/Down on Alt+↑/↓; resolved 2026-10-27, see [menus](menus.md) Conflict check). playback only uses Ctrl / Shift / Ctrl+Alt + ←/→, not Alt + arrow keys alone.
- On macOS, `Ctrl+X` is triggered by both ⌃X and ⌘X (reference §4.1). **Ctrl+Q is not bound**, because ⌘Q is Quit; "image scaling settings" becomes a page in the Picture Adjustments window, without a shortcut. The ⌘ versions of Ctrl+H and Ctrl+M are the system's "Hide" and "Minimize"; when the system takes them first, only ⌃H and ⌃M work. This is an acceptable trade-off; test with the ⌃ key during acceptance.
- Keys that pop up submenus, such as `V` (pops up the "select image" menu) and `S` (pops up the "Pixel Shaders" menu), are not bound: we have no way to pop up a main menu submenu (`commands.popup` was rejected by ruling; "select image" is not done, so `V` is no longer needed).

**Picture Adjustments window: tabbed layout**

The window keeps its width of 301 and is about 250 high (UI-TOOL-06 requires no more than 320×280), with a tab strip added at the top (31 high, each tab 74 wide, `#333` background when selected, [UI spec](ui-spec.md) §7.1). The content area is the same height on all four pages (7 rows × 24 = 168), and the bottom status line is shared by all four pages: the "Enable picture effects" checkbox, a one-sentence status, and "Retry" on failure (existing behavior kept). All rows keep the existing "label 65 right-aligned 11px + panel slider 154×12 + default 41×18, row height 24", and checkboxes keep the existing `Check` (12px box). Changes take effect immediately; there is no "Apply" button (UI-TOOL-04). The window remembers the last opened tab (stored in the plugin data directory).

| Tab | Rows (top to bottom) | ID |
| --- | --- | --- |
| **Color** | Brightness, contrast, saturation, hue, gamma (5 slider rows); preset button row: Original / Vivid / Cinema / Soft (69×22) | Existing + E02 |
| **Effects** | Each row is "checkbox + strength slider + default": Gaussian blur, sharpen, edge sharpen, denoise (the checkbox is followed by a 60-wide mode drop-down 3D / progressive / temporal, slider shrunk to 88), deblock, deband; row 7: frame doubling (checkbox 65) + small grey text "Rate already high enough" (84, when the original frame rate is ≥ 50) + motion interpolation (checkbox 100, grey with software rendering) | E03, E04, E05, E06, E18 |
| **Picture** | Flip: two toggle buttons [Vert.][Horiz.] 51×22; rotate: four radio buttons [0°][90°][180°][270°] 51×22; crop/stretch: method drop-down 72 (none / stretch / crop) + ratio drop-down 76 (4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1); aspect ratio: drop-down 154 + "…" button 21×21; display: drop-down 154; zoom: slider 154 + default (logarithmic scale 25%–400%, the current factor shown on hover; the aspect ratio, display and zoom rows are grey until `video.view()` in round 4a) | E08, E09, E11, E12, E13 |
| **Advanced** | Deinterlace: mode drop-down 90 (auto / always / never) + output drop-down 104 (by frame / by field (double rate)); HDR: drop-down 90 (tone mapping: auto / off / BT.2390 / Hable / Mobius / Reinhard / clip; a custom drop-down that moves up when the list does not fit) + target brightness slider 100; next row, small grey text (label slot empty, 214 wide) "Target 203 nits", followed by " · Not an HDR source" when the source is known not to be HDR; shaders: checkbox "Enable" 50 + text "3 shaders" 70 + [List…] 50 | E10, E16, E17 |

With software rendering, the HDR drop-down and slider on the "Advanced" page, the shaders' "Enable" and motion interpolation on the "Effects" page are grey, with the hover hint "Needs GPU rendering (currently software rendering)", which the small HDR text row also shows; [List…] stays available (before the render round the reason was "Waiting for the GPU rendering prototype result", before round 4a "Needs GPU rendering, not supported in this version"). No row is wider than the 285 content area (HDR row 267, small text row 285, shader row 253); all drop-downs are the plugin's own 21-high drop-downs, not the std-widgets ComboBox. Frame doubling on the "Effects" page and the aspect ratio, display and zoom rows on the "Picture" page are enabled in round 4a (grey before). When the aspect ratio drop-down list does not fit, it moves up to stay inside the window's bottom edge (with 7 items it covers the drop-down itself). Window height 245 (tabs 31 + top margin 10 + content 168 + spacing 6 + status line 22 + bottom margin 8).

**Processing chain order and persistence (shared by the whole chapter)**

The plugin recomputes one complete node chain each time, in a fixed order that the user cannot change: `deinterlace` → `reframe` (crop / add borders) → `transform` (flip and rotate) → `denoise` (3D → progressive → temporal) → `deblock` → `deband` → `eq` → `hue` → `sharpen` (sharpen, small) → `sharpen` (edge sharpen, large) → `gblur` (round 4a adds `frameBlend` at the end). At most 13 nodes. Reasons: deinterlacing has to come before any spatial filter; cropping and adding borders have to come before rotation (the ratio is relative to the source picture); denoise before sharpening; frame doubling last.

Effects that are off do not appear in the chain; neutral values (for example hue 0) do not produce a node either (consistent with the existing `nodesFor`). Toggling an effect changes the chain's structure, and the core rebuilds the filter graph (`UpdatePlan::NeedsReconfigure`); this is a filter reconfiguration, not a restart of the playback session. When dragging the strength slider of an enabled effect, if the core can change that parameter at runtime (marked "runtime" in the table below), only runtime commands are sent; if not, the plugin **submits only when the slider is released** (`Slider.released`, provided by `@vestra/widgets.slint`) and sends nothing during the drag, to avoid stutter from continuous rebuilds (interaction-requirements.md §3).

> Round 2 (rulings R-V1–R-V7): the core marks only the parameters of `eq`, `hue` and `gblur` as runtime-changeable; changing parameters of the other nodes (sharpen, denoise, deblock, deband, crop / add borders, flip and rotate, deinterlace) rebuilds the filter, so these strength sliders all submit only on release; wherever the sections below say "runtime" or "submit continuously while dragging", this note takes precedence.

Settings are written with `plugin.storage.json("settings")` to `plugin.dataDirectory/settings.json` (the SDK handles debouncing and temporary file plus rename; a corrupt file is renamed to `settings.json.bad-<timestamp>` and the defaults are used), and read at startup. Two classes:

- **Kept across files and restarts**: the five color items, all "Effects" page settings (including the frame doubling toggle), deinterlace mode, aspect ratio and display mode, crop/stretch choice and position, HDR algorithm and target brightness, the shader list and toggle, the last tab. These are global picture settings that do not follow the file.
- **Reset on source change**: flip, rotate, framing (zoom, pan). They usually apply to one particular file, and carrying them over to the next file would make people think the source is broken. No OSD on reset.

The file format has a version number `{ "version": 1, ... }`; for an unknown version, the plugin starts with the defaults and does not overwrite the original file until the user changes a setting.

### E02 Hue

**Requirement**: adjust the picture hue within −100%…+100%, step 1%, default 0%. ±100% corresponds to a hue angle of ±180°, so 1% = 1.8°.

**Entry points**: menu "Video ▸ Hue −1%" (I) / "Hue +1%" (O), auto-repeating while held (`repeat: true`); the "Hue" row on the "Color" page of the Picture Adjustments window (slider + default).

**Behavior details**:
- Keys and menu: the value is rounded first, then 1 is added or subtracted, then it is clamped to ±100; OSD "Hue +1% (+5%)" / "Hue -1% (-6%)", in the same format as the existing brightness messages. At +100%, pressing O again: the value does not change, and the OSD still shows "Hue +1% (+100%)" so the user knows the limit is reached.
- Reset ▸ Picture (Q) and the "Original" preset both set hue to 0; the Vivid / Cinema / Soft presets do not change hue (hue is fixed at 0 in the preset objects).
- The existing gamma is kept, as the last row of the color page.
- With no ready media: the value changes as usual, the OSD shows as usual, the filter state is `waiting`, and it is applied automatically once the media is ready (existing SDK behavior).
- With "Enable picture effects" off: keys still change the value and show the OSD, but the processing chain is empty; the OSD gets "(picture effects off)" appended.
- Audio-only sources (`player.stats().video === null`): the command runs as usual, and the core applying it to an empty video stage is not an error; if the core rejects with `unsupported`, the window status shows "No picture in the current source", with no OSD error.

**Interface**: row 4 "Hue" on the color page, slider −100…100.

**Persistence**: `color.hue` (integer).

**Core dependencies**: `plugin.video.filter` (existing). [New core API] node type:
```ts
{ kind: "hue"; degrees: number }   // −180–180; h of the FFmpeg hue filter, adjustable at runtime
```
The plugin writes `hue = round(value) * 1.8` into `degrees`; with value 0, no node is added.

**Acceptance**:
- Hold O for two seconds: the hue changes continuously without the picture freezing or stuttering; after release, the OSD shows the final value.
- Drag the slider to +100: red in a color bar test video turns cyan (hue rotated by 180°).
- Set +20, restart the player and open another file: hue is still +20.
- After Q resets, the slider is back at 0 and there is no `hue` node in the chain (TestHost `host.processing`).

**Notes**: "Hue" is the Chinese UI term 色调.

### E04 Sharpen (including edge sharpening)

> Downgraded in round 2 (ruling R-V2): there is only one node `{ kind: "sharpen"; amount: 0–2; size: "small" | "large" }` (unsharp 3×3 / 7×7); `cas` is not added. "Sharpen" = `small`, `amount` = strength × 1.5 / 100; "Edge Sharpen" = `large`, `amount` = strength / 100. Neither is a runtime parameter; the strength sliders submit only on release. This note takes precedence over `luma`/`chroma` and CAS below.

**Requirement**:
- "Sharpen": toggle + strength 0–100, off by default, strength 50. Maps to a luma sharpening factor of 0–1.5 (linear), with chroma sharpening fixed at 1/3 of the luma factor.
- "Edge Sharpen": toggle + strength 0–100, off by default, strength 50. Uses contrast adaptive sharpening (CAS), with strength mapped directly to 0–1. "Edge sharpen" is listed in the ID description of E06, but since it is also sharpening, it is written up together in this section.
- Both can be on at the same time.

**Entry points**: "Video ▸ Image Processing ▸ Sharpen" (checkbox, Ctrl+R), "Edge Sharpen" (checkbox, no shortcut); the two matching rows on the window's "Effects" page.

**Behavior details**:
- The menu and shortcut only switch the toggle, using the last strength; OSD "Sharpen: on" / "Sharpen: off", "Edge sharpen: on" / "Edge sharpen: off".
- Dragging the strength slider in the window does not turn the toggle on automatically (consistent with the existing Gaussian blur: the slider is grey while the toggle is off).
- The core cannot change the parameters of the sharpen node (unsharp) at runtime, so the strength slider submits only on release; CAS strength can change at runtime and is submitted continuously while dragging.
- Strength 0 with the toggle on: no node is added, and the check mark still shows on (the user's intent is kept).
- When on together with Gaussian blur, the order is sharpen first, then blur (see the processing chain order). The combination is not blocked; the effects just partly cancel each other.

**Interface**: rows 2 and 3 of the effects page.

**Persistence**: `effects.sharpen = { on, strength }`, `effects.cas = { on, strength }`.

**Core dependencies**: [New core API] node types:
```ts
{ kind: "sharpen"; luma: number; chroma: number } // unsharp 5×5; luma 0–1.5, chroma 0–1; not runtime
{ kind: "cas"; strength: number }                 // 0–1; adjustable at runtime
```

**Acceptance**:
- Toggling with Ctrl+R visibly changes the sharpness of text edges, and the menu check and window checkbox stay in sync.
- Dragging the sharpen strength does not make playback stutter; after release the effect updates once (TestHost: `filter.set` is called only once during the drag).
- With sharpen and edge sharpen both on for a 1080p H.264 source, the dropped frame rate on the development machine is below 1% (`player.stats().framesDropped`).

**Notes**: each kind of sharpening has only one strength (the second parameter is fixed), so one slider per window row is enough.

### E05 Denoise (3D / temporal / progressive)

> Downgraded in round 2 (ruling R-V3): there is only one node `{ kind: "denoise"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` (hqdn3d, each 0–20); `temporalDenoise` (atadenoise) is not added. The three kinds of denoise are presets in the plugin, with weights per point of strength s (spatial luma / spatial chroma / temporal luma / temporal chroma): 3D 0.1 / 0.075 / 0.15 / 0.1125 (s = 40 is the hqdn3d default); progressive (moderate, balanced) 0.06 / 0.045 / 0.06 / 0.045; temporal (mainly temporal, a little spatial) 0.02 / 0.015 / 0.2 / 0.15. Chained in the order 3D → progressive → temporal. The parameters are not runtime; the strength slider submits on release. The window drop-down order is 3D / progressive / temporal.

**Requirement**: three kinds of denoise, each with its own toggle, sharing one place for the strength setting:
- **3D Denoise**: spatial + temporal denoise (hqdn3d). Strength 0–100, default 40; mapped to luma spatial = s×0.1, chroma spatial = s×0.075, luma temporal = s×0.15, chroma temporal = s×0.1125 (s is the strength, i.e. s/40 times the hqdn3d default ratio 4:3:6:4.5).
- **Temporal Denoise**: only adaptive averaging over time (atadenoise), effective against flickering noise on still pictures. Strength 0–100, default 40, mapped to a threshold of 0.01–0.2.
- **Gradual Denoise**: hqdn3d over time only, not space (spatial components 0), keeping the most detail with the least ghosting. Strength 0–100, default 40, temporal components mapped with the 3D denoise ratio.
- When all three are on, they are chained in the order 3D → progressive → temporal.

**Entry points**: "Video ▸ Image Processing ▸ 3D Denoise" (checkbox, Ctrl+N), "Gradual Denoise" (checkbox), "Temporal Denoise" (checkbox, Ctrl+M); the "Denoise" row on the window's "Effects" page: checkbox + mode drop-down (3D / temporal / progressive) + strength slider. Whichever kind the drop-down selects, the checkbox and slider are that kind's toggle and strength; the state of each of the three is saved separately.

**Behavior details**:
- OSD: "3D denoise: on", "Gradual denoise: on", "Temporal denoise: on" (likewise for off).
- The core can change all strength parameters at runtime, so they are submitted continuously while dragging.
- Temporal denoise depends on neighboring frames. The first few frames after a seek may show slight ghosting; this is normal and needs no handling.
- Adjusting while paused: for paused video the core resets the whole graph to refresh the current frame (existing logic in `processing.rs`), so adjusting strength while paused shows the effect.

**Interface**: row 4 of the effects page.

**Persistence**: `effects.denoise = { d3: {on, strength}, temporal: {on, strength}, gradual: {on, strength}, shownMode }`.

**Core dependencies**: [New core API] node types:
```ts
{ kind: "denoise3d"; lumaSpatial: number; chromaSpatial: number; lumaTemporal: number; chromaTemporal: number } // hqdn3d, each 0–20, runtime
{ kind: "temporalDenoise"; threshold: number } // atadenoise 0a/0b/1a/1b/2a/2b all take this value, 0.01–0.3, runtime
```
Gradual denoise reuses `denoise3d` with the two spatial components set to 0.

**Acceptance**:
- On a test clip with visible noise: with 3D denoise on, the noise on the still background clearly drops; it comes back when turned off.
- With all three on, 3 denoise nodes appear in order in the chain (TestHost).
- With 3D denoise on for a 1080p source, real-time playback on the development machine drops no frames.

**Notes**: 3D denoise has only one strength, mapped with fixed ratios to the three internal parameters.

### E06 Deblock / deband

> Round 2: the `deband` threshold is not a runtime parameter (under ruling R-V4 only `hue` is), so the deband slider also submits only on release; deblock still submits only on release when the weak/strong level changes.

**Requirement** (for edge sharpen, see E04):
- **Deblock**: removes blocks in low-bitrate video. Toggle + two strength levels: weak / strong (a 0–100 slider in the window, below 50 is weak, 50 and above is strong), off by default, weak.
- **Deband**: removes color banding in gradient areas. Toggle + strength 0–100, off by default, 40; mapped to a threshold of 0.005–0.05 (linear), radius fixed at 16.

**Entry points**: "Video ▸ Image Processing ▸ Deblock" (checkbox, Ctrl+H), "Deband" (checkbox, no shortcut); rows 5 and 6 on the window's "Effects" page.

**Behavior details**:
- OSD "Deblock: on", "Deband: on" (likewise for off).
- Switching deblock between weak and strong changes the filter parameter type, which the core treats as a non-runtime parameter, so the deblock slider submits only on release when the weak/strong level changes; the deband threshold can be adjusted at runtime.
- Deblocking works on 8×8 macroblocks (the effect on 4×4 deblocking in H.264/HEVC is weaker). This is said in the hover hint: "Most effective on low-bitrate MPEG-2/MPEG-4".

**Persistence**: `effects.deblock = {on, strength}`, `effects.deband = {on, strength}`.

**Core dependencies**: [New core API] node types:
```ts
{ kind: "deblock"; strength: "weak" | "strong" }  // FFmpeg deblock filter=weak|strong, block=8; not runtime
{ kind: "deband"; threshold: number; range: number } // threshold 0.003–0.5 written to 1thr–4thr, range 1–64; runtime
```

**Acceptance**:
- On a low-bitrate MPEG-4 test clip, block edges fade with deblock on.
- On gradient sky footage, banding drops with deband on.
- After toggling with Ctrl+H, the menu check, the window checkbox and the OSD agree.

### E08 Vertical flip / horizontal flip

> Round 2 (ruling R-V5): flip and rotate are combined into one node `{ kind: "transform"; rotate; flipH; flipV }`; the core rotates first and then flips; to keep "flip first, then rotate", the plugin swaps the two flip axes at 90° / 270° before sending.

**Requirement**: two independent toggles, both off by default. Both on equals a 180° rotation.

**Entry points**: "Video ▸ Image Processing ▸ Flip Vertically" (checkbox, Ctrl+P), "Flip Horizontally" (checkbox, Ctrl+Z); toggle buttons [Vert.][Horiz.] in row 1 of the window's "Picture" page (the pressed state means on, using `checked` of `Button`).

**Behavior details**:
- OSD "Flip vertically: on", "Flip horizontally: off".
- Flipping is pixel processing, so screenshots with effects and offline exports are flipped as well.
- Reset to off on source change (see the persistence classes). If it was on before the reset, there is no message.
- Flipping is processed before rotation, so "flip horizontally + rotate 90°" flips first, then rotates.
- Subtitles do not flip with the picture (the core overlays subtitles after the filters).

**Persistence**: not saved across files.

**Core dependencies**: [New core API] node type:
```ts
{ kind: "flip"; horizontal: boolean; vertical: boolean } // hflip / vflip; the plugin does not send it when both are false
```

**Acceptance**:
- After Ctrl+P the picture is upside down and subtitles are still normal; pressing Ctrl+P again restores it.
- After opening a new file, both toggles reset to off automatically.
- A screenshot "with effects" taken while flipped is flipped too.

**Notes**: there is no separate "rotate (switch horizontal/vertical)", which would duplicate E09.

### E09 Rotate 90 / 180 / 270

> Round 2 (ruling R-V5): the node is `transform` (see E08). "Rotate automatically by aspect ratio" is not done this round: `player.stats().video.width/height` are the decoder output size, without the host's orientation handling, so portrait sources would be judged wrongly; it waits until there is data for the "upright size".

**Requirement**: rotate clockwise by 0° / 90° / 180° / 270°, radio, default 0°. The cycle order is 0 → 90 → 180 → 270 → 0. Optional "Rotate automatically by aspect ratio": portrait sources (height > width) are automatically rotated 90° to display them landscape; off by default.

**Entry points**: "Video ▸ Rotation ▸ Cycle" (Alt+K), "No Rotation / 90° / 180° / 270°" (radio), "Rotate automatically by aspect ratio" (checkbox, after the radio group); four radio buttons in row 2 of the window's "Picture" page.

**Behavior details**:
- OSD "Rotation: 90°" / "Rotation: none".
- The user's rotation is applied on top of the core's "correct orientation from metadata": the core first makes the picture upright from the source's rotation metadata (`source_geometry.rs`), and this plugin's rotation is applied on top of that.
- 90° / 270° swap the output width and height. The core's software rendering lays out again by the output ratio, and the aspect ratio setting (E11) applies to the rotated picture.
- Automatic rotation: after the source is ready, read the width and height of `player.stats().video` (already corrected by metadata). It only takes effect when the user has not chosen a rotation manually in this pass; after a manual choice, there is no more automatic rotation for this source.
- Reset to 0° on source change (with automatic rotation on, the new source is judged again).
- Rotation is pixel processing, so screenshots with effects are rotated too.

**Persistence**: only the "Rotate automatically by aspect ratio" toggle is saved; the rotation angle is not.

**Core dependencies**: [New core API] node type:
```ts
{ kind: "transpose"; degrees: 90 | 180 | 270 } // 90 = transpose=clock, 270 = transpose=cclock, 180 = hflip,vflip
```
mpv's `video-rotate` is not used: the software renderer does not support rotation, and the host now uses `format=rotate=0` specifically to zero the metadata rotation and handle it with filters instead (`INITIAL_FILTERS` in `source_geometry.rs`). The user's rotation also goes through filters, consistent with that path.

**Acceptance**:
- On a landscape source, press Alt+K four times: 90°, 180°, 270°, no rotation in turn, each with an OSD.
- At 90° the picture is shown portrait, with the correct ratio and no distortion, and black bars on both sides of the video area.
- A source with rotation metadata (shot portrait on a phone): shown upright by default; choosing 90° rotates on top of the upright picture.
- With automatic rotation on, open a portrait source: the picture is shown landscape; after manually choosing "No Rotation", it is not changed back automatically.

**Notes**: the two kinds of automatic rotation (0/90 and 180/270) are merged into one toggle that always rotates by 90°.

### E10 Deinterlace (auto / on / off)

> Downgraded in round 2 (ruling R-V6): the node is `{ kind: "deinterlace"; when: "interlaced" | "all"; output: "frame" | "field" }`, with the algorithm fixed at BWDIF. The "Method ▸ BWDIF / YADIF" radio items and the algorithm drop-down in the window are removed; the deinterlace row on the window's "Advanced" page becomes a mode drop-down + output drop-down. Persisted as `deinterlace = { mode, output }`. `method` and YADIF below no longer apply.

**Requirement**:
- Mode, one of three: **Auto** (only frames flagged as interlaced) / **Always** (all frames) / **Never**. Default auto.
- Method, one of two: BWDIF (default, good quality) / YADIF (less CPU).
- Output, one of two: by frame (frame rate unchanged, default) / by field (frame rate doubled, smoother motion).
- The cycle order is Auto → Always → Never → Auto.

**Entry points**: "Video ▸ Deinterlace ▸ Cycle" (Ctrl+I), the mode radio items, the "Method ▸" radio items, the "Output ▸" radio items; the "Deinterlace" row on the window's "Advanced" page: mode drop-down + method drop-down (output is only set in the menu, not in the window, to keep it compact).

**Behavior details**:
- OSD "Deinterlace: auto" / "Deinterlace: always" / "Deinterlace: never".
- In auto mode the node is always in the chain (it only processes flagged frames), with almost no overhead on progressive sources; with "Never" no node is added.
- Deinterlacing comes first in the chain, before every other filter of this plugin.
- "By field" output doubles the frame rate, which costs a lot of CPU on 1080i material. If `player.stats().framesDropped` grows by more than 5% of the total frames within 5 seconds, the OSD shows once "Many frames dropped with field output, try frame output" (at most once per source).
- When on together with E18 frame doubling, deinterlace first, then frame doubling.

**Persistence**: `deinterlace = { mode: "auto"|"on"|"off", method: "bwdif"|"yadif", output: "frame"|"field" }`.

**Core dependencies**: [New core API] node type:
```ts
{ kind: "deinterlace"; method: "bwdif" | "yadif"; when: "interlaced" | "all"; output: "frame" | "field" }
// deint=interlaced|all, mode=send_frame|send_field; not runtime
```
mpv's `deinterlace` property is not used: it is global player state that belongs to no plugin and cannot be cleanly withdrawn when the plugin is disabled; as a node, it follows the lifecycle of this plugin's processing chain.

**Acceptance**:
- A 1080i test clip shows comb artifacts with "Never", which disappear with "Auto".
- A progressive source with "Auto" looks exactly the same as with "Never" (pixel-by-pixel screenshot comparison, error 0).
- Ctrl+I cycles through the three modes, and the menu radio dot follows.
- With "by field" output, `player.stats().video.fps` doubles (if the core reports the output frame rate) or the motion is clearly smoother.

**Notes**: hardware deinterlacing (H/W) and field order settings are not done; the filter detects the field order automatically.

### E11 Aspect ratio and display mode

> Schedule (ruling R-V9): `plugin.video.view()` and C-SIZE are provided in round 4a (single owner, `scaleX` / `scaleY` removed), and this item is implemented in round 4a. In round 2, the aspect ratio and display rows on the window's "Picture" page are grey, and the "Aspect ▸" menu does not appear.

**Requirement**:
- **Aspect ratio** (radio): Default (Source) / 4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1 / custom. Custom takes "width:height" as two positive numbers (for example 21:9) or one decimal (for example 2.39), range 0.25–4.0. The default is "source".
- **Display mode** (radio): Keep Aspect Ratio (default, adds black bars) / Ignore Aspect Ratio (stretch to fill the video area) / Fill Window (keep the ratio and enlarge until filled, cropping what overflows).
- Cycle Aspect Ratio (Shift+F6): Default → 4:3 → 16:9 → 16:10 → 1.85:1 → 2.35:1 → (if set) custom → Default.

**Entry points**: the items under "Video ▸ Aspect ▸"; "Custom Aspect Ratio…" opens a small input window (`windows.create("ui/aspect.slint")`, 240×98: one line of explanation + a 21-high input box + [OK][Cancel]; on invalid input, "Format: 16:9 or 2.39" is shown below the box in `Theme.accent` (the theme has no red); Esc equals Cancel; "21/9" and a full-width colon are accepted too); the "Aspect ratio" (drop-down + "…" opens the same small window) and "Display" rows on the window's "Picture" page.

**Behavior details**:
- OSD wording: "Aspect ratio: Default", "Aspect ratio: 4:3" … "Aspect ratio: Custom - 21:9"; display mode "Keep Aspect Ratio", "Ignore Aspect Ratio", "Fill Window".
- The aspect ratio is a display-layer effect: screenshots, offline exports and the processing chain are not affected.
- After rotating by 90° / 270°, the aspect ratio applies to the rotated picture.
- Fullscreen and windowed modes use the same settings.
- Aspect ratio and crop/stretch (E12) can be set together: E12 first changes the ratio of the picture content, then this item decides the displayed ratio.
- Audio-only sources or cover images: the settings are kept, with no message.
- The display layer has only one owner (core rule). When another plugin already holds it, the menu and window can be used as usual; the on-screen message is "Display unavailable: another plugin controls the display", and the window status line shows "Display unavailable" (hover for the reason); after the holder releases it, acting again or clicking "Retry" takes effect. No message when it is held at plugin startup.

**Persistence**: `view.aspect` (`null` means source, otherwise a number), `view.customAspect` (the latest custom text, for example "21:9"), `view.fit`.

**Core dependencies**:
- Display layer interface (shared by E11 and E13, provided in round 4a, reference §8.1):
  ```ts
  plugin.video.view(): VideoView   // needs processing.video; the first plugin to call it owns the display layer, other plugins get conflict; a second call from the same plugin throws
  interface VideoView extends Disposable {
    set(value: Partial<ViewSettings>): void;   // merged write, last write wins
    readonly status: FilterStatus;              // the same four states as filters
    onStatus(listener: (s: FilterStatus) => void): Disposable;
  }
  interface ViewSettings {
    aspect: number | null;                // null = source; 0.25–4 → mpv video-aspect-override (null writes -1)
    fit: "keep" | "stretch" | "fill";     // keep: keepaspect=yes,panscan=0; stretch: keepaspect=no; fill: panscan=1
    zoom: number;                         // linear factor 0.25–4 → video-zoom = log2(zoom)
    panX: number; panY: number;           // −1–1 → video-pan-x / video-pan-y
  }
  ```
  Why the core has to do it: these are mpv vo-level properties that processing chain nodes cannot express, and plugins cannot write mpv properties directly. Semantics (ruling R-V9): **single owner**, a second plugin gets `conflict`; when the owner is disabled or calls `dispose()`, the core restores the five items to their defaults; on source change the core keeps the current values.
- Host behavior **C-SIZE: render size follows the video area** (done in round 4a): the render target follows the video area's physical pixels, capped at the source resolution, and its ratio always equals the video area's ratio, so "Ignore Aspect Ratio" and "Fill Window" are correct at any window ratio.

**Acceptance**:
- Choose 4:3 for a 16:9 source: the picture is squeezed horizontally, with black bars on the left and right; OSD "Aspect ratio: 4:3".
- Drag the window to nearly 1:1 and choose "Ignore Aspect Ratio": the picture fills the video area without any black bars.
- Choose "Fill Window": no black bars; the picture is cropped at the top and bottom or left and right.
- After entering a custom "21:9", the Cycle Aspect Ratio sequence includes "Custom - 21:9".
- With "abc" entered, the OK button does nothing and the format hint shows below the box.
- The size and ratio of "with effects" screenshots are not affected by the aspect ratio setting.
- After the plugin is disabled, the picture goes back to the source display.

**Notes**: "Default (Source)" means the source's output ratio; "fixed at 0.5 / 1.0 / 1.5 / 2.0× frame" is not done (framing zoom E13 achieves the same); TV output device ratio (4:3/16:9) is not done; "Fill Window" means keep the fullscreen aspect ratio and crop the excess; "Cycle Aspect Ratio" uses Shift+F6 (Ctrl+F6 is taken by macOS).

### E12 Stretch (add borders) / crop to a given ratio

> Round 2 (ruling R-V7): one node `{ kind: "reframe"; mode: "crop" | "pad"; aspect: 0.25–4; x; y: −1–1 }`; the plugin maps position 0%–100% to −1–1. In the window this becomes two drop-downs, "Method" and "Ratio" (the lists are short, so the popups stay inside the window).

**Requirement**: change the ratio of the picture **content** itself (pixel processing, unlike E11 which only changes the display). Radio:
- **None** (default).
- **Stretch to X** (X = 4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1): add black borders around the picture so that the overall ratio becomes X, without distorting the original picture. For example "Stretch to 4:3" on a 16:9 source adds black borders at the top and bottom; subtitles can be shown in the borders, and screenshots include the borders.
- **Crop to X**: cut a region with ratio X out of the picture and discard the rest. For example a 2.35:1 film squeezed into a 16:9 file with black bars: "Crop to 2.35:1" removes the top and bottom bars.
- **Position**: horizontal and vertical position, each 0%–100%, default 50% (centered), deciding which side the picture leans to when adding borders or cropping. The menu's Move Up / Move Down / Move Left / Move Right change it by 5% each time, and Reset ▸ Crop Position goes back to 50% / 50%.
- When the source ratio already equals X, the option has no effect (no node is added).

**Entry points**: the items under "Video ▸ Crop/Stretch ▸", and Reset ▸ Crop Position (Crop/Stretch ▸ Reset Position until 2026-11-01); the "Crop/Stretch" drop-down on the window's "Picture" page (the position is only adjusted in the menu).

**Behavior details**:
- OSD "Crop/stretch: crop to 2.35:1", "Crop/stretch: none"; when moving, "Crop position (50%, 45%)".
- Processing comes before rotation, so the ratio is computed for the source picture (not rotated).
- The border color is fixed black.
- The core cannot change the position parameters at runtime; on repeated arrow key presses the plugin submits the last value (the SDK already merges in-flight requests), and each key press still updates the OSD.
- When combined with E11 "source" display after cropping, the displayed ratio is the cropped ratio.
- Cropping / adding borders is pixel processing, so screenshots and offline exports include it.

**Persistence**: `crop = { mode: "none" | "pad" | "crop", aspect: number, x: number, y: number }`, kept across files (global setting).

**Core dependencies**: [New core API] node types (the core computes sizes from the input size, so the plugin does not need to know the source resolution):
```ts
{ kind: "crop"; aspect: number; x: number; y: number } // aspect 0.25–4; x, y 0–1
// crop=w='min(iw,ih*A)':h='min(ih,iw/A)':x='(iw-ow)*X':y='(ih-oh)*Y', even width and height; not runtime
{ kind: "pad"; aspect: number; x: number; y: number }
// pad=w='max(iw,ih*A)':h='max(ih,iw/A)':x='(ow-iw)*X':y='(oh-ih)*Y':color=black; not runtime
```

**Acceptance**:
- A 16:9 letterboxed source with 2.35:1 content: after "Crop to 2.35:1" the top and bottom bars disappear and the picture is not distorted.
- A 16:9 source with "Stretch to 4:3": black borders appear at the top and bottom, subtitles can show in the bottom border, and a "with effects" screenshot is 4:3.
- While cropping, press "Move Up" three times: the crop window moves up and the OSD shows the position percentages.

**Notes**: "scale by pixel units" and background color choice are not done; Ctrl+Q is not bound (see the conflict notes).

### E13 Picture zoom and pan (framing)

> Schedule and downgrade (ruling R-V9): round 4a provides `video.view()` and implements this item; `scaleX` / `scaleY` are removed, so the four commands "frame width + / −" and "frame height + / −" are deleted (already removed from the menu and shortcut tables). In round 2 the zoom row on the window's "Picture" page is grey and the "Framing ▸" menu does not appear. The numpad binding names were provided in round 2.

**Requirement**:
- **Zoom In / Out**: overall zoom, factor range 0.25–4.0, each step multiplies or divides by 1.05, default 1.0.
- **Pan → ← ↑ ↓**: move the picture, each step 1% of the video width (height), range −100%–+100%, default 0.
- **Reset Framing** (Reset ▸ Framing): restore zoom and pan to their defaults. **Center Frame**: only set the pan to zero.
- The "Zoom" slider on the window's "Picture" page controls the overall factor (25%–400%, logarithmic scale, 100% at the slider's midpoint), and its "Default" button equals Reset Framing.

**Entry points**: the items under "Video ▸ Framing ▸", Reset ▸ Framing (Framing ▸ Reset Framing until 2026-11-01), and the shortcut table above (numpad + fallback keys), repeating while held (`repeat: true`); the "Zoom" row on the window's "Picture" page.

**Behavior details**:
- OSD wording with values: "Framing: larger (110%)", "Framing: smaller (95%)", "Framing: move left (-3%, 0%)" (the pan is (horizontal, vertical), right and down positive), "Framing: reset", "Framing: centered".
- A display-layer effect; screenshots and exports are not affected.
- At the range limits the value does not change, and the OSD still shows the current value.
- Reset to the defaults on source change (no message).
- Combined with the E11 display mode: the display mode decides the picture frame first, then zoom and pan apply.
- Quality after zooming in: zooming happens at mpv's render stage; C-SIZE widens the render size with `zoom`, so zooming in does not enlarge a low-resolution buffer.
- Dragging the window's "Zoom" slider takes effect continuously (a display-layer property, no filter rebuild).

**Persistence**: not saved across files.

**Core dependencies**:
- `zoom`, `panX`, `panY` of `plugin.video.view()` (defined in E11; the core keeps the current values on source change, and the plugin resets them when `sourceId` changes).
- [New core API] the binding syntax gains numpad key names `Numpad0`–`Numpad9`, `NumpadPlus`, `NumpadMinus`, `NumpadMultiply`, `NumpadDivide`, `NumpadEnter`, `NumpadDecimal`, distinct from the digit and symbol keys of the main keyboard. Why: the framing keys are all on the numpad, while today `"5"` matches both the main keyboard and the numpad (or only one of them, depending on the host implementation), so the plugin cannot tell them apart. Keyboards without a numpad use the fallback keys.

**Acceptance**:
- With an external keyboard that has a numpad: Numpad9 zooms in, Numpad1 zooms out, Ctrl+Numpad6 pans right, Numpad5 resets, each with an OSD.
- Laptop: Alt+Plus / Alt+- / Alt+arrow keys / Alt+Backspace have the same effects.
- The digit 5 on the main keyboard does not trigger Reset Framing.
- Holding Numpad9 zooms in continuously and stops at 400%.
- After zooming to 200%, take a screenshot (with effects): the screenshot is still the full, unzoomed picture.

**Notes**: "preset" editing and screen margins are not part of this ID and are not done; "Reset Framing" only resets and does not toggle between the reset and the previous value.

### E16 HDR correction / tone mapping

> **Implemented (render round, 2026-10-21)**: connected after the host completed C-GPU and provided `plugin.video.render()` and `PlaybackStats.video.transfer` (reference §8.2, §5.4). Available with the default GPU rendering on macOS; with software rendering (headless, `VESTRA_SOFTWARE_RENDER=1`, Windows, Linux, OpenGL initialization failure) the menu is disabled and the window is grey, with the reason "Needs GPU rendering (currently software rendering)".

**Requirement**:
- "HDR Tone Mapping" radio: **Auto** (default: tone map when the source is HDR (PQ / HLG)) / **Off** / algorithms: BT.2390 / Hable / Mobius / Reinhard / Clip. These correspond to `auto`, `off`, `bt.2390`, `hable`, `mobius`, `reinhard`, `clip` of `toneMapping`.
- "Target brightness": SDR display brightness 100–400 nits, step 10, default 203 (BT.2408 reference white; only the default is not a multiple of 10, and dragging rounds to 10) → `targetPeak`.
- "Dynamic Peak Detection" toggle, on by default → `peakDetect`: estimates the peak brightness per scene, so dark scenes are not darkened overall.
- The actual behavior follows the host (reference §8.2): "Off" maps to mpv `tone-mapping=clip` + `gamut-mapping-mode=clip` without peak detection, so whatever exceeds the target brightness is clipped, which is not quite the same as "no processing at all, washed out"; macOS's OpenGL 4.1 has no compute shaders, so mpv turns dynamic peak detection off, and "Dynamic Peak Detection" is saved and sent as usual on macOS but has no effect; the output is always SDR (8-bit), even when the display supports EDR, and "Auto" always maps HDR sources (the original requirement "output HDR directly on EDR displays" is not implemented).

**Entry points**: the seven radio items under "Video ▸ Pixel Shaders ▸ HDR Tone Mapping ▸", followed by a separator and the "Dynamic Peak Detection" checkbox; "Video ▸ Pixel Shaders ▸ HDR Settings…" opens the Picture Adjustments window on the "Advanced" page; the HDR row on the "Advanced" page: algorithm drop-down 90 + target brightness slider 100 (takes effect continuously while dragging), and in the next row the small grey text "Target N nits".

**Behavior details**:
- Only HDR sources (`stats().video.transfer` is `pq` or `hlg`) are mapped; on SDR sources these settings have no effect: they are kept, and the commands can be chosen as usual. The plugin reads `transfer` once after each source becomes ready (if it cannot be read, again every 400 ms, at most 8 times). When the source is known not to be HDR, the OSD of a menu action adds a note: "HDR tone mapping: BT.2390 (not an HDR source)", and the window's small text row gets " · Not an HDR source"; on HDR sources the OSD is "HDR tone mapping: BT.2390", "HDR tone mapping: off"; no note when there is no media or the value cannot be read.
- "Dynamic Peak Detection" OSD "Dynamic peak detection: on / off". Changes in the window show no OSD (consistent with the other pages).
- Tone mapping happens at the GPU render stage, not in the processing chain; "Enable picture effects" does not affect it, and screenshots and offline exports do not include it.
- Software rendering: the seven radio items, Dynamic Peak Detection and HDR Settings… are disabled with the reason "Needs GPU rendering (currently software rendering)"; the window's HDR drop-down and slider are grey, and the hover and small text row show the same reason. When the player falls back to software rendering during playback (the host pushes `unsupported`), they are disabled accordingly.
- Host rejection (`invalid`, `backend`): window status line "Render settings failed" (hover for the reason) + "Retry". Another plugin holding the render options (`conflict`): status line "Render options unavailable", hover "Another plugin controls the render options"; when the user acts, OSD "Render options unavailable: another plugin controls them", with no message at activation.

**Persistence**: `render.toneMapping`, `render.targetPeak`, `render.peakDetect` (`settings.json`, still `version: 1`; old files without this section use the defaults). At activation the saved values are `set` in one call.

**Core dependencies** (provided in the render round):
- **C-GPU**: libmpv OpenGL rendering by default on macOS, hardware decoding `hwdec=auto-safe`; software rendering is kept as a fallback.
- Render options interface (shared by E16, E17 and E18, reference §8.2):
  ```ts
  plugin.video.render(): RenderOptions   // needs processing.video; single owner, same rules as video.view()
  interface RenderOptions extends Disposable {
    set(value: Partial<RenderSettings>): void;   // merged write, last write wins
    readonly status: FilterStatus;               // failed / unsupported under software rendering
    onStatus(listener: (s: FilterStatus) => void): Disposable;
  }
  interface RenderSettings {
    toneMapping: "auto" | "off" | "bt.2390" | "hable" | "mobius" | "reinhard" | "clip"; // → tone-mapping
    targetPeak: number;          // 100–400 nits → target-peak
    peakDetect: boolean;         // → hdr-compute-peak
    shaders: string[];           // E17: absolute paths, files must exist, at most 16 → glsl-shaders
    interpolation: boolean;      // E18: → interpolation + video-sync=display-resample + display-fps-override
  }
  ```
  When the plugin is disabled, the host restores the mpv defaults.
- `PlaybackStats.video.transfer`: the raw value of `video-params/gamma` (`bt.1886`, `srgb`, `pq`, `hlg` …); the plugin decides whether it is HDR.

**Acceptance**:
- An HDR10 test clip (PQ / BT.2020) on an SDR display: "Off" and "Auto" look different, and "Auto" has normal brightness and color.
- Switching algorithms changes the picture immediately, without interrupting playback.
- Changing the target brightness from 100 to 300 changes the overall brightness of the picture.
- SDR source: the window's small text shows "Not an HDR source", and switching algorithms in the menu adds the same note to the OSD.
- Start with `VESTRA_SOFTWARE_RENDER=1`: the menu items are disabled and show the reason, and the window controls are grey.

**Notes**: HDR has only one "Auto" option, without separate ST 2084 / ST 2086; ICC correction and YCgCo correction are not part of this ID; on macOS the system color management takes care of them.

### E17 Pixel shaders (custom GLSL)

> **Implemented (render round, 2026-10-21)**: goes through the `shaders` field of `plugin.video.render()`; with software rendering everything except "Shader List…" and "Open Shader Folder" is disabled with the reason "Needs GPU rendering (currently software rendering)" (the list can be prepared in advance).

**Requirement**:
- The user can load GLSL user shaders in mpv format (`.glsl` or `.hook`, with `//!HOOK` declarations, for example FSRCNNX, Anime4K, KrigBilateral), applied on top of each other in list order, at most 16.
- Master toggle "Use Shaders" (off by default); when off, an empty list is sent and the plugin keeps its own list.
- Shader folder: `plugin.dataDirectory/shaders/`, created the first time something is added or the folder is opened. `.glsl` / `.hook` files the user puts in it appear in the available list (files whose names start with `.` do not count).
- "Shader List…" opens a small window (`ui/shaders.slint`, 301×260): title row "Enabled N/16 (run in order)"; the enabled list (219 wide, 113 high, 20 per row, click to select) with a column of 60×21 buttons on the right [Add…][Remove][Move Up][Move Down][Clear]; below it the list "Other files in the shader folder (double-click to add)" (3 rows high, scrollable); on the bottom row, a line of red explanatory text on the left ("Needs GPU rendering (currently software rendering)" with software rendering, ""Use Shaders" is off" when the toggle is off) and [Open Folder] on the right. Items whose files no longer exist are shown as "name (file missing)" and skipped on submit.
- "Add…" uses the system file dialog (`plugin.dialogs.openFile`) to pick a file anywhere; the plugin **copies** it into the shader folder (a file with the same name is replaced; nothing is copied when the chosen file is already in the folder), then adds it to the list and selects it, so the list stays valid after the original file is moved away. When it is not `.glsl` / `.hook`, OSD "Only .glsl or .hook shader files can be added"; with 16 already, OSD "At most 16 shaders"; when copying fails, OSD "Could not copy the shader: <reason>".
- "Remove" only takes it out of the list; the file stays in the folder (back in the available list below); "Clear" works the same way.
- "Reload Shaders": rescan the folder, remove items whose files no longer exist from the list with the message "Removed missing shaders: a.glsl, b.glsl" (otherwise "Shaders reloaded (N)"), then submit the list again (for after editing shaders externally; whether the same list makes mpv recompile is up to the host).
- "Open Shader Folder": `plugin.system.openPath` (opened by the player, Finder on macOS, File Explorer on Windows); on failure OSD "Could not open the shader folder: <reason>".

**Entry points**: "Video ▸ Pixel Shaders ▸ Use Shaders" (checkbox, Shift+P), "Shader List…", "Open Shader Folder", "Reload Shaders"; the "Shaders" row on the window's "Advanced" page: checkbox "Enable" + text "N shaders" + [List…] button.

**Behavior details**:
- OSD "Pixel shaders: on (3)" / "Pixel shaders: off"; turning the toggle on with an empty list: OSD "Pixel shaders: no shaders"; the toggle is still recorded as on and takes effect once the first shader is added. Checking in the window shows no OSD.
- Absolute paths are sent to the host (`<dataDirectory>/shaders/<name>`).
- Compile failure: mpv compiles a shader the first time it is used, so the host first reports `applied` and later pushes the state to `failed` (`message` starts with `shader compile failed: ` and carries the compiler's first error line, **without the file name**, which mpv does not report). The plugin shows the OSD "Shader failed to load", and the window status line "Shader failed to load" (hover for the full reason) + "Retry"; the other shaders in the list still run (host behavior). The user can recover by removing the faulty one from the list (or turning the toggle off).
- Shaders run at the GPU render stage, not in the processing chain; "Enable picture effects" does not affect them, and screenshots and exports do not include them.
- Shaders can be very GPU-heavy (for example FSRCNNX on 4K). The plugin makes no performance judgment.

**Persistence**: `render.shadersEnabled`, `render.shaders` (list of file names relative to the shader folder; on read, items with paths, hidden items or wrong extensions are removed, duplicates removed, at most 16). The shader files themselves live in `plugin.dataDirectory/shaders/`.

**Core dependencies**: C-GPU; the `shaders` field of `plugin.video.render()` (defined in E16), with absolute paths; `plugin.dialogs.openFile` (needs `ui.dialog`, added to the manifest this round); `plugin.system.openPath`.

**Acceptance**:
- Add a test shader that inverts the picture and turn the toggle on: the picture is inverted; after Shift+P turns it off, it is restored.
- Add a shader with a syntax error: the OSD and the window status line show "Shader failed to load", hovering shows the compiler error, and playback is not interrupted.
- After "Add…" with a file on the desktop, a copy appears in the shader folder; delete the original on the desktop, and the list is still valid.
- The list and toggle state are kept after restarting the player.

**Notes**: mpv shaders declare their own stage with `//!HOOK`, so one list and one toggle (Shift+P) are enough, without separate "pre-resize / post-resize" groups.

### E18 Frame doubling / motion interpolation

> **Motion interpolation implemented (render round, 2026-10-21)**: goes through the `interpolation` field of `plugin.video.render()`; with software rendering the menu item and window checkbox are disabled with the reason "Needs GPU rendering (currently software rendering)". Frame doubling (frameBlend, software) was implemented in round 4a.

> Schedule (ruling R-V8): frame doubling (`frameBlend`) is implemented in round 4a and verified in the same batch as C-SIZE's render pacing changes; motion interpolation is connected in the render round, and the mutual exclusion of the two takes effect then.

**Requirement**:
- **Frame doubling** (software): raises the frame rate to 2× (at most 60 fps), generating the in-between frames by blending the frames before and after, so motion in low-frame-rate pictures is smoother (with slight motion blur). A toggle, off by default. Not applied when the original frame rate is already at least 50 fps.
- **Motion interpolation** (GPU): mpv's interpolation, resampling the output to the display refresh rate (the host sets `interpolation=yes`, `video-sync=display-resample`, `display-fps-override` = the main display's refresh rate), removing the judder of 24 fps content on 60 Hz displays. A toggle, off by default.
- The two are mutually exclusive: turning one on turns the other off automatically, with the OSD "Frame doubling: on (motion interpolation turned off)" / "Motion interpolation: on (frame doubling turned off)" (shown from both the menu and the window, because the other toggle was changed).

**Entry points**: "Video ▸ Image Processing ▸ Frame Doubling" (checkbox), "Motion Interpolation" (checkbox), neither with a shortcut; the two checkboxes in row 7 of the window's "Effects" page.

**Behavior details**:
- OSD "Frame doubling: on", "Motion interpolation: on"; checking in the window shows an OSD only when the mutual exclusion kicks in.
- The frame doubling node goes last in the chain (the core also always puts it last in the whole video graph). Target frame rate = min(original frame rate × 2, 60) (at least 24), with the original frame rate read from `player.stats().video.fps`; when it is `null` (unknown), 2 × 30 = 60 is used. With frame doubling on, `stats().video.fps` reads the output frame rate, so the original frame rate is read while the chain has no frame doubling node: once per source and per deinterlace setting (field output doubles the original frame rate), and no node is added until it has been read; with no picture, it gives up after about 3 seconds until the source changes or the box is checked again.
- With an original frame rate ≥ 50, no node is added, the check still shows on, and the small grey text "Rate already high enough" is shown next to the frame doubling checkbox in the window.
- Frame doubling runs on the CPU, so 4K sources may drop frames. If more than 5% of the total frames are dropped within 5 seconds, the OSD shows once "Many frames dropped with frame doubling; consider turning it off" (once per source). When "Enable picture effects" is off, frame doubling is withdrawn too; motion interpolation is not part of the processing chain and is not affected by "Enable picture effects".
- Frame doubling is included in screenshots with effects and in offline exports (the exported video's frame rate changes accordingly); motion interpolation is not.
- Motion interpolation resamples to the main display's refresh rate, so it may not be smooth enough when the window is on a display with another refresh rate (host limitation).
- With software rendering, motion interpolation is disabled with the reason "Needs GPU rendering (currently software rendering)"; the saved toggle does not change.

**Persistence**: `effects.frameBlend`, `render.interpolation` (if both are on when read, frame doubling is kept and motion interpolation turned off).

**Core dependencies**:
- Frame doubling: node type
  ```ts
  { kind: "frameBlend"; fps: number } // 24–120; FFmpeg framerate=fps=N (blending interpolation); not runtime
  ```
- Motion interpolation: C-GPU; the `interpolation` field of `plugin.video.render()` (defined in E16). FFmpeg `minterpolate` (motion-estimation interpolation) is not used, because it is nowhere near real time at 1080p.

**Acceptance**:
- A 24 fps panning shot: with frame doubling on, motion is clearly smoother, and `player.stats().video.fps` or the look shows 48 fps.
- A 60 fps source with frame doubling on: there is no `frameBlend` node in the chain.
- Turn frame doubling on, then motion interpolation: frame doubling turns off automatically with an OSD; likewise the other way round.
- With motion interpolation on, the frame count from `VESTRA_FRAME_LOG=1` is close to the display refresh rate, and the picture does not stutter.
- With frame doubling on, export a 5-second clip "with effects": `ffprobe` shows 48 fps (24 fps source).

**Notes**: "frame doubling" is blending interpolation; motion interpolation is provided by mpv's GPU interpolation.

**Chapter core API request summary**

| Interface | Reason |
| --- | --- |
| `VideoNode` gains `{ kind: "hue"; degrees }` (−180–180, runtime) | E02. FFmpeg eq has no hue, so the hue filter is needed; plugins cannot add filter strings themselves |
| `VideoNode` gains `{ kind: "sharpen"; luma; chroma }`, `{ kind: "cas"; strength }` | E04 sharpen, edge sharpen |
| `VideoNode` gains `{ kind: "denoise3d"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }`, `{ kind: "temporalDenoise"; threshold }` | E05 three kinds of denoise |
| `VideoNode` gains `{ kind: "deblock"; strength }`, `{ kind: "deband"; threshold; range }` | E06 |
| `VideoNode` gains `{ kind: "flip"; horizontal; vertical }`, `{ kind: "transpose"; degrees: 90 \| 180 \| 270 }` | E08, E09. The software renderer does not support `video-rotate`; the host already handles orientation with filters |
| `VideoNode` gains `{ kind: "deinterlace"; method; when; output }` | E10. The node is withdrawn with the plugin's lifecycle, which the global `deinterlace` property cannot do |
| `VideoNode` gains `{ kind: "crop"; aspect; x; y }`, `{ kind: "pad"; aspect; x; y }` (sizes computed by the core from the input) | E12. The plugin does not know the filter input size |
| `VideoNode` gains `{ kind: "frameBlend"; fps }` | E18 frame doubling |
| `plugin.video.view(): VideoView`, `set(Partial<{ aspect; fit; zoom; panX; panY }>)`, single owner (provided in round 4a; ruling R-V9 removed `scaleX`, `scaleY`) | E11, E13. These are mpv vo-level properties that the processing chain cannot express, and plugins cannot write mpv properties directly |
| Host behavior C-SIZE: render size follows the video area (done in round 4a; before, a fixed 960×540 then contain scaling) | Prerequisite for E11 and E13; otherwise stretch and fill are computed against the wrong 16:9 buffer |
| Binding syntax gains `Numpad0`–`Numpad9`, `NumpadPlus`, `NumpadMinus`, etc. | The E13 framing keys are all on the numpad, which the existing syntax cannot distinguish |
| Host behavior C-GPU: libmpv switches to GPU rendering (done in the render round: OpenGL by default on macOS, software rendering as fallback) | Prerequisite for E16, E17 and E18 motion interpolation; the software renderer does not support these three |
| `plugin.video.render(): RenderOptions`, `set(Partial<{ toneMapping; targetPeak; peakDetect; shaders; interpolation }>)` (provided in the render round, single owner) | E16, E17, E18 motion interpolation, matching mpv GPU render properties |
| `PlaybackStats.video.transfer` (provided in the render round: the raw value of `video-params/gamma`, such as `bt.1886`, `pq`, `hlg`) | E16 decides whether the source is HDR |
| ~~(optional) `plugin.commands.popup(path: string[]): Promise<void>`~~ | Rejected by ruling; S is not bound |

Every new node has to support both the live mpv graph and the offline export graph (shared `render_lavfi`); the total number of nodes is still bound by the existing limit of "64 nodes for all plugins together". In the worst case this plugin uses about 15 nodes.

---

## Audio workbench (`audio`)

**Location**: `src/plugins/audio/` (existing plugin, manifest id `vestra.audio`)

**Scope**

This plugin owns every feature in the "Audio" menu except volume and mute: audio track selection and loading external tracks, channels, equalizer, audio sync, the various audio effects, and the output device. Today it has a 6-band equalizer, 4 presets, a delay slider, normalize and clarity enhancement, acting on its own audio processing chain `workbench`. This round adds 7 items, A11, G03–G05, G09, G11 and G16, and the window gets three tabs. Basic volume and mute still belong to playback; subtitle sync belongs to subtitles.

Boundary with the core:
- Channel processing continues to go through this plugin's processing chain `plugin.audio.filter("workbench")`; the core only needs to add node types to `AudioNode` (`AudioNode` in `effects.rs` currently has only `Volume`, `Equalizer`, `Treble`, `Dynaudnorm`).
- The audio track list and selection, loading external tracks, and the output device are player state and need new core interfaces.
- The audio sync and crystalizer shortcuts, and automatic loading of same-name external audio (Node scans the folder), are done in the plugin and do not need the core.

**Behavior change (important)**: today "settings belong to one loaded source + one selected audio track, and any change resets them to neutral" (README). This round changes that: **equalizer, presets, normalize, crystalizer, the various effects and the channel mode are global settings**, kept across audio tracks, files and restarts (stored in the plugin data directory); **only audio sync (delay) follows the source**, and whether it resets is decided by the "Audio Sync" keep mode (G09). Reason: G03/G04 let users switch audio tracks often, and clearing the equalizer on every switch would be a poor experience. After a track or file change, the SDK automatically reapplies the current processing chain to the new track (existing behavior).

Manifest capabilities added: `playback.control` (switch audio track, load external audio track, choose output device), `ui.dialog` (file selection for A11).

**Feature overview**

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| A11 | Load external audio track | ✗ | Yes: `player.addAudioTrack` |
| G03 | Select audio track | ✗ | Yes: `player.selectAudioTrack` (ruling R-T2) |
| G04 | Cycle audio tracks | ✗ | Same as G03 |
| G05 | Channel selection (stereo / left / right / mono mix / swap left and right) | ✗ | Yes: `channels` node |
| G09 | Audio sync shortcuts | ◐ | No (delay range widened to ±10 000, ruling R-A5, provided in round 2) |
| G11 | Crystalizer shortcuts | ◐ | No |
| G16 | Audio output device selection | ✗ | Yes: `player.audioDevices`, `player.setAudioDevice` (round 3a, rulings R-A2–R-A4) |

**Menus and shortcuts**

Menu structure: select audio (including channels) → output device → volume (playback) → equalizer → presets → audio matching (sync) → audio processing. The existing order 510–516 is reallocated within the 510–529 range:

| Menu path | order | group | Default shortcut | ID |
| --- | --- | --- | --- | --- |
| Open ▸ Load External Audio… | 114 | open | None | A11 |
| Audio ▸ Volume + / Volume − / Mute (playback, unchanged) | 501–503 | domain | ↑ / ↓ / M | — |
| Audio ▸ Audio Workbench… | 510 | audio-tools | | Existing |
| Audio ▸ Audio Track ▸ [each track, radio] · —— · Cycle · —— · Stereo · Left Channel · Right Channel · Mono Mix · Swap Left and Right (radio) | 511 | audio-tools | Cycle Alt+A | G03, G04, G05 |
| Audio ▸ Output Device ▸ System Default · [each device] (radio) (round 3a) | 512 | audio-tools | | G16 |
| Audio ▸ Equalizer (checkbox) | 513 | audio-eq | Shift+E | Existing |
| Audio ▸ Preset ▸ Flat / Voice / Music / Cinema (radio) | 514 | audio-eq | | Existing |
| Audio ▸ Audio Sync ▸ 50 ms Earlier · 50 ms Later · —— · Don't Keep · Keep for This Run · Always Keep (radio) | 515 | audio-eq | `>` / `<` / `?` (`?` is Reset ▸ Audio Sync, 530, reset-audio, since 2026-11-01; see below) | G09 |
| Audio ▸ Processing ▸ Normalize · Crystalizer (checkboxes) | 516 | audio-eq | Shift+N · Shift+C | G10 (entry point only), G11 |
| Reset ▸ Audio Processing (moved to Reset ▸ 2026-11-01, see menus.md); equalizer, Normalize and Crystalizer only, sync and channels kept | 531 | reset-audio | | Existing |

"Open ▸ Load External Audio…" goes in the "Open" menu, right after subtitles' "Open Subtitle…" (113), and is not repeated under "Audio ▸ Audio Track". Normalize (G10, an existing feature) only gets an entry point and Shift+N in the "Processing" submenu, because it belongs to the audio processing group, which would be incomplete without it.

**Shortcut conflicts and trade-offs**:
- The physical keys for audio sync are Shift+. / Shift+, / Shift+/. Our binding syntax matches the typed character and does not write Shift (reference §4.1), which on a US keyboard is `>` `<` `?`. subtitles currently uses `>` `<` `/` (subtitle sync). **Decision**: audio uses `>` `<` `?`; subtitle sync changes to `.` `,` `/` (already updated in the subtitles chapter). The reset key `?` does not conflict with the subtitles' `/`.
- Alt+A, Shift+C and Shift+N are not used by anyone now (the only existing binding with Shift is Shift+E).
- `A` (pops up the "select audio" menu) pops up a submenu and is not bound this round, for the same reason as V and S in the video chapter.

**Audio Workbench window: tabbed layout**

The width stays at 301, height about 260 (UI-TOOL-07: no more than 320×280). At the top is the same tab strip as the Picture Adjustments window (31 high, tabs 74 wide); the content area is the same height on all three pages; the bottom status line is shared (the existing status text, "Retry" on failure). The last opened tab is remembered.

| Tab | Content | ID |
| --- | --- | --- |
| **Equalizer** | The existing "Equalizer" toggle, preset popup button and 6 vertical band sliders, unchanged | Existing |
| **Effects** | Each row is "label 65 + slider 154 + default 41" or "checkbox 65 + slider + default": audio sync (−10000…+10000 ms, value shown on the right, existing), normalize (checkbox, existing), crystalizer (checkbox + 0–6 dB slider, the existing "clarity") | G09, G11 |
| **Output** | Audio track: drop-down 154 + [Load…] button 50×18 ("Load…" does not fit in 41); channels: drop-down 154 (Stereo / Left Channel / Right Channel / Mono Mix / Swap Left and Right); output device: drop-down 200 (grey before round 3a, with the reason on hover); checkbox "Auto-load audio files with the same name"; audio sync keep mode: drop-down 154 (Don't Keep / Keep for This Run / Always Keep) | A11, G03, G05, G16, G09 |

The tab strip takes 31 in height, so the existing three rows "delay / normalize / clarity" move from below the equalizer to the "Effects" page, and the equalizer page holds only the equalizer.

**Processing chain order and persistence (shared by the whole chapter)**

The chain order is fixed: `channels` → `equalizer`×6 → `treble` (crystalizer) → `dynaudnorm` (normalize). Reason: decide the channels first, then correct the frequency response, and normalize last to catch the loudness.

Settings are written with `plugin.storage.json("settings")` to `plugin.dataDirectory/settings.json` (the SDK handles debouncing and temporary file plus rename; a corrupt file is renamed to `settings.json.bad-<timestamp>` and the defaults are used; with `version`): `{ version: 1, equalizer, gains[6], normalize, clarity: { on, db }, channels: { mode }, autoLoadExternal, syncKeep, syncMs?, tab }` (`device` is added in round 3a).

Effects that are off produce no nodes, and neutral settings are still an empty chain (existing principle). Toggling an effect changes the chain structure, and the core rebuilds the audio filter graph (possibly a very short audio dropout, not a session restart).

### A11 Load external audio track

> Round 2: `player.addAudioTrack` is provided. An external audio track stays until the source changes and is not withdrawn when the plugin is disabled (that is how the core works).

**Requirement**:
- Load an external audio file for the video currently playing (.mka / .m4a / .aac / .ac3 / .eac3 / .dts / .flac / .mp3 / .ogg / .opus / .wav, etc., whatever the core can decode), added as a new audio track and selected immediately.
- Several can be loaded, each becoming a separate audio track.
- "Auto-load audio files with the same name" toggle (off by default): when a local file is opened, the plugin scans the same folder and automatically loads files whose names start with the video's base name and whose extensions are in the audio formats above (for example `movie.ac3`, `movie.jpn.mka` for `movie.mkv`), but does not select them automatically (the core's default selection is kept).

**Entry points**: "Open ▸ Load External Audio…"; the [Load…] button in the audio track row on the window's "Output" page; the "Auto-load audio files with the same name" checkbox on the same page.

**Behavior details**:
- Files are picked with `plugin.dialogs.openFile({ title: "Load External Audio" })`. On cancel nothing happens.
- The submit is conditioned on the `player.state` from before the dialog opened (`ifCurrent`). If the user changed files while the dialog was open, it fails with `stale`, OSD "The source changed; the audio was not loaded".
- On success, OSD "Audio loaded: movie.jpn.mka", and the new track is selected (the G03 menu rebuilds automatically as `audioTracks` changes).
- Decode failure: OSD "Cannot load audio: <reason given by the core>"; the track list does not change.
- With no source (`phase` is `empty`) the command is disabled with the reason "No file is playing"; also disabled while `loading`, with the reason "Loading".
- Loading the same path again for the same source: the plugin remembers the paths loaded for this source, sends no request for a repeat, OSD "This audio is already loaded".
- External audio tracks belong to the current source and disappear after the source changes (core behavior); automatic loading rescans each time a source becomes ready.
- Network sources (http/https): loading a local file manually is allowed as usual (the timeline may not match; the user adjusts with audio sync); automatic loading only applies to local files.
- The automatic scan only looks at the same folder level, not recursively, and loads at most 8 files; the scan runs asynchronously in Node and failures (no permission, etc.) are skipped silently.
- External audio tracks cannot be used in clip export (existing core rule: `AudioTrack.external`, `exportable`).

**Persistence**: `autoLoadExternal`.

**Core dependencies**: `plugin.dialogs.openFile` (existing); `player.state.audioTracks` (existing). [New core API]
```ts
player.addAudioTrack(path: string, opts?: { select?: boolean; ifCurrent?: PlayerState }): Promise<AudioTrack>
// needs playback.control; corresponds to mpv `audio-add <path> select|auto`; select defaults to true
```
Why the core has to do it: adding a track to the current playback session is an operation inside the player that plugins cannot do.

**Acceptance**:
- While playing a silent video, load an .mp3: the audio track menu gains an item that is selected, and sound is heard.
- With automatic loading on, open `a.mkv`: `a.ac3` in the same directory appears in the audio track list but is not selected.
- While picking a file in the dialog, switch to the next file through the playlist; after confirming, "The source changed" is shown, and the new file's audio tracks are not affected.
- Pick a text file: "Cannot load audio" is shown, and playback is not interrupted.

### G03 Select audio track

> Round 2 (ruling R-T2): the interface is `player.selectAudioTrack(id, { ifCurrent })`. In the "Audio Track ▸" submenu, the track list is rebuilt as `audioTracks` changes (order 511), and Cycle and the five channel modes are fixed after it (order 512, separated as a group).

**Requirement**: list all audio tracks of the current source (including externally loaded ones) and switch with radio items. Each item shows "number. title [language] (codec)"; external tracks get "(external)"; missing fields are left out, for example "1. Japanese [jpn] (aac)", "3. movie.ac3 (ac3) (external)"; with no title and no language, "Track 2 (opus)".

**Entry points**: "Audio ▸ Audio Track ▸ [each track]" (radio); the audio track drop-down on the window's "Output" page; if playback's bottom bar ever gets a "stream selection" button, playback implements it itself by reading `player.state.audioTracks`; this plugin does not provide it.

**Behavior details**:
- OSD "Audio track: 1. Japanese [jpn] (aac)".
- The menu is rebuilt when `player.state.audioTracks` changes (compare the id list and the selection; when only the check changes it must refresh too, which a `checked` function handles without a rebuild).
- `audioTracks === null` (loading): the submenu only shows a disabled "Loading…"; `[]` (no audio tracks): only a disabled "No Audio Tracks".
- Selecting the already selected track: no request, no message.
- The submit is conditioned on the current `player.state`; on `stale` nothing happens and there is no message (the source has changed, and the menu will be rebuilt accordingly).
- After switching tracks, the SDK automatically reapplies this plugin's processing chain to the new track; equalizer and other settings are not reset (see "Behavior change" in this chapter).
- After a source change, the core's default selection applies; the user's last choice is not remembered. "Remember the user's language / stream choice" is not part of this ID.
- Plugin-generated streams (`kind: "stream"`, for example NES) without a track list are treated as "No Audio Tracks".

**Interface**: row 1 of the "Output" page.

**Persistence**: none.

**Core dependencies**: `player.state.audioTracks` (existing). [New core API]
```ts
player.selectTrack(kind: "audio", id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>
// needs playback.control; corresponds to mpv `aid`; kind is only "audio" for now, the parameter is kept for later extension
```
Why the core has to do it: switching audio tracks is an operation inside the player, and plugins cannot reach mpv.

**Acceptance**:
- Bilingual MKV: the submenu lists two audio tracks with a radio dot on the current one; choosing the other switches the sound, with the correct OSD.
- Set the equalizer to "Voice", then switch tracks: the equalizer is still "Voice".
- Open a file with no audio: the submenu only has a disabled "No Audio Tracks".

### G04 Cycle audio tracks

**Requirement**: switch to the next track in list order; after the last one, back to the first.

**Entry point**: "Audio ▸ Audio Track ▸ Cycle" (Alt+A).

**Behavior details**:
- OSD the same as G03: "Audio track: 2. English [eng] (ac3)".
- Only one audio track: no request, OSD "Only one audio track". No audio tracks or loading: the command is disabled with the reason "No audio tracks" / "Loading".
- Fast repeated presses: each computes the next track from the latest `player.state`. When the previous switch has not completed (in flight in the SDK), this press is computed from the selection the plugin sees, which may be one step behind but never wrong or failing. The plugin does not queue.
- No grouping by language; strictly list order.

**Persistence**: none.

**Core dependencies**: same as G03.

**Acceptance**:
- In a file with three audio tracks, press Alt+A three times: tracks 2, 3, 1 in turn, each with an OSD.
- In a single-track file, press Alt+A: "Only one audio track" is shown.

### G05 Channel selection (stereo / left / right / mono mix / swap left and right)

> Round 2 (ruling R-A1): node `{ kind: "channels"; mode }`; the core uses `pan`, and more than two channels are first downmixed to stereo (`aformat`). The mode parameter is not runtime: switching channels rebuilds the audio filter (possibly a very short dropout); this note takes precedence over "adjustable at runtime" and "no noticeable dropout" below. The channel count is read from `player.stats().audio.channels` when the source is ready and after a track change, and read again 1 second later (the decoder switches tracks with a delay).

**Requirement**: radio, default "Stereo":
- **Stereo**: output as is.
- **Left Channel**: both speakers play the left channel.
- **Right Channel**: both speakers play the right channel.
- **Mono Mix**: left and right are mixed as (L+R)/2, the same on both sides. For recordings with sound on only one side, or single-ear headphones.
- **Swap Left and Right**: the left speaker plays the right channel, and the right speaker plays the left channel. For recordings or headphones wired the wrong way round.

**Entry points**: "Audio ▸ Audio Track ▸ Stereo / Left Channel / Right Channel / Mono Mix / Swap Left and Right" (radio, after the track list and Cycle); the "Channels" drop-down on the window's "Output" page.

**Behavior details**:
- OSD "Channels: Left Channel".
- Mono audio track (`player.stats().audio.channels === 1`): all five options are disabled with the reason "The current track is mono"; the setting is kept and takes effect automatically when switching to a stereo track.
- Multichannel tracks (5.1 / 7.1): when anything other than "Stereo" is chosen, the core first mixes the audio to stereo, folding the surround channels into left and right (the OSD gets "(mixed to stereo)" appended, shown on every choice). Choosing "Stereo" again restores the original multichannel output (no node).
- The channel count is read with `player.stats()` when the source is ready and after switching tracks; if the read fails, stereo is assumed.

**Persistence**: `channels.mode`.

**Core dependencies**: [New core API] node type:
```ts
{ kind: "channels"; mode: "stereo" | "left" | "right" | "mono" | "swap" }
// FFmpeg stereotools mode: lr>lr / lr>ll / lr>rr / lr>l+r / lr>rl; adjustable at runtime
// with more than 2 input channels the core inserts a stereo downmix before it; the plugin does not send it for mode=stereo
```

**Acceptance**:
- Test tone that differs left and right (left 440 Hz, right 880 Hz): after "Left Channel" both sides are 440 Hz; after "Mono Mix" both sides are the same and contain both frequencies.
- After "Swap Left and Right", the left ear hears 880 Hz and the right ear 440 Hz.
- On a mono file the five options are disabled, and hovering shows the reason.
- A 5.1 file with "Right Channel": the "mixed to stereo" message appears, and the sound is normal.
- No noticeable dropout when switching (runtime parameter).

**Notes**: includes "Mono Mix" (required by the G05 ID description); "swap left and right channels" is merged into this radio group as one channel mode.

### G09 Audio sync shortcuts

**Current state**: the window has a delay slider (±2000 ms, rounded to 10 ms, positive values make the audio later), with no menu commands and no shortcuts; the delay resets when the source or audio track changes.

**Requirement** (only the gaps):
- Menu commands: "50 ms Earlier" (delay −50, sound earlier), "50 ms Later" (delay +50, sound later), auto-repeating while held; Reset ▸ Audio Sync (delay 0; Audio Sync ▸ Reset until 2026-11-01). Step 50 ms, range ±10 000 ms (widened in core round 2, ruling R-A5).
- Keep mode (radio, default "Don't Keep"):
  - **Don't Keep**: the delay goes back to 0 on every source change (as now).
  - **Keep for This Run**: the delay is kept on source change and goes back to 0 after the player quits.
  - **Always Keep**: the delay is written to the data directory and still applies after a restart.
- Switching audio tracks (G03/G04) **no longer** resets the delay (different tracks of the same source usually need the same offset; users adjust themselves when they need something else).

**Entry points**: the items under "Audio ▸ Audio Sync ▸", and Reset ▸ Audio Sync; shortcuts `>` earlier, `<` later, `?` reset (subtitle sync changes to `.` `,` `/` accordingly, see the conflict notes); the audio sync slider on the window's "Effects" page (existing), the keep mode drop-down on the "Output" page.

**Behavior details**:
- OSD wording: "Audio sync: -0.05 s", "Audio sync: +0.3 s", "Audio sync: 0 s".
- At the ±10 000 ms limit the value does not change, and the OSD still shows the current value.
- With no audio track selected, the command is disabled with the reason "No audio track selected" (consistent with the existing editing commands).
- The delay is submitted through the existing `filter.set({ nodes, delayMs })`, which does not change the node chain or rebuild the filter graph.
- External audio tracks (A11) are covered too.
- Subtitle sync is another plugin, and the two do not affect each other; the OSD wording "Audio sync" tells them apart.

**Persistence**: `syncKeep`; with "Always Keep", `syncMs` is saved as well.

**Core dependencies**: the existing `delayMs` of `audio.filter`. [New core API, optional] widen the `delayMs` range from ±2000 to ±10000 (consistent with the subtitles' `offsetMs`), because externally loaded audio tracks (A11) are often off by more than 2 seconds. The core now budgets "positive and negative contributions each at most 2000" (`processing.rs`), which has to be adjusted together.

**Acceptance**:
- Press `>` three times: the OSD shows "-0.05 s", "-0.1 s", "-0.15 s" in turn, and the window slider moves along.
- `?` resets to 0.
- With "Don't Keep" the delay is 0 when the next file opens; with "Keep for This Run" it is kept, and 0 after a restart; with "Always Keep" it is still kept after a restart.
- After switching audio tracks, the delay stays the same.
- The subtitle sync keys `.` `,` `/` only affect subtitles, and `>` `<` `?` only affect audio.

**Notes**: the step is fixed at 50 ms; there is no setting to change it.

### G11 Crystalizer shortcuts

**Current state**: "clarity enhancement" (3 kHz high-shelf boost 0–6 dB) can only be dragged in the window, with no toggle, menu or shortcut; a value of 0 equals off.

**Requirement** (only the gaps):
- Rename "clarity enhancement" to "Crystalizer" and split it into a toggle + strength: the toggle is off by default, strength 0–6 dB, default 3 dB, step 0.1.
- The menu item "Audio ▸ Processing ▸ Crystalizer" (checkbox, Shift+C) only switches the toggle, using the last strength.
- The crystalizer row on the window's "Effects" page: checkbox + slider + default. With the checkbox off, the slider is grey but keeps its value.
- Old settings migration: an old `clarity > 0` migrates to toggle on with the original value as strength; `clarity = 0` migrates to toggle off with strength 3.

**Behavior details**:
- OSD "Crystalizer: On" / "Crystalizer: Off".
- Toggle on but strength 0: no node is produced, and the check still shows on.
- With no audio track selected, the command is disabled (consistent with the existing editing commands).

**Persistence**: `clarity = { on, db }`.

**Core dependencies**: the existing `treble` node; nothing new needed. (FFmpeg's `crystalizer` effect is closer to a crystalizer, but it is not required and is not requested this round.)

**Acceptance**: toggling with Shift+C gives an audible change in the high frequencies; the menu check, window checkbox and OSD agree; after upgrading from old settings, the values are correct.

**Notes**: the crystalizer has only one strength parameter (no separate bandwidth extension or harmonic boost).

### G16 Audio output device selection

> Schedule (rulings R-A2–R-A4): moved to round 3a (core size budget). Round 2 has no "Output Device ▸" menu; the output device drop-down on the window's "Output" page is grey, with the hover hint "Output device selection needs a core interface and will come in a later version". The round 3a interface drops `devicesRevision`; while the window is open, `audioDevices()` is polled every 5 seconds, and the device choice is saved across launches by the plugin.

**Requirement**:
- List the system's audio output devices (built-in speakers, headphones, AirPods, HDMI displays, USB sound cards, virtual devices, etc.), radio. The first item is "System Default" (the default, following the output device in the macOS "Sound" settings, and switching automatically when it is switched in the system).
- After choosing a specific device, the player's sound (including streams generated by plugins such as NES) comes out of that device, regardless of the system default.

**Entry points**: "Audio ▸ Output Device ▸ System Default · [each device name]" (radio); the "Output Device" drop-down on the window's "Output" page (200 wide; names that are too long are truncated with an ellipsis, with the full name on hover).

**Behavior details**:
- OSD "Audio output: MacBook Pro Speakers" / "Audio output: System Default".
- Device list: read once at activation; when the core reports a device list change (plug / unplug), it is read again and the submenu is rebuilt as described in reference §4 (order 512 unchanged).
- The selected device is unplugged: the core continues playing on the system default (no stop), the plugin shows the OSD "Output device disconnected, using System Default"; the menu's radio dot stays on the device the user chose, but that item is shown disabled with the reason "Device not connected". When the device is reconnected, the core switches back automatically, and the plugin shows the OSD "Audio output: <device name>".
- The device saved at startup does not exist: same as above, falling back to the system default, without an OSD (to avoid a message at every launch), and shown disabled in the menu.
- Switching devices does not pause or seek; a short silence of up to about 0.5 seconds is allowed (the audio output reopens).
- The switch request fails (device in exclusive use, etc.): OSD "Cannot switch to <device name>: <reason>", keeping the old device.
- Choosing a device does not need a source: it can be chosen with nothing playing, and media opened later use that device.
- Headless mode (tests) or the null backend (`ao=null`): the submenu only has a disabled "No output devices available".

**Persistence**: `device` (the device id given by the core; `null` means system default). The plugin hands it to the core at activation.

**Core dependencies**: [New core API]
```ts
player.audioDevices(): Promise<AudioDevice[]>                  // needs playback.read; mpv audio-device-list
interface AudioDevice { id: string; name: string }             // stable id (CoreAudio UID), name is the localized display name
player.setAudioDevice(id: string | null): Promise<void>        // needs playback.control; null = system default (mpv audio-device=auto)
interface PlayerState {
  readonly audioOutput: {
    requested: string | null;   // the device the user chose, null = system default
    active: string | null;      // the device actually in use, differs from requested after a fallback
    devicesRevision: number;    // increments when the device list changes; plugins re-read audioDevices() on it
  };
}
```
Why the core has to do it: the output device is held by the player's audio output, and plugins cannot open or switch it. The core also has to make the audio path of generated streams (`producer_audio.rs`, where cpal opens the system default device) use the same device, otherwise NES sound does not follow the choice; and it has to switch back to `requested` automatically when a disconnected device reappears.

**Acceptance**:
- After plugging in headphones, the submenu automatically gains a headphones item; after choosing it, sound comes out of the headphones while the system default is still the speakers.
- Unplug the selected USB sound card during playback: sound continues on the system default, with an OSD; after plugging it back in, it switches back automatically.
- With "System Default" chosen, switch the output in macOS Control Center: the player follows.
- Choose a device and restart the player: it still uses that device.
- While a plugin's generated stream (e.g. the VestraNES emulator from the store) runs, its sound also comes out of the chosen device.

**Notes**: pass-through and a second output device belong to G17 / are Windows-only, and are not done.

**Chapter core API request summary**

| Interface | Reason |
| --- | --- |
| `player.addAudioTrack(path: string, opts?: { select?: boolean; ifCurrent?: PlayerState }): Promise<AudioTrack>` | A11. Adding a track to the playback session is an operation inside the player (mpv `audio-add`) |
| `player.selectAudioTrack(id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>` (rewritten by ruling R-T2: `kind` removed) | G03, G04. Switching audio tracks is an operation inside the player (mpv `aid`) |
| `AudioNode` gains `{ kind: "channels"; mode: "stereo" \| "left" \| "right" \| "mono" \| "swap" }` | G05. The core handles multichannel downmix; plugins cannot add filters themselves |
| `player.audioDevices(): Promise<AudioDevice[]>` | G16. The device list comes from the player's audio output |
| `player.setAudioDevice(id: string \| null): Promise<void>` | G16. Covers both audio paths, mpv and generated streams (cpal) |
| `PlayerState.audioOutput: { requested: string \| null; active: string \| null }` (ruling R-A4: `devicesRevision` removed, round 3a) | G16. The plugin shows the fallback state from it; while the window is open, the device list is re-read every 5 seconds |
| Audio `delayMs` widened to −10000–10000 (provided in round 2) | G09 / A11. External audio tracks are often off by more than 2 seconds |

No core changes needed for: the G09 shortcuts, the G11 shortcuts, A11's automatic loading of same-name files (Node scans the folder).

## Capture (`capture`)

**Location**: `src/plugins/capture/` (existing plugin)

### Scope

The capture plugin saves playback content as image or video files: single screenshots, bursts, clip export. (2026-11-09: clip export left the built-in plugin for a store plugin; the built-in keeps frames and bursts. The clip rows below are the record of this round.) This round adds: copying the frame to the clipboard, a single "rendered frame" screenshot (with effects and subtitles), JPEG/BMP and quality, and opening the screenshot folder.

Boundaries and trade-offs:
- Frames are always taken with core snapshots (`media.capture` + `Snapshot.export`), so the freeze and conditional-submit semantics are the same as the existing screenshots.
- JPEG/BMP are exported directly by the core snapshot (`jpeg` / `bmp` of `Snapshot.export`, reference §10; since portability round P1, replacing the plugin's former macOS `sips` conversion). Copying to the clipboard uses `plugin.clipboard.writeImage`, opening the folder uses `plugin.system.openPath`, and the settings file uses `plugin.storage.json` (round 2 general capabilities, replacing the former `osascript`, `open` and hand-written settings I/O).
- Screenshot settings (format, quality) apply to all image output: single shots, save as, bursts.

The manifest adds the capability `clipboard` (F03); the rest already exist (`media.read`, `media.control`, `playback.read`, `subtitles.read`).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| F03 | Copy the current frame to the clipboard | ✗ | No (general capability `plugin.clipboard`) |
| F04 | Single screenshot with effects and subtitles | ◐ | No |
| F07 | JPEG/BMP format and quality | ◐ | Yes (`jpeg` / `bmp` of `Snapshot.export`, delivered in portability round P1) |
| F09 | Open the screenshot folder | ◐ | No (general capability `plugin.system`) |

### Menus and shortcuts

`Video ▸ Snapshot ▸` items 420–449 (the submenu's position is the first item's order).

| Menu path | order | Group | Default shortcut | ID |
| --- | --- | --- | --- | --- |
| Video ▸ Snapshot ▸ Save Current Frame | 420 | capture-source | Ctrl+E (existing) | |
| Video ▸ Snapshot ▸ Save Current Frame As… | 421 | capture-source | — | |
| Video ▸ Snapshot ▸ Copy Current Frame | 422 | capture-source | Ctrl+C | F03 |
| Video ▸ Snapshot ▸ Save Rendered Frame | 423 | capture-screen | Ctrl+Alt+E | F04 |
| Video ▸ Snapshot ▸ Save Rendered Frame As… | 424 | capture-screen | — | F04 |
| Video ▸ Snapshot ▸ Copy Rendered Frame | 425 | capture-screen | Ctrl+Alt+C | F03/F04 |
| Video ▸ Snapshot ▸ Burst… (checked while the window is open) | 426 | capture-export | Ctrl+G (existing) | |
| Video ▸ Snapshot ▸ Export Clip… | 428 | capture-export | — | |
| Video ▸ Snapshot ▸ Export Tasks… | 429 | capture-export | — | |
| Video ▸ Snapshot ▸ Capture as PNG / JPEG / BMP (radio) | 430–432 | capture-format | — | F07 |
| Video ▸ Snapshot ▸ Include Subtitles in Rendered Frame (checkbox) | 435 | capture-name | — | F04 |
| Video ▸ Snapshot ▸ Snapshot Settings… | 436 | capture-name | — | F07 |
| Video ▸ Snapshot ▸ Snapshot Folder… | 437 | capture-folder | — (existing, reordered) | |
| Video ▸ Snapshot ▸ Open Snapshot Folder | 438 | capture-folder | — | F09 |

Conflict check: Ctrl+C, Ctrl+Alt+E and Ctrl+Alt+C are not used now. C (faster 0.1) does not conflict with combinations that have modifiers. On macOS `Ctrl+C` also responds to ⌘C; the video area has no selectable text, so system copy is not affected; when an input box has focus the control takes the keys, so no screenshot is taken by mistake.

### F03 Copy the current frame to the clipboard

**Requirement**: copy the current frame to the system clipboard with one key (PNG image), ready to paste into chats and documents. There are "source frame" (Ctrl+C, the original frame) and "rendered frame" (Ctrl+Alt+C, with effects and subtitles, see F04).

**Entry points**: `Video ▸ Snapshot ▸ Copy Current Frame` (Ctrl+C), `Copy Rendered Frame` (Ctrl+Alt+C).

**Behavior details**:
1. Flow: `player.refresh()` records `seen` → `media.capture({ ifCurrent: seen, processed })` → `export({ format: "png", destination: <temporary directory>/clip-<n>.png })` → wait for completion → `plugin.clipboard.writeImage(<path>)` → delete the temporary file and release the snapshot.
2. Temporary directory: `os.tmpdir()/vestra-capture-<pid>/`, emptied when the plugin is disabled.
3. The clipboard only gets PNG (not affected by the screenshot format setting), at the original resolution.
4. Shares the "one at a time" mutual exclusion with Ctrl+E: pressing again before the previous one finishes shows the OSD "The previous snapshot is still being processed", without queuing. Reason: there is only one clipboard, so queuing makes no sense.
5. Success OSD: `Frame copied to the clipboard`. Failure: screenshot failures use the existing wording (`Snapshot failed: …`); clipboard write failure "Could not write to the clipboard".
6. Cannot capture (no media, network stream, `can.snapshot` false): disabled with the reason "No frame to capture" (existing `canCapture`).

**Core dependencies**: the existing `media.capture`, `Snapshot.export`; the round 2 general capability `plugin.clipboard.writeImage` (capability `clipboard`, replacing the former `osascript`).

**Acceptance**: press Ctrl+C during playback, then create a new image from the clipboard in an image application (macOS "Preview", Windows "Paint"): the image has the same size as from Ctrl+E; the temporary file has been deleted; the command is disabled with no media; pressing twice quickly gives a message on the second press.

### F04 Single screenshot with effects and subtitles

**Current state**: bursts and clip export can choose "include picture effects / capture subtitles too"; single screenshots (Ctrl+E, save as) only capture the source frame.

**Requirement**: single screenshots can also capture the "rendered frame": including the plugins' picture effects (brightness, contrast, blur, etc.) and (optionally) the current subtitles.

**Entry points**: `Video ▸ Snapshot ▸ Save Rendered Frame` (Ctrl+Alt+E), `Save Rendered Frame As…`, `Copy Rendered Frame` (Ctrl+Alt+C); `Include Subtitles in Rendered Frame` (checkbox, on by default).

**Behavior details**:
1. "Rendered frame" = `media.capture({ processed: true, subtitles: <checkbox> })`; the "source frame" stays `processed: false`.
2. Save location, format and file name are the same as for the source frame (F07); file names do not distinguish source / rendered.
3. "Include subtitles" is on by default (reason: the user's intent with "rendered frame" is what you see is what you get). When no subtitle is selected, there are naturally no subtitles, without an error.
4. Limitations (stated in the Snapshot Settings window): the rendered frame is produced by offline rendering, and subtitles are redrawn with the plugin style, so it is not pixel-identical to the screen; on-screen messages (OSD) are not included; subtitles are always drawn inside the picture.
5. OSD: on success the same as the existing "Frame saved as …"; when processing effects fails, the snapshot error wording is used.
6. With no effects and no subtitles, the result is the same as the source frame, without a message.

**Core dependencies**: the existing `media.capture({ processed, subtitles })` (needs `playback.read`, `subtitles.read`, already in the manifest).

**Acceptance**: with Gaussian blur on and subtitles shown, Ctrl+Alt+E gives a blurred image with subtitles; Ctrl+E gives a sharp image without subtitles; after unchecking "Include Subtitles in Rendered Frame", Ctrl+Alt+E has no subtitles.

### F07 Screenshot format JPEG / BMP, quality

**Current state**: PNG only.

**Requirement**: the screenshot format can be PNG (default), JPEG or BMP; JPEG quality 1–100, default 90.

**Entry points**: `Video ▸ Snapshot ▸ Capture as PNG / JPEG / BMP` (radio); "Format" and "Quality" in the `Snapshot Settings…` window; the format text in the burst window's "Other settings" group becomes a drop-down (sharing the same setting with the menu).

**Behavior details**:
1. Applies to: Save Current Frame, Save As, Save Rendered Frame, bursts. Copying to the clipboard is always PNG.
2. Flow: export directly to the target: `export({ format: "jpeg", destination, sampleTime, quality })`, `export({ format: "bmp", destination, sampleTime })`. The player converts from a validated PNG and publishes the target file only after a successful conversion; the intermediate file is deleted by the player (reference §10). When conversion fails, the job fails just like a failed PNG export (OSD "Snapshot failed: …", retryable in the task list), and no PNG is left behind.
3. Save As: the dialog's default file name has the current format's extension; when the user enters `.png/.jpg/.jpeg/.bmp`, the user's extension decides the format for this save (the setting does not change).
4. PNG by default (decision D3). Quality defaults to 90 (JPEG 100 files are too large and differ little from PNG, so 90).
5. Switching format OSD: `Snapshot format: JPEG`.

**Interface**: "Snapshot Settings" window (360 wide): group "Format": format drop-down, quality slider + value (JPEG only); group "Save location": screenshot folder (read-only + "…"), checkbox "Save to the media folder when possible" (off by default).

**Persistence**: `settings.json` gains `format` and `quality` (in the same file as the existing `folder`), read and written with `plugin.storage.json("settings")`: a corrupt file is renamed to `settings.json.bad-<timestamp>`; when a single field is invalid, only that field uses its default.

**Core dependencies**: `jpeg` (`quality` 1–100, default 90) and `bmp` of `Snapshot.export` (delivered in portability round P1, reference §10).

**Acceptance**: choose JPEG quality 80: Ctrl+E produces a `.jpg` (the content is JPEG, e.g. the `file` command reports JPEG image data), clearly smaller than the PNG; no temporary PNG is left in the screenshot folder; BMP works; Save As with `a.png` gives a PNG even when the setting is JPEG; a burst of 10 gives 10 JPEGs.

### F09 Open the screenshot folder

**Current state**: the screenshot folder can be set and is remembered; the burst window and task list have open buttons, but the menu has no "Open Snapshot Folder".

**Requirement**: open the screenshot folder in the system file manager (Finder on macOS, File Explorer on Windows) from the menu with one click.

**Entry point**: `Video ▸ Snapshot ▸ Open Snapshot Folder`.

**Behavior details**:
1. Uses `plugin.system.openPath(folder)`. When the folder does not exist, it is created first (consistent with the existing `openFolder`). "Open" in "Export Tasks" uses `openPath` too.
2. When "Save to the media folder when possible" (F07 setting) is on and the current source is a local file, the video's folder is opened.
3. Failure OSD "Could not open the folder." (existing wording).
4. Available with no media.

**Core dependencies**: none.

**Acceptance**: after running it, the file manager opens the folder; after the folder is deleted, running it creates the folder again and opens it.

### Chapter core API request summary

| Interface | Reason | Used by |
| --- | --- | --- |
| `{ format: "jpeg"; quality? }` and `{ format: "bmp" }` of `Snapshot.export` (delivered in portability round P1) | Plugins do not call system commands to convert images (community workflow §1.5); the player only publishes successfully converted files, so the plugin no longer needs a temporary PNG and fallback | F07 |

No general image conversion interface is requested (portability round ruling §4.3). Clipboard images, opening folders and settings files use the round 2 general capabilities instead (`clipboard`, `system`, `storage`).

## Playback info (`info`)

**Location**: `src/plugins/info/` (existing plugin)

### Scope

The info plugin shows the playback state to the user, read-only. Today it only has I02: the full info overlay opened with Tab. This round adds three items: brief info (I03), the playback properties dialog (I05) and About (I09).

Boundaries: info is read-only, does not control playback and does not call other plugins. The data in I05 and I09 that "only the core knows" (live decoding statistics, mpv/FFmpeg versions, the list of loaded plugins) has to come from the core. File-level information (container, bitrate, tags, attachments) is read by the plugin itself with ffprobe (`plugin.tools.run`). Copying to the clipboard uses `plugin.clipboard.writeText`.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| I03 | Brief info (file name, time, codecs), shown for 3 seconds per press | ✗ | None |
| I05 | Playback properties dialog (playback / file / system pages, copy, save) | ✗ | Extended `PlaybackStats` fields (round 2); version rows etc. from `plugin.app.info()` (3a) |
| I09 | About (version, mpv/FFmpeg versions, loaded plugins) — moved to 3a | ✗ | `plugin.app.info()`; image properties (the logo can use compile-time `@image-url`, nothing new needed) |

Manifest capabilities grow from `commands, playback.read, events, ui.osd` to: `+ ui.window` (I05 and I09 windows), `+ ui.dialog` (I05 "Save As"), `+ clipboard` (I05 and I09 copy).

### Menus and shortcuts

| Menu path | order | group | Default binding | Notes |
| --- | --- | --- | --- | --- |
| Playback Info (checkbox) | 810 | windows | Tab | Existing, unchanged |
| Playback Properties… | 811 | windows | Ctrl+F1 | I05 |
| About Vestra Player… | 880 | windows | F1 | I09 |
| (no menu) Brief Info | — | — | Shift+Tab | I03, `path` omitted, `title: "Brief Info"` |

Shortcut conflict check: Shift+Tab, Ctrl+F1 and F1 are not in [menus.md](menus.md). Two things to note on macOS: ⌃F1 is by default the system's "Turn keyboard access on or off" and may be taken by the system first; users can press ⌘F1 instead (a `Ctrl` binding accepts ⌘); on laptops F1 needs the fn key. I03 is not in the context menu, because it is normally used by key. If a menu entry is needed later, it will be reviewed together with the rest.

### I03 Brief info

> The content has been extended by IA-INFO-01 in the [interaction spec](interaction-spec.md): line 2 adds the current chapter after the time (the last chapter in `player.chapters()` whose start time is not later than the current position; "Chapter N" when it has no title); when a subtitle track is selected, line 3 shows the subtitle track name (title, otherwise language, otherwise "Subtitles"). Each line is truncated to the video area width (at least 44 columns). This note takes precedence over "2 lines" and "44 columns" below.

**Requirement**
- After Shift+Tab, 2 lines of brief info are shown in the info block at the top left of the video and disappear automatically after 3 seconds. Pressing again while shown restarts the 3 seconds.
- Line 1 is the file name, with the same rules as I02 (`fileName()`).
- Line 2 is `time / duration · video codec resolution · audio codec channels`, for example `01:23 / 45:00 · h264 1920×1080 · aac 2 ch`. When an item is unknown, the whole item is left out, without writing "-".
- While shown, the time refreshes once per second.

**Entry point**: Shift+Tab (no menu item).

**Behavior details**
- The plugin has only one `osd.panel()`. The info block text is concatenated in order: brief info (while shown), full info (while open). Blocks are separated by an empty line, at most 32 lines in total (in practice at most 2 + 8 = 10 lines).
- Brief info can be stacked while the full info (Tab) is open; neither closes the other.
- No media open: line 1 shows "No media open", and line 2 is left out.
- Live streams (`duration === null`): the time part shows `01:23 / Live`.
- When `player.stats()` fails, the previous text is kept and it retries the next second (consistent with I02).
- When the 3 seconds are up, only the brief info block is removed. When the full info is not open, `panel.set(null)` is called.
- No single-line on-screen message: the content is already in the info block.

**Interface**: uses the host info block (semi-transparent black background, 11px monospace, top left); the plugin cannot control the position or style.

**Persistence**: none.

**Core dependencies**: the existing `osd.panel()`, `player.state`, `player.stats()`.

**Acceptance**
- Press Shift+Tab while playing a local video: 2 lines appear at the top left, matching the current file; they disappear after 3 seconds.
- Press again at second 2 while shown: the disappearance moves from second 3 to second 5.
- Press Tab first to open the full info, then Shift+Tab: both blocks show, brief info on top; after 3 seconds only the full info remains.
- Press Shift+Tab with no media open: only "No media open" is shown.
- Unit test: after `host.press("Shift+Tab")`, `host.osdPanel` contains 2 lines; after advancing the timer by 3 seconds it is `null`.

**Notes**: brief info is a show-once action, not a toggle; the style is always the host info block.

### I05 Playback properties dialog

> Downgraded in round 2 (ruling R-P7, scheduling R-W6): `PlaybackStats` does not get `hwdec`; the "Playback" page has a separate "Hardware decoding" row that always shows "Not used" (the player always decodes in software), to be connected with C-GPU. **Connected in the render round**: `PlaybackStats.video.hwdec` (mpv `hwdec-current`) was delivered with GPU rendering, and the "Hardware decoding" row shows the truth: the hardware decoder in use is shown as is (e.g. `videotoolbox`); `"no"` (software decoding; currently always the case with software rendering and on Windows / Linux, and also for formats mpv cannot decode under GPU rendering) shows "Not used"; when the player does not report it (`null`), "-". `plugin.app` is delivered in 3a, so the Vestra Player, mpv and FFmpeg versions on the "System" page show "Unknown" for now (the rows are kept). The OS version, machine model and displays on the system page were at that time read by the plugin with macOS commands; since portability round P1 they come from `plugin.app.system()` (reference §3.5), CPU and memory from Node `os`, and the plugin no longer calls system commands.

**Requirement**
- Tool window "Playback Properties" with three tabs:
  1. **Playback** (live, refreshed every second)
     - Video group: decoder, hardware decoding (`video.hwdec`: e.g. `videotoolbox`, "Not used" for software decoding), input format (codec, pixel format, color space), size (source size → display size), source frame rate, current frame rate, video bitrate.
     - Audio group: decoder, sample rate, bits per sample (sample format), channel count (with layout), audio bitrate.
     - Other: backend, session, frames shown, frames dropped, A/V sync offset (milliseconds), buffer length (seconds).
  2. **File** (read once when opened, with a "Refresh" button)
     - Source: full path, file size, modification time.
     - Container: format name, total duration, total bitrate.
     - Per-stream information: type, index, codec, language, title, default/forced flags, the key parameters of each type.
     - Chapter count, and metadata tags (title, artist, album, etc.).
     - Embedded resources: file names and MIME types of attachment streams (fonts, cover art).
  3. **System**
     - Vestra Player version, plugin API version, mpv version, FFmpeg (libavcodec/libavformat) versions.
     - OS version (such as "macOS 26.2 (25C56)", "Windows 11 23H2 (22631.4460)"), machine model, CPU (model and core count), total and available memory, displays (one line each: name · pixel size · logical size @ refresh rate, such as "Color LCD · 2940 x 1912 · 1470 x 956 @ 60 Hz"; with several, numbered "Display 1", "Display 2").
- Bottom buttons: `Copy to Clipboard`, `Save As…`, `Close`. Copy and save use the plain text of the current tab, one `key: value` per line, with empty lines between groups.

**Entry point**: top level of the context menu "Playback Properties…" (811), shortcut Ctrl+F1.

**Behavior details**
- Once opened, the window stays open alongside playback and does not pause it (interaction requirements §3). Pressing Ctrl+F1 again while the window is open brings it to the front instead of closing it.
- With no media, the "Playback" and "File" pages show one line "No media open"; the "System" page is as usual.
- "File" only reads local files. When the source is a URL, it shows "Network source: only what the player reports" and lists what can be obtained from `player.state.source` and `stats()`.
- ffprobe runs with `plugin.tools.run("ffprobe", …)`, arguments `-v error -show_format -show_streams -show_chapters -of json <path>`, timeout 10 seconds. On failure or with no ffprobe, the page shows "Cannot read file information (ffprobe not found)" or "Cannot read file information: <one-sentence reason>".
- On source change (`sourceId` changes): "Playback" follows automatically; "File" is read again automatically, and the old ffprobe process is aborted.
- "Copy to Clipboard" uses `plugin.clipboard.writeText`. On success the on-screen message is `Playback info copied`; on failure the page's status line shows "Copy failed".
- "Save As…" calls `dialogs.saveFile({ fileName: "<file name> info.txt" })` and writes via temporary file plus rename. On success the message is `Saved to <file name>`. When the target exists, the system dialog confirms the overwrite.
- Each value is at most 512 characters; longer values are truncated with "…". The total property text is bound by the 64 KB string limit, split into lines by stream.

**Interface**
- Window 460×420, minimum 400×320. Tabs at the top 31px high (the same as the tab strip of the Picture Adjustments window).
- The content area is a read-only two-column table: left column labels 120px right-aligned, right column values selectable for copying. Group titles 11px bold. Scrolls when the content is long.
- Bottom button row: buttons 21px high, minimum width 66px, right-aligned, 10px from the content area.
- Uses `Theme` colors and the Button of `@vestra/widgets.slint`.

**Persistence**: the last opened tab is remembered in the `propertiesTab` field of `info.json`.

**Core dependencies**
- Existing: `player.stats()`, `dialogs.saveFile`, `windows.create`.
- [New core API] extend `PlaybackStats` (all new fields may be `null`; old fields unchanged):

  ```ts
  interface PlaybackStats {
    backend: string; session: number; framesShown: number; framesDropped: number | null;
    video: {
      codec; width; height; fps;                      // existing
      decoder: string | null;          // mpv video-codec
      hwdec: string | null;            // hwdec-current; "no" is recorded as null
      pixelFormat: string | null;      // video-params/pixelformat
      colorspace: string | null;       // video-params/colormatrix
      displayWidth: number | null; displayHeight: number | null; // video-out-params
      displayFps: number | null;       // estimated-vf-fps
      bitrate: number | null;          // video-bitrate, bit/s
    } | null;
    audio: {
      codec; sampleRate; channels;                    // existing
      decoder: string | null; format: string | null;  // audio-codec, audio-params/format
      channelLayout: string | null; bitrate: number | null;
    } | null;
    avSyncMs: number | null;           // avsync × 1000
    cacheSeconds: number | null;       // demuxer-cache-duration
  }
  ```

  Reason: these are live states that exist only while the player decodes (whether hardware decoding is in effect, current frame rate, A/V sync, buffer). ffprobe only sees the file, not the decoding session. Only mpv has these values, so only the core can forward them.
- [New core API · 3a] `plugin.app.info(): Promise<AppInfo>` (defined in I09). The Vestra Player, mpv and FFmpeg versions on the "System" page come from it, and show "Unknown" before 3a. OS name, version, build number, machine model and displays come from `plugin.app.system()` (portability round P1, reference §3.5, no capability needed; items the system does not report show "-"), and CPU and memory are read by the plugin with the Node `os` module.

**Acceptance**
- Play a 1080p H.264 + AAC file and press Ctrl+F1: the "Playback" page shows the decoder, 1920×1080, frame rate and sample rate; the "Hardware decoding" row is `videotoolbox` under GPU rendering on macOS, and "Not used" when started with `VESTRA_SOFTWARE_RENDER=1` or `VESTRA_HWDEC=no`.
- The "File" page lists all streams. An MKV with chapters shows the correct chapter count; an MKV with font attachments lists the font file names under "Embedded resources".
- Opening the window during playback does not pause or stutter; while dragging the seek bar, the "Playback" page still refreshes every second.
- After "Copy to Clipboard", pasting into a text editor gives plain text matching the tab.
- While playing a YouTube link, the "File" page shows the network source note and does not call ffprobe.
- After removing ffprobe from PATH, the "File" page shows "ffprobe not found", and the other two pages are normal.

**Notes**: "embedded media resources" are merged into the "File" page; "online information" is not done; "Playback" is a page of its own (video decoder, input/output format, source frame rate, size, current frame rate; audio decoder, sample rate, bits per sample, channel count; bitrates).

### I09 About

> Not done in round 2; moved to 3a (depends on `plugin.app.info` / `onChange`, `packages.list`).

**Requirement**
- Tool window "About Vestra Player", with:
  - The brand logo, 96×96.
  - The words "Vestra Player" (13px bold).
  - Version line: `Version 0.x.y (build 2026-10-20 abcdef0)`.
  - Component versions: plugin API `vestra/1`, mpv, FFmpeg (libavcodec / libavformat / libavfilter), Slint, Node (the Node the host uses for plugins).
  - List of loaded plugins: plugin id, version, origin (installed / development / trial), state (running / starting / failed). Sorted by id, scrollable.
- Buttons: `Copy Info` (puts all the text on the clipboard, for pasting into problem reports), `Close`.

**Entry point**: top level of the context menu "About Vestra Player…" (880), shortcut F1.

**Behavior details**
- While the window is open, the plugin list updates live: when a plugin is installed, disabled or fails, it refreshes within 1 second (subscribing to changes of `app.info`, or reusing J07's `packages.onChange`, see below).
- When a component version cannot be obtained, "Unknown" is shown and the row is not hidden.
- On a successful "Copy Info", the on-screen message is `Version info copied`.
- The full license text is not shown. Licenses are in the repository's `licenses/` and are not put in this window (there is no "terms and policies" page).

**Interface**: window 360×420, not resizable. The top part (logo, name, version) is centered; the middle is a two-column table of component versions (labels 100px); the bottom part is the plugin list (4 columns, row height 18px, at most 8 rows shown, scrolling beyond that); the bottom button row is right-aligned. The logo uses Slint's compile-time `@image-url("logo.png")`: at build time `src/assets/brand/logo.png` is copied into the info plugin package (a package resource), so no runtime image capability is needed.

**Persistence**: none.

**Core dependencies**
- [New core API]

  ```ts
  plugin.app.info(): Promise<AppInfo>                 // no capability needed: read-only public information
  plugin.app.onChange(listener: (info: AppInfo) => void): Disposable // plugin list or state changes
  interface AppInfo {
    version: string;            // Vestra Player version
    build: string | null;       // build date + short commit hash
    apiVersion: string;         // "vestra/1"
    components: { mpv: string | null; ffmpeg: Record<string, string>; slint: string | null; node: string | null };
    plugins: PluginInfo[];      // defined in "Plugin management" J07
  }
  ```

  Reason: the mpv and FFmpeg library versions are those of the libraries linked into the host, which the plugin process cannot see. The version of the ffmpeg executable on PATH is not necessarily the same as the libav version the host links. The list of loaded plugins is core runtime state, and plugins cannot query each other.

**Acceptance**
- Press F1 to open the window: the version matches `cargo pkgid` / the build metadata; the mpv version matches what is derived from `mpv_client_api_version`.
- The plugin list contains all currently running plugins (at least 10 product plugins). After disabling audio in J07, audio's state becomes "Disabled" within 1 second.
- After "Copy Info", pasting gives plain text containing the versions and the plugin list.
- The logo is clear on a dark background and not distorted.

**Notes**: only one page, including "loaded plugins"; the "shortcut list" is provided by J01. "Check for updates" (J10, restored 2026-11-01) adds an Updates row to this window (info 0.1.11): the latest release from `latest.json`, Release Notes, and Check Now / Download / Run Installer.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| Extended `PlaybackStats` fields (`decoder`, `pixelFormat`, `colorspace`, `displayFps`, `bitrate`, `avSyncMs`, `cacheSeconds`, etc.; `hwdec` not adopted, ruling R-P7) | Live decoding state exists only in mpv, and ffprobe cannot see it (delivered in round 2) |
| `plugin.app.info(): Promise<AppInfo>`; `plugin.app.onChange(listener)` | The versions of the libraries the host links and the list of loaded plugins are core state |

---

## Window (`window`)

**Location**: `src/plugins/window/` (created in round 3a; see the README in that directory for implementation notes)

### Scope

The window plugin owns the state of the main window on the desktop: setting the window to a multiple of the video size, fitting the video aspect ratio, resizing the window to the video size when playback starts (G-02), mini size, picture-in-picture (macOS), always on top, and hiding the mouse pointer during playback in windowed mode. It saves these preferences and restores them at startup, and switches on-top according to the playback state ("While Playing", "While Playing in Fullscreen").

Boundaries:
- The window itself (NSWindow level, size, pointer, fullscreen, picture-in-picture) is held by the Rust core; the plugin only sends requests. This chapter needs a set of `plugin.window` interfaces, exposed through the new capability `window.control`.
- Ordinary "Fullscreen" (Toggle Fullscreen, Esc; Enter / Leave Fullscreen) is still a host command; this plugin does not register it again.
- Saving the window size and position across launches is still the core's job (C8, `ui-state.json`); this plugin only saves the preferences for on-top, the pointer and the window size when playback starts (`settings.json`, shared by the "Options ▸ Playback ▸ Window" settings page and the menu items).
- The aspect ratio, cropping and zoom of the video picture belong to the video domain (other chapters); this chapter only changes the window, not the picture ratio.
- It does not read any state of other plugins; the video size comes from the core (3a: `video.displayWidth/Height` of `player.stats()`, see H03 item 1).

Manifest capabilities: `commands`, `events`, `playback.read`, `ui.osd`, `ui.settings`, `window.control`.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| B30 | Hide the mouse pointer during playback (windowed mode) | ✓ (3a) | Yes (`window.setCursorAutoHide`) |
| H03 | Window at 0.5 / 1 / 1.5 / 2× video size | ✓ (3a) | Yes (`window.setVideoArea`; the size comes from `stats()`, `PlayerState.videoSize` did not make 3a) |
| H04 | Fit to video aspect ratio | ✓ (3a) | Yes (same as above) |
| H05 | On top: never / always / while playing / while playing in fullscreen (default) | ✓ (3a; "while playing in fullscreen" in the interaction alignment round, IA-WIN-03) | Yes (`window.setAlwaysOnTop`) |
| H07 | Mini size (`` ` ``, all systems) and picture-in-picture (macOS, a separate command) | ✓ (render and picture-in-picture round; split by IA-WIN-01 and IA-WIN-02 in the interaction alignment round) | Yes (`window.setPictureInPicture`; the host implements picture-in-picture frame output); mini size only uses W2 |
| G-02 | Resize the window to the video size when playback starts (default 1×) | ✓ (interaction alignment round, see G-02 in interaction-spec.md) | No (`setVideoArea`, `stats()`) |

### Menus and shortcuts

| Menu path | order / group | Default shortcut | Notes |
| --- | --- | --- | --- |
| Screen ▸ Picture-in-Picture (checkbox; only shown when the system supports picture-in-picture, id `pip`) | 709 / screen | — | IA-WIN-01: an extra Vestra command with no default key. The submenu as a whole sits after the host's "Fullscreen" (700) and before the window entry group (800); the first item uses group `screen`, so no separator is drawn between the submenu and "Fullscreen" |
| Screen ▸ Mini Size (checkbox, all systems, id `mini`) | 710 / screen | `` ` `` | IA-WIN-01 |
| Screen ▸ 0.5× · 1× · 1.5× · 2× (radio, dot shown according to the current size) | 711–714 / screen-size | 1 · 2 · 3 · 4 | |
| Screen ▸ Fit to Video Aspect Ratio | 715 / screen-size | 8 | |
| Screen ▸ When Playback Starts ▸ Don't Resize · 0.5× · 1× · 1.5× · 2× · Fit to Video Aspect Ratio (radio, default 1×) | 716–721 / screen-auto | — | G-02 |
| Screen ▸ When Playback Starts ▸ Only Once (checkbox, off by default) | 722 / screen-auto-once | — | G-02; a separator between it and the radio items |
| Screen ▸ Always on Top ▸ Cycle Mode | 725 / screen-top | Ctrl+T | |
| Screen ▸ Always on Top ▸ Never · Always · While Playing · While Playing in Fullscreen (radio, default the last one) | 726–729 / screen-top-mode | — | A separator between them and "Cycle Mode"; IA-WIN-03 |
| Playback ▸ Hide Pointer While Playing (checkbox) | 241 / playback-options | — | Placed in the playback domain, in the same group as playback's 240, 242, 243 |

Conflict check: 1, 2, 3, 4, 8, `` ` `` and Ctrl+T are not used by any command now. On macOS a Ctrl binding can also be triggered with ⌘: ⌘T does not conflict with a system shortcut. ⌘` (cycle windows of the same app) is not affected, because mini size binds `` ` `` without modifiers. The digit keys 1–4 and 8 are global bindings; if another domain (for example aspect ratio presets) wants digit keys later, they have to be allocated together in menus.md.

---

### B30 Hide the mouse pointer during playback

Current state: hidden only in fullscreen, while playing, after 3 seconds idle (main.rs `cursor-hidden = fullscreen && idle && playing`).

> The default has been superseded by G-06 in the [interaction spec](interaction-spec.md): on by default; users who have already saved a setting keep their value.

**Requirement**: a toggle, on by default (G-06; originally off by default); when off, things stay as now. When on, windowed mode works the same way: while playing, if the pointer rests in the video area for 3 seconds, it is hidden.

**Entry point**: "Playback ▸ Hide Pointer While Playing" (checkbox).

**Behavior details**
1. Hide when:
   - The toggle is on, or fullscreen (as now).
   - `phase === "ready" && !paused`.
   - The pointer is in the video area; never hidden over the bottom bar, sidebar, floating panels, tool windows or menus.
   - At least 3 seconds since the last pointer move, key press or click, using the host's `IDLE_MS`.
2. Any pointer movement shows it again immediately.
3. Not hidden while a menu or system dialog is open (the host's obstruction rule).
4. Not hidden when the main window loses focus, because the pointer may be over another application's window.
5. On toggle, message "Hide pointer while playing: On / Off".
6. When the plugin is disabled, the core goes back to hiding only in fullscreen.

**Persistence**: `hideCursorWhilePlaying: boolean` in `plugin.dataDirectory/settings.json`, default `true` (G-06). `setCursorAutoHide` is called once at activation (only when on; the core default `"fullscreen"` is not sent, to avoid overwriting a value another plugin wrote last; with the default on, `"playing"` is sent at activation).

**Core dependencies** [New core API]:

```ts
window.setCursorAutoHide(mode: "fullscreen" | "playing"): Promise<void>
// "fullscreen": as now; "playing": hide in windowed mode too. Last write wins; back to "fullscreen" when the plugin is disabled
```

Reason: pointer visibility and the idle timer live in the host event loop, and plugins do not receive pointer events.

**Acceptance**
- Toggle on, playing in windowed mode: the pointer disappears after resting on the video for 3 seconds and reappears when moved.
- It does not disappear after pausing.
- It does not disappear while resting on the bottom bar.
- With the toggle off, it disappears only in fullscreen.

### H03 Window at 0.5 / 1 / 1.5 / 2× video size

**Requirement**: with one key, set the video area to 0.5, 1, 1.5 or 2 times the video's display size.

**Entry point**: "Screen ▸ 0.5× (1) / 1× (2) / 1.5× (3) / 2× (4)".

**Behavior details**
1. The video display size comes from `player.state.videoSize`: the display width and height after the container aspect ratio and rotation; for plugin simulator streams, the width and height given to `openStream`.
   - 3a implementation: the ruling did not provide `PlayerState.videoSize`, so `video.displayWidth/Height` of `player.stats()` is used instead (mpv `video-out-params/dw`, `dh`, after aspect ratio correction and filters), falling back to `video.width/height`. It is read when the source first becomes `ready`; while mpv has not provided it yet, it is re-read every 500 ms, at most 10 times; it is read again before each run (the picture may have been rotated). `video` being `null` or an empty size means "no video".
2. 1× = one video pixel per logical point (on Retina one point is 2 physical pixels, which looks the same as 1:1 on non-Retina; QuickTime's "Actual Size" on macOS does the same).
3. The target is the **video area**, not the whole window. The core adds the bottom bar (62px) and the attached sidebar (including the 3px divider) automatically.
4. The window's top-left corner stays in place. When it does not fit:
   - First move the window back into the visible screen area (excluding the menu bar and Dock).
   - If it still does not fit, shrink the video area proportionally until it fits, with the message "Window: 2× (shrunk to fit the screen)".
   - When enlarged to the minimum size (item 5), the message is "Window: 0.5× (enlarged to the minimum size)". Both are decided from the returned `clamped` and the actual size.
5. Below the minimum size (video area width 318 / window width 320, video height 120), the core enlarges to the minimum; when the attached sidebar does not fit, it is hidden by the existing rule (`reason: "space"`).
6. Radio dot: when the video area is within 1 point of a multiple, that item shows the dot; otherwise none do.
7. Unavailable (disabled with a reason):
   - No video (empty, audio only): "No video".
   - In fullscreen: "Not available in fullscreen". Fullscreen is not exited automatically, so pressing a digit key by mistake does not suddenly change the picture.
8. Message "Window: 1.5×".
9. Pressing again while a resize is in progress: the command is busy (reason "Resizing the window"), no queuing. When the core refuses, the message is "Cannot resize the window: <reason>".

**Core dependencies** [New core API]:

```ts
window.setVideoArea(size: { width: number; height: number }): Promise<{ width: number; height: number; clamped: boolean }>
// logical points; returns the video area size actually applied
// added to PlayerState
readonly videoSize: { width: number; height: number } | null;  // display size, including DAR and rotation
```

Reason: the window size is controlled by the host, and only the host knows how the bottom bar, sidebar and divider are laid out; `stats()` only has the decoded size, without the display aspect ratio, and has to be queried actively rather than pushed with the state.

**Acceptance**
- A 1920×1080 source on a 2560×1440 screen, press 2: the video area is 1920×1080.
- Press 4: shrunk until it fits, with the message "shrunk to fit the screen".
- A 720×576 (DAR 16:9) source, press 2: the video area is 1024×576.
- With the sidebar open, press 2: window width = video width + 3 + sidebar width.
- In fullscreen the menu items are grey.

**Notes**: the window is resized with its top-left corner as the anchor; there is no option to anchor at the center.

### H04 Fit to video aspect ratio

**Requirement**: resize the window so that the video area's aspect ratio equals the video's, removing black bars.

**Entry point**: "Screen ▸ Fit to Video Aspect Ratio", 8.

**Behavior details**
1. Keep the current video area width; height = width × `videoSize.height / videoSize.width`.
2. When the height exceeds the visible screen area, compute the width from the available height instead. If it is still below the minimum size, H03 item 5 applies (there will be black bars at the minimum size).
3. When it already fits (within 1 point), nothing changes and the message is "The window already fits the video aspect ratio"; otherwise "Window: Fit to video aspect ratio".
4. Unavailable in the same cases as H03 (no video, fullscreen).
5. It resizes only once and does not follow source changes automatically. Automatic resizing belongs to the "playback window size" setting, handled by "Screen ▸ When Playback Starts ▸" (superseded by G-02 in interaction-spec.md: by default each new source is resized to 1×).

**Core dependencies**: `window.setVideoArea`, `PlayerState.videoSize`, `window.state.videoArea` (same as H03).

**Acceptance**: a 16:9 source in an 800×800 video area, press 8: the video area becomes 800×450 with no black bars at the top or bottom; the same for a 2.39:1 source; pressing again only shows the "already fits" message.

### H05 On top: always / while playing / while fullscreen

> Item 6 and the default have been superseded by IA-WIN-03 in the [interaction spec](interaction-spec.md): four modes, default "While Playing in Fullscreen"; mini size is no longer on top (IA-WIN-02).

**Requirement**: choose how the main window stays on top: never, always, while playing, while playing in fullscreen (default). When on top, the main window floats above other applications' windows.

**Entry points**
- "Screen ▸ Always on Top ▸ Never · Always · While Playing · While Playing in Fullscreen" (radio); the same radio group on the "Options ▸ Playback ▸ Window" settings page.
- "Screen ▸ Always on Top ▸ Cycle Mode", Ctrl+T, cycling never → always → while playing → while playing in fullscreen → never.

**Behavior details**
1. Always: call `setAlwaysOnTop(true)` immediately.
2. While playing: on top when `phase === "ready" && !paused`; cancelled on pause, end, close or error; follows immediately when switching back and forth.
3. When the core puts the window on top, the main window and the floating sidebar window use the same floating level. Tool windows opened while on top use the same level too, so they are not hidden behind the main window. System file dialogs are not affected.
4. Messages: "Always on top: Always", "Always on top: While playing", "Always on top: While playing in fullscreen", "Always on top: Off".
5. When the plugin is disabled, the core cancels on-top.
6. While playing in fullscreen: on top when `window.state.fullscreen && phase === "ready" && !paused`, otherwise not; follows `window.onChange` and the playback state. macOS fullscreen uses a separate Space where the main window is already frontmost, so this mode has no visible effect on macOS; it is still offered and is the default, so that the settings are the same on both systems; on Windows the effect is visible. (The former item 6 "not offered" has been superseded by IA-WIN-03.)

**Persistence**: `onTop: "never" | "always" | "playing" | "fullscreen"` in `settings.json`, default `"fullscreen"`; values already saved are kept.

**Implementation**: actual on-top = always, or (while playing and playing), or (while playing in fullscreen and fullscreen and playing); `setAlwaysOnTop` is called only when this value changes, and nothing is sent at activation when the value is "not on top" (so a value another plugin wrote last is not overwritten). Mini size takes no part (IA-WIN-02).

**Core dependencies** [New core API]:

```ts
window.setAlwaysOnTop(onTop: boolean): Promise<void>   // back to false when the plugin is disabled
```

Reason: the window level (NSWindow level) can only be set by the host.

**Acceptance**
- After choosing "Always", click Finder: the player stays above the Finder window.
- After choosing "While Playing", pause: Finder can cover the player; after resuming, the player comes back on top.
- Ctrl+T cycles through the four modes with the correct message text.
- On first launch the dot is on "While Playing in Fullscreen"; while playing in windowed mode, Finder can cover the player.
- The mode is kept after a restart.

**Notes**: there is no "Other" menu; it goes under "Screen".

### H07 Picture-in-picture (replaces mini size)

> **Superseded by IA-WIN-01 and IA-WIN-02 in the [interaction spec](interaction-spec.md) (interaction alignment round)**: `` ` `` is "Screen ▸ Mini Size" on all systems (id `mini`, order 710), and mini size is no longer on top; "Screen ▸ Picture-in-Picture" is a separate command (id `pip`, order 709, no default key), registered only when `pictureInPicture.supported` is true. The two are mutually exclusive: before entering picture-in-picture, the size from before mini size is restored first; running mini size during picture-in-picture exits picture-in-picture first. The one-or-the-other form and the "replaces" relation below are void; the behavior rules of picture-in-picture itself are unchanged.
>
> **Implemented (render and picture-in-picture round, 2026-10)**. The window plugin chooses one of the two by `window.state.pictureInPicture.supported` and follows `window.onChange` (the host reports `true` only after the main window has been created, so it is usually still `false` when the plugin activates; on change the same command switches title, check and action, and when switching to picture-in-picture while mini size is in effect, the original size and on-top mode are restored first). Menu position 710 / group `screen`, shortcut `` ` `` and command id `mini` are the same on all platforms:
>
> | Platform | Menu item | Behavior |
> | --- | --- | --- |
> | macOS 14+ (`supported: true`) | "Screen ▸ Picture-in-Picture" (checkbox) | The requirement below: `setPictureInPicture(!active)`, the check follows `state.pictureInPicture.active` (`onChange`). Disabled with no video ("No video"; can still exit when already in picture-in-picture); available in fullscreen (the core exits fullscreen first). Success message "Picture-in-picture: On / Off", failure "Cannot enter picture-in-picture: <reason>", `busy` ignored, the command busy while switching ("Switching picture-in-picture"). When the small window's "back" button, close button or the core closes it, "Picture-in-picture: Off" is shown once, or "Picture-in-picture: Off (no video)" when the source no longer has video. Does not change the window size and does not put it on top. |
> | Windows, Linux, headless (`supported: false`) | "Screen ▸ Mini Size" (checkbox) | Falls back to "3a form: mini size" below (`setVideoArea` + `setAlwaysOnTop`), behavior unchanged. When the host later provides its own small window on these platforms and `supported` becomes `true`, the plugin switches to picture-in-picture automatically without code changes. |
>
> Differences from the text below: item 1's "picture-in-picture is not supported in this version" never appears, because without support it is simply mini size; "has video" is decided by the video size from `player.stats()` (`PlayerState.videoSize` is not provided, the same as H03).

Current state (when the item was opened): no mini size and no picture-in-picture. macOS has system picture-in-picture, whose floating window can sit above fullscreen apps and follows the user across Spaces, with position and size managed by the system; this fits macOS habits better than shrinking the main window to its minimum. So this item replaces mini size with picture-in-picture and no longer offers "shrink the main window".

**Requirement**
- A toggle: after entering picture-in-picture, the video keeps playing in the native macOS picture-in-picture floating window; the main window stays, and its video area shows the centered grey text "Playing in picture-in-picture". Running it again, or clicking the "back" button on the floating window, exits picture-in-picture and the picture returns to the main window.
- The play / pause button on the floating window controls the player and shares one playback state with the bottom bar and the Space key.
- The floating window's position, size and corner snapping are managed by the system; the plugin neither sets nor saves them.

**Entry point**: "Screen ▸ Picture-in-Picture" (checkbox), `` ` ``.

**Behavior details**
1. Available when (otherwise disabled with a reason):
   - There is video (`player.state.videoSize` non-empty); audio only or idle gives "No video".
   - The host supports it (`window.state.pictureInPicture.supported`); otherwise "Picture-in-picture is not supported in this version".
2. Enter: call `window.setPictureInPicture(true)`. On success the message is "Picture-in-picture: On" (drawn on the main window; the floating window has no OSD); on failure "Cannot enter picture-in-picture: <reason>", and the check does not change.
3. Run in fullscreen: the core exits fullscreen first, then enters picture-in-picture.
4. Ways to exit: press `` ` `` again or use the menu; the floating window's "back" button (does not pause); the floating window's close button (the core pauses playback at the same time, consistent with Safari and QuickTime). When the plugin sees `pictureInPicture.active` become `false` in `window.onChange`, it updates the check and shows "Picture-in-picture: Off".
5. Source change: picture-in-picture stays, and the floating window shows the new source's picture. When the new source has no video, the core exits picture-in-picture and the plugin shows "Picture-in-picture: Off (no video)". Closing the source exits as well.
6. The floating window's content is the frames mpv outputs, including plugin picture effects and subtitles rendered by mpv; the on-screen messages, info block and bottom bar that the host draws with Slint are not in the floating window.
7. During picture-in-picture the main window can be minimized or covered, and the floating window keeps playing.
8. Repeated triggers: a press before the previous `setPictureInPicture` has returned is ignored (the command is `busy`).
9. When the plugin is disabled, the core exits picture-in-picture.

**Persistence**: none.

**Core dependencies** [New core API]:

```ts
window.setPictureInPicture(on: boolean): Promise<void>
// added to WindowState
readonly pictureInPicture: { supported: boolean; active: boolean };
// supported: the host implements picture-in-picture and the OS version qualifies; active: the floating window is showing. The core exits picture-in-picture when the plugin is disabled
```

Reason: picture-in-picture can only be started by the host process through AVKit's `AVPictureInPictureController`; the floating window belongs to the host's windows and layers, and the video frames exist only in the host. Plugins cannot do it.

Host work and feasibility (libmpv currently renders in software, `vo=libmpv`):
- The player is not an `AVPlayer`, so only the sample buffer path is possible: the host creates an `AVSampleBufferDisplayLayer`, creates the controller with `AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer:playbackDelegate:)` (available since macOS 12), and implements `AVPictureInPictureSampleBufferPlaybackDelegate` (play / pause, playable time range, whether paused, floating window size changes). mpv's own macOS picture-in-picture only belongs to its own `vo=mac` / `vo=gpu` window and is not available for `vo=libmpv`.
- Frame source: software rendering already produces each frame as RGB pixels in CPU memory. During picture-in-picture, the host copies each frame into a `CVPixelBuffer` (RGB24 to BGRA), wraps it in a `CMSampleBuffer` with a presentation timestamp, and enqueues it to the `AVSampleBufferDisplayLayer`. This is doable, at the cost of one copy and format conversion per frame (about 2 MB/frame at 960×540).
- Limitation 1: the floating window's picture has the resolution of the render buffer. That is fixed at 960×540 now, so non-16:9 sources get black bars and the picture is blurry when the floating window is enlarged. We suggest determining the render size separately from the video display ratio during picture-in-picture (long side at most 1280); this is the same kind of change as C-SIZE (render size follows the video area).
- Limitation 2: during picture-in-picture the main window's video area shows no picture, but the host must keep driving mpv rendering at the video frame rate; when the main window is minimized or covered, the system may lower its refresh rate, so rendering must not depend on the main window's draw callback.
- Limitation 3: A/V sync is guaranteed by the host's timestamps. Frames must carry the correct presentation time (mpv's playback time converted to `CMTimebase`), otherwise the floating window's picture drifts from the sound.
- After C-GPU (libmpv switches to GPU rendering), `CVPixelBuffer`s backed by IOSurface can be enqueued directly, saving the copy. C-GPU is not a prerequisite for this item.
- If the host's minimum OS version is below macOS 12, `supported` is `false` on older systems.

**Acceptance**
- Press `` ` `` while playing a video: the system picture-in-picture floating window appears and keeps playing, and the main window's video area shows "Playing in picture-in-picture"; switch to another fullscreen app: the floating window stays on top.
- Pause on the floating window: the bottom bar button changes to paused accordingly; press Space to resume, and the floating window follows.
- Click the floating window's "back": the picture returns to the main window without interrupting playback, and the menu item is unchecked. Click the floating window's close button: picture-in-picture exits and playback pauses.
- In an audio-only file the menu item is grey, with the reason "No video".
- Play a 1080p source in picture-in-picture for 1 minute: the dropped frame rate is below 1% (`player.stats().framesDropped`), and the A/V sync offset is within ±40 ms (`avSyncMs` from I05).
- When the window plugin is disabled, the floating window closes.

#### 3a form: mini size (decided by author A, 2026-10-21; on all systems since the interaction alignment round, no longer on top)

3a provides mini size with the existing W2 (`setVideoArea`) and W3 (`setAlwaysOnTop`), taking picture-in-picture's menu position and key. After picture-in-picture was delivered (render and picture-in-picture round), mini size appears in the same position only when `pictureInPicture.supported` is `false` (Windows, Linux, headless); the two never appear together.

- Entry point: "Screen ▸ Mini Size" (checkbox), `` ` ``, order 710 / group `screen`.
- Enter: the video area shrinks to the largest same-ratio size that fits in 400×300 (16:9 → 400×225, 4:3 → 400×300; portrait 9:16 → 169×300, which the core enlarges to the minimum width 318, with black bars), keeping the window's top-left corner. Not on top; whether it is on top is decided only by the H05 mode (IA-WIN-02; the former "also on top" is void). Message "Mini size: On".
- Exit: press again, and the video area returns to the size before entering; message "Mini size: Off". When the size cannot be restored in fullscreen, mini size simply ends.
- Ends automatically (without restoring the old size, without a message): the user drags to change the window (the video area differs from mini size by more than 1 point), enters fullscreen, or runs an H03 multiple or H04 fit.
- Unavailable: in fullscreen "Not available in fullscreen"; no video "No video" (can still exit when already in mini size); busy while a resize is in progress.
- Not saved; after a restart it is not in mini size. When the plugin is disabled, the window keeps its size at that moment.

Acceptance: on a 1080p source press `` ` ``: the video area is 400×225; with the on-top mode "Never", clicking Finder covers the player; pressing again returns to the original size; dragging the window larger during mini size removes the check.

### Chapter core API request summary

The new capability `window.control` exposes the following interfaces. All setters are last-write-wins; when a plugin is disabled, the core withdraws the effects that plugin caused (consistent with the withdrawal rule for filters). The core does not save these preferences; the plugin reapplies them at activation.

```ts
interface WindowState {
  readonly fullscreen: boolean;
  readonly videoArea: { width: number; height: number };    // current video area, logical points
  readonly onTop: boolean;
  readonly pictureInPicture: { supported: boolean; active: boolean };
}
```

| Signature | Reason | Source |
| --- | --- | --- |
| `window.state: WindowState`; `window.onChange(listener): Disposable` | The plugin needs to know fullscreen, video area size and picture-in-picture state to decide whether commands are available and checked | H03–H05, H07 |
| `window.setVideoArea(size: { width; height }): Promise<{ width; height; clamped: boolean }>` | The window size is controlled by the host; only the host knows how the bottom bar and sidebar are laid out and how large the visible screen area is | H03, H04 |
| `PlayerState.videoSize: { width; height } \| null` | The display size (including DAR, rotation, stream size) has to be pushed with the state; `stats()` only has the decoded size | H03, H04, H07 |
| `window.setPictureInPicture(on: boolean): Promise<void>` | macOS picture-in-picture can only be started by the host through AVKit (`AVPictureInPictureController` + `AVSampleBufferDisplayLayer`), and the frames exist only in the host | H07 |
| `window.setAlwaysOnTop(onTop: boolean): Promise<void>` | The window level can only be set by the host; it applies to the floating sidebar and newly opened tool windows as well | H05 |
| `window.setCursorAutoHide(mode: "fullscreen" \| "playing"): Promise<void>` | Pointer visibility and the idle timer live in the host event loop | B30 |

---

## Playback history (`history`)

**Location**: `src/plugins/history/` (new plugin, to be created)

### Scope

history records "what the user opened and how far they watched": the recent playback menu (A12), and remembering the playback position to resume next time (B25). It does not care who opened a source: sources opened from the bottom bar, the playlist, site plugins or drag and drop are all only seen and recorded through `player.state`. Reopening calls `player.open`.

Boundaries: history cannot communicate with playlist. After a file is reopened from "Recently Played", if the user has set A13 "Add All Files in the Folder" in playlist, playlist adds the whole folder itself — the two plugins each act on the core state without knowing about each other.

Manifest capabilities: `playback.read`, `playback.control`, `events`, `commands`, `ui.window`, `ui.osd`, `ui.dialog`. This plugin **needs no new core API**.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| A12 | Recent playback history | ✗ | No |
| B25 | Remember playback position | ✗ | No |

### Menus and shortcuts

| Menu path | order | group | Default binding | ID |
| --- | --- | --- | --- | --- |
| Open ▸ Recently Played ▸ <at most 20 entries> | 114 | recent | | A12 |
| Open ▸ Recently Played ▸ Remove Invalid Entries | 114 | recent-manage | | A12 |
| Open ▸ Recently Played ▸ Clear Entries | 114 | recent-manage | | A12 |
| Open ▸ Recently Played ▸ Record Playback History (checkbox) | 114 | recent-manage | | A12 |
| Playback ▸ Resume Playback ▸ Don't Resume / Resume Automatically / Ask Every Time (radio) | 250–252 | domain | | B25 |
| Playback ▸ Resume Playback ▸ Also Remember Audio Files (checkbox) | 253 | resume-options | | B25 |
| Playback ▸ Resume Playback ▸ Clear All Saved Positions | 254 | resume-options | | B25 |

Notes:

- All items under "Open ▸ Recently Played" use order 114 and are ordered by declaration (reference §4: equal order follows the declaration order of the same plugin), so the submenu sits after "Open Subtitle…" (113).
- "Playback ▸ Resume Playback ▸" uses 250–254, after playlist's 230–249; 250–259 has to be registered to history in the allocation table of menus.md.

### A12 Recent playback history

**Requirement**: the context menu "Open ▸ Recently Played" lists the files and links opened recently (newest on top); clicking an item reopens it; the history can be cleared and recording can be turned off.

**Entry points**: see the menu table.

**Behavior details**:

1. **When to record**: in `player.onChange`, record once when a `sourceId` first reaches `phase === "ready"` with `kind === "media"` and a non-empty `source`. Sources with `phase === "error"` are not recorded. `kind === "stream"` sources such as simulators are not recorded. Replays of the same `sourceId` are not recorded again.
2. **Identity**: local files use `realpath` (the original path when that fails); URLs use the literal text with the `#` fragment removed. If it already exists, it moves to the top and its time is updated.
3. **Capacity**: at most 100 entries are saved and the first 20 are shown in the menu; beyond 100, the oldest are discarded.
4. **Menu item text**: `<number 1–9, 0, then none>  <display name>`, followed by `(12:34)` when there is a B25 saved position. Display name: the file name for local files; `host › last path segment` for URLs (decoded, at most 60 characters, shortened in the middle). When a URL carries credentials (`user:pass@`), the display name leaves them out, while storage keeps the original. Menu text is at most 80 characters.
5. **Click**: `player.open(source)` (unconditional, started by the user). When a local file does not exist (checked with `fs.access` before opening), it is not opened, OSD "File not found: <file name>", and the entry is kept (it can be cleaned up with "Remove Invalid Entries"). Open failure (`backend`, etc.): OSD "Cannot open: <reason>".
6. **Menu rebuild**: after the entries change, the child scope is released and rebuilt with the new content (as in reference §4); several changes within 500 ms are merged.
7. **Remove Invalid Entries**: check all local entries concurrently and delete those that do not exist; URLs are kept. OSD "Removed N invalid entries".
8. **Clear Entries**: clears directly (no confirmation), OSD "Playback history cleared". B25 saved positions are not cleared (there is a separate command).
9. **Record Playback History** (on by default): when off, no new entries are added and existing entries are kept; the menu still shows them. Privacy: the history file is only in the local data directory.
10. With no entries, the submenu shows one disabled item "(No Entries)".

**Persistence**: `<dataDirectory>/recent.json`: `{ version: 1, enabled: true, items: [{ key, source, kind: "file"|"url", openedAt }] }`, read and written with `plugin.storage.json("recent")`, written 1 second after the last change (temporary file plus rename). File permissions follow the system default (after the round 2 migration to `storage`, 0600 is no longer set separately; the data directory is under `~/Library`, which only this user can enter). A corrupt file is renamed to `.bad-<timestamp>`.

**Core dependencies**: the existing `player.state`, `player.onChange`, `player.open`, `commands.add` + child scope rebuild.

**Acceptance**:
- Open a, then b, then a again from the bottom bar: the menu order is a, b.
- Play c from the playlist (opened by playlist): c also appears at the top (recording does not distinguish who opened it).
- Delete b's file and click b: OSD "File not found: b.mkv", not opened; after "Remove Invalid Entries" b is gone.
- Turn "Record Playback History" off and open d: d is not in the menu.
- The menu is kept after a restart.

**Notes**: recent playback goes in "Open ▸ Recently Played", consistent with the macOS "Recent Items" habit.

### B25 Remember playback position

**Requirement**: a file stopped partway continues from the last position the next time it is opened; optionally ask every time.

**Entry point**: "Playback ▸ Resume Playback ▸": Don't Resume / **Resume Automatically** (default) / Ask Every Time; "Also Remember Audio Files" (off by default); "Clear All Saved Positions".

**Behavior details**:

1. **Scope**: sources with `kind === "media"` and a known `duration` of ≥ 60 seconds (live streams have an empty `duration` and are not recorded). Video always applies; audio (decided by the media extension table; for extensions not in the table, by `player.stats()`, where `video` being `null` means audio) only applies when "Also Remember Audio Files" is on.
2. **Recording**: track the last known `position` of the current source:
   - Write the saved position every 5 seconds during playback, on pause, before `sourceId` changes (using the last position seen before the change), and when the plugin is disabled;
   - Position < 10 seconds: not recorded (and an existing saved position is deleted, treated as starting from the beginning);
   - Position > `duration − 15 seconds` or ≥ 95%, or a natural end (`endReason === "natural"`): delete the saved position (watched to the end).
   - Identity is the same as A12 (realpath / URL). The file size is saved too; if the size differs at the next open (the file was replaced), the saved position is void.
3. **Restore** (mode "Resume Automatically"): when a new `sourceId` first reaches `phase === "ready"` and a valid saved position exists: `player.seek(<saved position> − 2 seconds, { ifCurrent: readyState })` (2 seconds back to pick up the story, minimum 0). OSD: "Resuming from 12:34".
   - `stale` (after ready, the user or another plugin has already seeked, paused or changed the source): give up, no message.
   - Replays of the same `sourceId` (`restart`, for example playlist loop one) are not restored.
   - The saved position is beyond `duration − 15 seconds` (the file was shortened, etc.): not restored, and deleted.
4. **Ask Every Time**: after ready, playback starts from the beginning as usual, and a small window `ui/resume.slint` (300×110) opens: "You didn't finish this last time. Continue? (12:34)" [Continue] (default button) [Start Over]. Clicking "Continue" seeks as in item 3 (`ifCurrent` uses the ready state from when the window opened; if the source has changed meanwhile, it is `stale` and the window closes without a message). After 10 seconds without action, or when the source changes, the window closes automatically, which counts as starting over. Playback is not paused (interaction-requirements §3: showing a tool interface should not pause on its own).
5. **Don't Resume**: nothing is read or written; existing saved positions are kept (they apply again after switching the mode back).
6. **Clear All Saved Positions**: clears them, OSD "All saved positions cleared".
7. Capacity: at most 1000 entries, LRU (by last update time).
8. Known limitations: restoring seeks after ready, so about one frame of picture and a very short sound may flash at the start; after B26 fast-advances to the next item, if that item has a saved position, it seeks after the switch (at the correct position).
9. The `(12:34)` on A12 menu items is taken from this table.

**Persistence**: `<dataDirectory>/positions.json`: `{ version: 1, mode: "auto"|"ask"|"off", audio: false, items: { [key]: { position, duration, size, updatedAt } } }`; read and written with `plugin.storage.json("positions")`; saved positions are updated every 5 seconds, written 300 ms after content changes, and fully written when the plugin is disabled.

**Core dependencies**: the existing `player.onChange` (`position`, `duration`, `phase`, `endReason`, `sourceId`), `player.seek({ ifCurrent })`, `player.stats()`, `windows.create`, `osd.show`, `plugin.dataDirectory`. Nothing new: restoring seeks conditioned on the ready state, and when it conflicts with other plugins' actions it yields naturally through `stale`.

**Acceptance**:
- Play a 10-minute video to 05:00 and close it, then open it from any entry point (bottom bar, list, recent playback): it starts at 04:58, OSD "Resuming from 05:00".
- Close at 9:50 (10 seconds before the end): opening again starts from the beginning, and the saved position has been deleted.
- A 30-second short clip: not remembered.
- mp3 (default settings): not remembered; remembered after turning "Also Remember Audio Files" on.
- "Ask Every Time" mode: after opening, it plays from the beginning and the window pops up; clicking "Continue" jumps to the saved position; after 10 seconds without action the window closes and playback continues from the beginning.
- After opening, the user presses → to seek before ready: no restore (`stale`).
- Restart after force-quitting the process: at most 5 seconds of position are lost.

**Notes**: going back 2 seconds, counting 95% as watched and not recording under 60 seconds are thresholds we defined.

### Chapter core API request summary

None. All features of this plugin are done with existing SDK v2 interfaces.

---

## Chapters, bookmarks and ranges (`marks`)

**Location**: `src/plugins/marks/` (created in round 3a: B21, B23; B22 in 3b)

### Scope

The marks plugin owns the "points" and "ranges" on the playback timeline: the media's own chapters (read-only), the user's bookmarks (add, delete, edit, persisted per file), and A-B range loops. It provides menus, shortcuts and the "Chapters and Bookmarks" list window.

Boundaries:
- **Chapter data** comes from the media and is read from mpv by the core; round 3a implements it as the on-demand `player.chapters()` (not in `PlayerState`), which marks reads once when each source first becomes ready.
- **A-B loops** are run by the core (mpv `ab-loop-a/b`). The plugin keeps the start point, end point and toggle, and decides when to submit them to the core.
- **Seek bar marks (B22)**: the seek bar belongs to the playback plugin; marks cannot draw on it directly or call playback. This document chooses **a general "timeline mark layer" provided by the core**: each plugin can publish one layer of marks to the core, which merges them and makes them readable by all plugins. marks publishes chapter and bookmark marks, and playback reads the merged result and draws it on the seek bar. Reasons in B22.

Manifest capabilities: `commands`, `playback.read`, `events`, `playback.control`, `ui.window`, `ui.osd`, `ui.dialog` (confirmation for clearing bookmarks); 3b adds `timeline.marks` (new capability).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| B21 | Chapter / bookmark menu and jumping, bookmark management | ✓ 3a | `player.chapters()` (already in 3a) |
| B22 | Chapter / bookmark markers on the seek bar | ✗ | Timeline mark layer `plugin.timeline.*`; playback draws them |
| B23 | A-B range loop (set points, fine-tune, toggle, loop the current range) | ✓ 3a | `player.setLoop()`, `PlayerState.loop` (already in 3a) |

### Menus and shortcuts

New items in the "Playback" submenu. playback already uses 201–226, playlist 230–249, window 241, history 250–254 (Playback ▸ Resume Playback); marks uses 260–289.

| Menu path | order | group | Default binding | ID |
| --- | --- | --- | --- | --- |
| Playback ▸ Chapters and Bookmarks ▸ Chapter and Bookmark List… | 260 | marks-list | H | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Add Bookmark | 261 | marks-edit | P | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Previous Bookmark or Chapter | 262 | marks-edit | Shift+PageUp | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Next Bookmark or Chapter | 263 | marks-edit | Shift+PageDown | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Previous Chapter | 264 | marks-edit | Ctrl+PageUp | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Next Chapter | 265 | marks-edit | Ctrl+PageDown | B21 |
| Playback ▸ Chapters and Bookmarks ▸ (dynamic) chapters 1…N (radio, the current chapter selected) | 10000 + i (numbering inside the submenu, see B21) | marks-chapters | | B21 |
| Playback ▸ Chapters and Bookmarks ▸ (dynamic) bookmarks 1…N | 20000 + i | marks-bookmarks | | B21 |
| Playback ▸ Chapters and Bookmarks ▸ Show Marks on Seek Bar (checkbox) | 30000 | marks-option | | B22 (3b) |
| Playback ▸ A-B Loop ▸ Set Start Point | 270 | ab-a | `[` | B23 |
| Playback ▸ A-B Loop ▸ Clear Start Point | 271 | ab-a | `{` (Shift+[) | B23 |
| Playback ▸ A-B Loop ▸ Start Point Back 0.1 s | 272 | ab-a | Ctrl+[ | B23 |
| Playback ▸ A-B Loop ▸ Start Point Forward 0.1 s | 273 | ab-a | Alt+[ | B23 |
| Playback ▸ A-B Loop ▸ Set End Point | 274 | ab-b | `]` | B23 |
| Playback ▸ A-B Loop ▸ Clear End Point | 275 | ab-b | `}` (Shift+]) | B23 |
| Playback ▸ A-B Loop ▸ End Point Back 0.1 s | 276 | ab-b | Ctrl+] | B23 |
| Playback ▸ A-B Loop ▸ End Point Forward 0.1 s | 277 | ab-b | Alt+] | B23 |
| Playback ▸ A-B Loop ▸ Move Range Back 0.1 s | 278 | ab-both | Ctrl+Alt+[ | B23 |
| Playback ▸ A-B Loop ▸ Move Range Forward 0.1 s | 279 | ab-both | Ctrl+Alt+] | B23 |
| Playback ▸ A-B Loop ▸ Loop Current Chapter or Bookmark Range | 280 | ab-toggle | Alt+\ | B23 |
| Playback ▸ A-B Loop ▸ A-B Loop On/Off (checkbox) | 281 | ab-toggle | `\` | B23 |
| Playback ▸ A-B Loop ▸ Loop Count ▸ Endless / 1 / 2 / 3 / 5 / 10 (radio) | 282–287 | ab-count | | B23 |

Conflict check and trade-offs:
- PgUp/PgDn without modifiers are playback's "previous / next"; this plugin only uses combinations with Shift or Ctrl, so there is no conflict: bookmarks/chapters use Shift+PgUp/PgDn, chapters use Ctrl+PgUp/PgDn.
- P has no playlist binding (coordinator ruling 2026-11-08: Play from the list is Enter or a double-click), so P adds a bookmark whatever has focus, the list included.

### B21 Chapter / bookmark menu and jumping

**Requirement**
- **Menu**: under "Playback ▸ Chapters and Bookmarks", in order:
  - The action items.
  - Chapters: `01  Opening    00:00`; when the title is `null`, written as `Chapter 1`; the current chapter shows a radio dot.
  - Bookmarks: `bookmark title    01:23:45`, sorted by time.
  - The "Show Marks on Seek Bar" toggle.
  - Choosing an item jumps to its time.
- **Shortcuts**: P adds a bookmark at the current position; Shift+PgUp / Shift+PgDn jump to the previous / next point among chapters and bookmarks merged; Ctrl+PgUp / Ctrl+PgDn jump only between chapters.
- **Bookmark management window "Chapters and Bookmarks" (H)**:
  - The list shows chapters (read-only) and bookmarks, sorted by time, in three columns: type icon / time / title.
  - Double-click to jump.
  - With a bookmark selected, you can: change the title (inline edit), change the time (time input box `h:mm:ss.d`), remove it.
  - Bottom buttons: `Add Bookmark`, `Remove`, `Clear Bookmarks` (clears all bookmarks of the current file after confirmation).
  - A search box at the top filters by title.
- Bookmark fields: `time` (seconds, one decimal place), `title` (1–100 characters, default `Bookmark N`, where N is the current file's bookmark count + 1), `created` (ISO time).
- At most 500 bookmarks per file.

**Entry points**: see the menu table; H opens the window, and pressing H again brings the open window to the front.

**Behavior details**
- "Previous" searches from `position − 1 second`, so repeated presses can pass the point just reached; "next" searches from `position + 0.5 seconds`. When there is no point before or after, the message is `No previous bookmark or chapter` / `No next bookmark or chapter`.
- Add bookmark: disabled when `phase !== "ready"` or the source cannot seek (live stream), with `reason: "Can't add bookmarks to this source"`. When a bookmark already exists within ±0.5 seconds of the same point, it is not added again, with the message `A bookmark already exists here`. On success the message is `Bookmark added: Bookmark 3 (01:23)`.
- Jump: both chapters and bookmarks use `player.seek(time)`. On success the message is `Chapter: Opening` / `Bookmark: Bookmark 3`.
- Dynamic menu: when chapters or bookmarks change, the child scope is released and rebuilt. Order allocation: chapter items `10000 + i`, bookmark items `20000 + i`, "Show Marks on Seek Bar" 30000. This keeps the chapter group and the bookmark group each contiguous, with separators between the groups (the order inside this submenu is independent of the outer level). The menu lists at most 50 chapters and 50 bookmarks; beyond that the last item is "More… (N in all)", which opens the window when clicked.
- **Media identity for bookmarks**: local files use `realpath`; URLs use the literal `player.state.source`. The file size is recorded too, used to show "The file has changed; bookmark times may be off" at the top of the list; bookmarks are not deleted.
- With no media, "Add / Previous / Next" are disabled; the menu only shows the action items; the window shows "No media open".
- On source change the window content switches accordingly; uncommitted inline edits are discarded.
- After the plugin is disabled, bookmarks stay in the data directory.

**Interface**: "Chapters and Bookmarks" window 380×420, minimum 320×300. Search box at the top (21px); list with three columns (type 16px icon / time 70px monospace digits / title), row height 18px; with a bookmark selected, the edit area below shows two input boxes "Time" and "Title" (21px, row spacing 26px); bottom button row `Add Bookmark` `Remove` `Clear Bookmarks` (21px). Chapter rows use the `text-dim` color to show they are read-only.

**Persistence**: `<dataDirectory>/bookmarks.json` (`plugin.storage.json`): `{ "version": 1, "media": { "<identity>": { "size": 123, "modified": "<ISO>", "items": [{ "time": 83.4, "title": "Bookmark 1", "created": "…" }] } } }`. Written 500 ms after each change, debounced (temporary file plus rename). When there are more than 2000 media entries, the least recently modified ones by `modified` are evicted; files without bookmarks leave no entry. When the file size has changed, the warning is kept, and the new size is adopted the next time that file's bookmarks are edited.

**Core dependencies**: the existing `player.seek`, `windows.create`, `osd.show`; [New core API]

```ts
interface PlayerState {
  readonly chapters: readonly Chapter[] | null;   // null = not ready yet; [] = no chapters; needs playback.read
}
interface Chapter { index: number; title: string | null; start: number }   // start in seconds, ascending by start
```

Reason: chapters are media information that mpv already reads (`chapter-list`), and plugins cannot reach mpv; plugins also cannot read network sources with ffprobe. On source change the core first sets it to `null`, and fills it in once ready.

**Acceptance**
- Play an MKV with 5 chapters: "Playback ▸ Chapters and Bookmarks" lists 5 chapters, with a radio dot on the current one; choosing chapter 3 jumps to its start and shows the chapter name.
- Press P at 00:10 and 00:50: two bookmarks appear in the menu; Shift+PgUp first jumps to 00:50, pressing again to 00:10.
- After restarting Vestra Player and opening the same file again, the bookmarks are still there.
- Rename "Bookmark 1" to "Climax" in the window: the menu updates within 1 second.
- In a live stream P is unavailable, and the menu item shows the disabled reason.
- With the playlist focused, P still adds a bookmark (the list plays with Enter or a double-click).

**Round 3a implementation notes**
- Each menu path segment is at most 80 bytes (host limit): chapter and bookmark menu text is shortened by bytes with `…` appended, and duplicates get ` (2)`; menu overflow items are split into "More Chapters… (N in all)" and "More Bookmarks… (N in all)".
- Title and time editing in the window happen in the edit area below the list (committed on Return or when leaving the input box), not inline in the list rows; a time beyond the duration or in the wrong format is reported in the status line and not committed.
- When Ctrl+PgUp/PgDn have no chapter to jump to, the message is `No previous chapter` / `No next chapter`.

**Notes**: bookmarks are sorted by time, with no "move up / move down"; "batch edit times" and "check bookmarks" are not done; bookmark files next to the video are not done (P2, can be discussed later as an export option); H opens a window, not a popup menu (the core cannot pop up a menu for a plugin at an arbitrary position, and nothing is added for that this round).

### B22 Chapter / bookmark markers on the seek bar

**Requirement**
- When the toggle "Show Marks on Seek Bar" (on by default) is on, the player's bottom bar seek bar draws a vertical line marker at each chapter start and bookmark, and shows the title and time on mouse hover.
- While an A-B loop is active, the seek bar draws points A and B and a background color for the range.
- The toggle belongs to marks; when off, marks publishes an empty mark layer.

**Entry point**: Playback ▸ Chapters and Bookmarks ▸ Show Marks on Seek Bar (checkbox).

**Choice of approach**

| Approach | How | Assessment |
| --- | --- | --- |
| A. The core holds bookmarks and chapters | The core stores bookmarks and provides add/delete interfaces; playback reads from the core | Naming, persisting and editing bookmarks are business logic; putting them in the core violates "keep the core lean" |
| B. The core draws marks into the seek bar directly | The core injects elements into playback's Slint | The seek bar is a plugin's interface, and the core cannot provide a dedicated layout by plugin name (interaction requirements §2) |
| **C. The core provides a general "timeline mark layer" (chosen)** | Each plugin publishes its own layer of marks to the core; the core manages and merges them per source and makes them readable by all plugins; playback only draws "the merged mark layers + `PlayerState.loop`" | The core only forwards data (the same pattern as one `osd.panel()` block per plugin and the playback sequence flags), with no business logic; any plugin can contribute marks (for example a future subtitles plugin could mark dialogue) |

Approach C is chosen. Chapter marks are also published by marks (it reads `PlayerState.chapters`) rather than added automatically by the core, so that the "show marks" toggle fully controls both kinds of marks, chapters and bookmarks. The A-B range is drawn by playback reading `PlayerState.loop` directly, because the loop state is in the core and should be shown whichever plugin set the loop.

**Behavior details (marks side)**
- With the toggle on and the media ready, publish `chapters ∪ bookmarks`: `kind` is `"chapter"` or `"bookmark"`, and `title` uses the text shown in the menu. Republish when chapters, bookmarks or the toggle change (last write wins).
- Publish with `ifCurrent: state`. The core automatically clears all mark layers when the source changes, so the old file's marks are not drawn on the new file; marks republishes once the new source is ready.
- At most 1000 marks per layer; beyond that only the first 1000 (by time) are published.
- Turning the toggle off: publish `[]`. On-screen message `Bookmark and chapter marks on the seek bar: on/off`.

**Behavior details (playback side, written for the playback chapter as a dependency)**
- On the 2px progress line, draw a 1px wide, 6px high vertical line for each mark (centered across the progress line), colored `Theme.text-dim` for chapters and `Theme.accent` for bookmarks. Draw the A-B range as a 2px high `Theme.playing` block below the progress line, with a 1px × 8px vertical line at each of A and B.
- When hovering within ±3px of a mark, the tooltip shows `title  01:23` (combined with the I06 hover time tooltip).
- Marks do not take clicks: a click still seeks by the seek bar's existing rules.

**Persistence**: the toggle is stored in `showOnSeekbar` of `<dataDirectory>/marks.json`.

**Core dependencies**
- [New core API] timeline mark layer:

  ```ts
  type MarkKind = "chapter" | "bookmark" | "marker";
  interface TimelineMark { time: number; kind: MarkKind; title: string | null; plugin: string /* publisher manifest id */ }
  interface Timeline {
    readonly marks: readonly TimelineMark[];                 // merged result of all layers on the current source, ascending by time; needs playback.read
    onChange(listener: (marks: readonly TimelineMark[]) => void): Disposable; // needs events
    layer(): TimelineLayer;                                  // this plugin's layer, one per plugin; needs timeline.marks
  }
  interface TimelineLayer extends Disposable {
    set(marks: { time: number; kind: MarkKind; title?: string }[], opts?: { ifCurrent?: PlayerState }): void; // last write wins; [] clears
  }
  // plugin.timeline: Timeline
  ```

  Rules: `time` must be within `[0, duration]` (only ≥ 0 when `duration` is unknown), otherwise it fails with `invalid`; `title` at most 100 characters; at most 1000 marks per layer; the core clears the layer on source change or when the plugin is disabled. A second call to `layer()` throws.
  Reason: plugins cannot share state, and the seek bar belongs to another plugin, so only the core can relay. This is a general data layer with no business logic, reusable for any future timeline annotation.
- playback has to read `plugin.timeline.marks` and `PlayerState.loop` and draw them (the playback chapter takes this on).

**Acceptance**
- A file with 5 chapters + 2 bookmarks: 7 vertical lines appear on the seek bar, bookmarks in the accent color; hovering a bookmark shows its title.
- Turn the toggle off: the lines disappear within 1 second; turn it on again: the lines come back.
- Switch to a file without chapters: the old marks disappear immediately, nothing is left.
- After disabling the marks plugin, there are no marks on the seek bar, and the A-B range (if set) is still shown.
- Unit test: `host.timeline` (to be added to TestHost) records what is published; after a source change the layer is empty.

**Round 3b implementation notes (marks side)**
- A mark's `title` is the chapter name (title or `Chapter N`) or the bookmark title itself, without the number and time from the menu row (the time is shown by playback's tooltip).
- When only A is set, `{ kind: "marker", title: "A" }` is published as well (B23); it belongs to this plugin's layer, so it is not shown when the toggle is off either.
- Marks whose time is outside `[0, duration]` (such as old bookmarks after a file got shorter) are left out, so the core does not reject the whole layer with `invalid`; the core compares `ifCurrent` only by source (item 3 of the maintainer's signature adjustment).
- The range of Alt+\ (B23) still only uses this plugin's chapters and bookmarks and does not read `timeline.marks`.

**Notes**: split into two parts, "marks publishes, playback draws".

### B23 A-B range loop

**Requirement**
- **Set points**: `[` sets start point A to the current position, `]` sets end point B to the current position, with 0.1-second precision (rounded).
- **Clear**: `{` clears A, `}` clears B.
- **Fine-tune**: start and end points each ±0.1 seconds (Ctrl+[ / Alt+[, Ctrl+] / Alt+]); the whole range ±0.1 seconds (Ctrl+Alt+[ / Ctrl+Alt+]), with its length unchanged.
- **Toggle** (`\`): with both A and B set, starts or stops the loop; A and B are kept after the loop stops.
- **Loop Current Chapter or Bookmark Range** (Alt+\): sets A and B to the mark range containing the current position (between two adjacent chapters or bookmarks) and starts looping immediately.
- **Loop count**: endless (default) / 1 / 2 / 3 / 5 / 10. When the count is used up, the loop stops automatically and playback continues.
- After A and B are set, looping **starts automatically** by default (the loop starts once the end point is set).

**Entry points**: see the menu table.

**Behavior details**
- Constraints: `0 ≤ A < B ≤ duration`, `B − A ≥ 0.2 seconds`.
  - When setting A, if A ≥ B − 0.2: clear B, message `Start point is after the end point; end point cleared`.
  - When setting B, if B ≤ A + 0.2: not set, message `End point must be after the start point`.
  - When setting B without A: A becomes 0, message `A-B loop: 00:00.0 ~ 01:10.0`.
- Fine-tune limits: A is clamped to `[0, B − 0.2]` and B to `[A + 0.2, duration]`; when moving the whole range, neither end goes out of bounds, and it stops at the limit (message `Reached the beginning` / `Reached the end`).
- Submitting to the core: when both A and B exist and the loop is on, call `player.setLoop({ a, b, count })`; otherwise call `setLoop(null)`. With only A set, there is no loop, and the seek bar shows an A marker (through marks' mark layer, `kind: "marker"`, title `A`).
- When the loop starts and the current position is not within `[A, B)`, `seek(A)` first. When the user seeks past B during the loop: no looping, playback continues (consistent with mpv's semantics of jumping back only when crossing B); after seeking back into the range, looping resumes as usual.
- When the count is used up, the core sets `PlayerState.loop` to `null`, the plugin sets the toggle state to off accordingly with the message `A-B loop ended`, and A and B are kept.
- On source change: the core clears the loop, and the plugin clears A and B (a range only makes sense for the current file; "remember the A-B range loop list" is not done this round).
- Live streams and sources that cannot seek: all A-B commands are disabled with `reason: "This source doesn't support A-B loop"`.
- Another plugin set a loop (`PlayerState.loop` is not the value marks submitted): marks' "on/off" check reflects the core state; pressing `\` turns off the current loop in the core. When marks sets points afterwards, it overwrites the loop in the core.
- Alt+\: find the range containing `position` among the merged time points (chapters and bookmarks in `timeline.marks`, plus 0 and duration). When there are no marks at all, the message is `No chapters or bookmarks to define a range`.
- On-screen messages: `A-B loop: 01:02.3 ~ 01:10.0` (when the loop starts and when adjusting); `A-B start: 01:02.3` (only A set); `A-B loop off` (when turned off). Time format `m:ss.d`, `h:mm:ss.d` above an hour.
- The fine-tune commands set `repeat: true`, so holding adjusts continuously; submits to the core after a 100 ms debounce, to avoid piling up requests.

**Interface**: only menus and shortcuts, no window. "Edit the A-B range loop list" is not done this round.

**Persistence**: the loop count is stored in `abCount` of `marks.json`. A and B are not persisted.

**Core dependencies**
- [New core API]

  ```ts
  interface Player {
    setLoop(loop: { a: number; b: number; count?: number /* 1–99; omitted = endless */ } | null,
            opts?: { ifCurrent?: PlayerState }): Promise<void>; // needs playback.control; conditioned on the current state by default
  }
  interface PlayerState {
    readonly loop: { a: number; b: number; count: number | null; remaining: number | null } | null;
  }
  ```

  Rules: fails with `invalid` when `0 ≤ a < b` does not hold; `b` greater than `duration` is clamped to `duration`; the core clears the loop on source change (mpv's `ab-loop-*` is kept across files by default, so the core has to reset it explicitly); sources that cannot seek fail with `unsupported`.
  Reason: if the plugin listened to the position and then called `seek`, the precision would only be the interval of position events (about 0.1 seconds), the jump-back point would jitter, and it would race with user actions through `stale`. mpv runs `ab-loop-a/b/count` precisely at the decoding layer, which only the core can use. With `PlayerState.loop` public, playback can draw the range.

**Acceptance**
- Press `[` at 00:10 and `]` at 00:20: message `A-B loop: 0:10.0 ~ 0:20.0`; when playback reaches 0:20 it goes back to 0:10, three times in a row with an error ≤ 0.1 seconds (judged by the time display).
- Press `\` to stop the loop: playback continues past 0:20; press `\` again: it goes back to 0:10 and loops.
- Hold Ctrl+] for 1 second: B moves back continuously, and the message time changes along.
- Set the loop count to 2: after looping twice it stops automatically with the message `A-B loop ended`, and the checked item becomes unchecked.
- After moving to the next file, the loop and A and B are all cleared, and there is no range on the seek bar.
- Alt+[, Alt+] and Alt+\ trigger on real macOS hardware (see the global conventions).
- Unit test: TestHost records the arguments of `player.setLoop`; the sequence `[`, `]`, `\` produces `{a:10,b:20}` → `null` → `{a:10,b:20}`.

**Round 3a implementation notes**
- The loop count is used directly as the core's `count` (number of jumps back): choosing "2 Times" = the range plays 3 times in all. The core does not report the remaining count (`PlayerState.loop` has no `remaining`).
- The constraint is unified as `B − A ≥ 0.2` (exactly 0.2 is allowed).
- The seek bar A marker for "only A set" is implemented with B22 in 3b; the range of Alt+\ uses marks' own chapters and bookmarks (3a has no `timeline.marks`).
- When clearing the start / end point: if the loop is on, the message is `A-B loop off`, otherwise `Start point cleared` / `End point cleared`; fine-tuning with only A or only B shows `A-B start: …` / `A-B end: …`; when the Alt+\ range is shorter than 0.2 seconds, the message is `Range too short to loop`.

**Notes**: no range list (Shift+\), no "delay at the end of the loop", no time correction.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| `PlayerState.chapters: readonly Chapter[] \| null` (`Chapter { index; title; start }`) | Chapters are media information that mpv already reads; plugins cannot read them for network sources; shared by several plugins |
| `player.setLoop(loop: { a; b; count? } \| null, opts?)`; `PlayerState.loop` | Precise looping can only be done by mpv; with the loop state public, playback can draw it |
| `plugin.timeline.marks` / `onChange` / `layer().set(marks, opts)`; capability `timeline.marks` | The seek bar belongs to another plugin, so marks can only be relayed through the core; a general data layer with no business logic |

---

## Shortcut settings (`hotkeys`)

**Location**: `src/plugins/hotkeys/` (new plugin, to be created)

### Scope

The hotkeys plugin provides the rebinding interface: keyboard and mouse bindings for any command registered by any plugin or the host (J01, J02), plus an "Ignore Default Shortcuts" toggle (J03). Design basis: [feature catalog, "About the shortcut settings plugin"](feature-catalog.md#about-the-shortcut-settings-plugin-j01j03):

- All bindings are in the core's binding table. Plugins only **declare** default bindings, and the core decides which input triggers which command.
- The core already has a **user overlay**: `bindings.rs::Table.overlay`, keyed by `"<manifest id>/<command id>"`, with arrays of bindings as values, where `[]` means unbound; the overlay takes priority over all default bindings. The overlay is saved across launches in the `bindings` field of `ui-state.json` (`ui_state.rs`). The current state is "read-only this round, kept on save": the host reads it with `set_overlay` at startup, and writing is not exposed to plugins.
- hotkeys only talks to the core (list, rebind, restore); it does not know about or call playback, playlist or other plugins. The commands of plugins installed later, and of plugins generated by the workshop, appear in the list automatically.

Additional current facts (read from `bindings.rs`):
- Host commands use `manifest = "host"`, so the overlay keys are `host/fullscreen`, `host/enter-fullscreen`, `host/exit-fullscreen`, `host/quit`.
- The bindable mouse inputs are a fixed list: `LeftDoubleClick[:LeftHalf|:RightHalf]`, `MiddleClick`, `X1`, `X2`, `WheelUp/Down/Left/Right`. All of them are read over the video area, except `X1` / `X2`, which are window-level as in browsers (host 0.6.0): anywhere in the main window or a floating panel, not over an open menu. **Left click cannot be bound** (the host uses left button down for focus and `capture`), and the right button is reserved for the menu.
- On conflict the overlay wins, then "host → earlier-loaded plugin → declaration order"; suppressed bindings are logged (`conflicts`).
- Panel scope (`focus: "panel"`) and global scope are two separate tables.

Whether the basic playback actions move to the core (the suggestion in that section of feature-catalog.md) is a change of core responsibilities and is not part of this plugin's requirements. This plugin has only one requirement for it: whoever registers a command, as long as it is in the binding table, it can be rebound.

Manifest capabilities: `commands`, `ui.window`, `ui.osd`, `bindings.manage` (new capability); if J04 approach A lands, `ui.settings` too.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| J01 | Shortcut settings: list all commands and their current bindings; rebind, unbind, restore defaults, conflict handling | ✗ | `plugin.bindings.list/set/reset/onChange`; capability `bindings.manage` |
| J02 | Mouse action settings: choose a command for each mouse input | ✗ | Same as above; [New core API] input `Mouse:LeftClick` |
| J03 | Ignore default shortcuts | ✗ | `plugin.bindings.ignoreDefaults` / `setIgnoreDefaults()` |

### Menus and shortcuts

| Menu path | order | group | Default binding |
| --- | --- | --- | --- |
| Settings ▸ Keyboard Shortcuts… | 832 | settings-input | (none) |
| Settings ▸ Mouse… | 833 | settings-input | (none) |
| Settings ▸ Ignore Default Shortcuts (checkbox) | 834 | settings-input | (none) |

hotkeys sets no default shortcuts of its own, to avoid conflicting with any plugin; if the user makes a mistake, they can find their way back through the menu. Once J04 approach A lands, "Keyboard Shortcuts…" and "Mouse…" change to `plugin.settings.show("hotkeys.keyboard" | "hotkeys.mouse")`, opening the matching page in the preferences window; until then they open this plugin's own tool windows. Both forms have the same layout.

### Round 3a implementation notes

- J03 is implemented with the alternative from ruling R-K2: there is no `ignoreDefaults` / `setIgnoreDefaults`, and no `ignoreDefaultBindings` in `ui-state.json`. hotkeys writes empty overrides for the commands still running on defaults, records the entries it wrote in `<dataDirectory>/ignore-defaults.json`, and only restores those when turned off; for newly loaded plugins it writes them after `bindings.onChange` (a one-round-trip window). This note takes precedence over the "Core dependencies" and "Persistence" sections of J03 below.
- J01 and J02 are two separate tool windows (J04 mounting comes in round 3b): keyboard 600×460 (minimum 520×380), mouse 540×410. J02's command selection opens as a filterable list in place of the table rather than a drop-down (there are over a hundred global commands, which a drop-down cannot hold).
- J01 has one more state, "Ignored" (the empty overrides J03 writes); the shortcut column shows the ignored default keys in parentheses (Slint text has no strikethrough).
- Capture cannot get physical keys: numpad keys are captured as main keyboard names; ⌃P–⌃X are captured with ⌘ (a `Ctrl+` binding is triggered by both ⌘ and ⌃). See the plugin README for details.

### J01 Shortcut settings

**Requirement**
- List all commands in the binding table, including host commands, plugin commands, and commands without a menu path (such as I03, exit fullscreen), one per row:
  - **Command**: the menu path joined with " ▸ ", for example `Playback ▸ Seek ▸ Forward 5 s`; `title` when there is no menu path.
  - **Shortcut**: all keyboard bindings currently in effect, comma-separated, shown as menu text like `Ctrl+O, F3`.
  - **Scope**: `Global` / `In panel` (commands with `focus: "panel"`, for example Delete in the playlist).
  - **Source**: the plugin id; host commands show "Player".
  - **State**: `Default`, `Changed` (this entry is in the overlay), `Conflict` (declared, but an input is suppressed by another command).
- **Filter**:
  - The search box matches command text or key text; typing `Ctrl+` finds all Ctrl combinations.
  - Drop-down "All / Changed / Conflicts / Unbound".
  - Filter by "Source".
- **Edit the selected command**:
  - The edit area below lists each binding of the command as a "tag", each with `×`; clicking `×` deletes that binding.
  - `Add Key…`: enters the capture state and waits for the user to press a key combination.
  - `Restore Default`: restores only this entry.
  - `Unbind All`: writes `[]`.
- **Global actions**:
  - `Restore All Defaults`: clears the whole overlay, with confirmation.
  - `Export List…`: saves the current full shortcut table as plain text or CSV for the user to view and print, i.e. the "shortcut list".
- At most 8 bindings per command.

**Entry point**: Settings ▸ Keyboard Shortcuts…

**Behavior details**
- **Capture**:
  - In the capture state, the capture box shows "Press a shortcut (Esc cancels)". Bindings do not take effect inside tool windows anyway (API reference §4.1), so the pressed key does not trigger a command.
  - Pressing a modifier alone does not end capture.
  - Esc cancels capture; to actually bind Esc, use the "Esc" item in the drop-down next to the capture box.
  - Backspace is captured normally.
- **Key conversion**: Slint `FocusScope.key-pressed` gives `event.text` and the modifiers. The plugin converts them to the canonical form with the same rules as `bindings.rs::from_slint_key`:
  - swap ⌘ and ⌃ on macOS;
  - convert private-use key codes to `ArrowUp`, `F5`, etc.;
  - drop Shift from symbol keys;
  - write `+` as `Plus`.
  The core's `set()` validation is final; when invalid, `This key cannot be used` is shown.
- **System-reserved keys**: ⌘Tab, ⌘Space, ⌃↑ etc. are taken by macOS and cannot be captured; no special handling is needed. ⌘Q is the default key of the host's "Quit": it can be changed, but binding ⌘Q to another command needs a second confirmation ("⌘Q is the default shortcut for quitting Vestra Player. Change it to …?").
- **Conflicts**: when, within the same scope, the new input is already an active binding of another command, a confirmation bar appears: `Ctrl+E is already used by "Video ▸ Snapshot ▸ Save Current Frame"` + `Replace` / `Cancel`.
  - Replace: first write the other command's binding as "its active bindings minus this input", then add this input to this command. It is complete only when both `set` calls succeed; if the second fails, the first is reverted.
  - There is no "keep both": by core rules only one of two user bindings at the same level takes effect, and keeping both would confuse users.
  - Panel scope and global scope do not count as a conflict (the core checks the panel first), but the status line says "This key does something else when the playlist has focus".
- **Defaults suppressed** (default conflicts between plugins, for example two plugins both declaring F6): that row's state is "Conflict", and hovering shows "F6 is taken by "Playlist" and has no effect". The user can resolve it by changing the key of one of them.
- **Liveness**: subscribe to `bindings.onChange`; when plugins are installed or disabled, declarations change, or the overlay is changed elsewhere, the list refreshes within 1 second, keeping the selected row (matched by `plugin + command`).
- **Plugins not running**: the overlay may contain entries of disabled or uninstalled plugins (`orphans`). They are hidden by default; they are shown when "Plugins not loaded" is chosen in the "Source" filter, and can be deleted. `Restore All Defaults` also clears these entries.
- **Feedback**: each successful change shows the on-screen message `Shortcut updated: Playback ▸ Play/Pause = Space, K`.
- When a command is unregistered during editing (its plugin is disabled), `set` fails with `missing`, "The command no longer exists" is shown, and the list refreshes.

**Interface**
- Since round 3b this is a settings page in the options window (id `hotkeys.keyboard`, category "Keys and Mouse"), 440 wide: the list has three columns, command adaptive (about 209; the 220 from the ruling did not fit after padding and spacing) / shortcut 140 / state 60; source and scope are written in the edit area title "Command (source · In panel)"; source can still be filtered, and the export still has five columns. At the top, search 190, state filter 90, source filter 128. The round 3a tool window (600×460) has been removed.
- Top (30px): search box (200px), state filter drop-down (97px), source filter drop-down (120px).
- List: five columns (command adaptive / shortcut 140px / scope 50px / source 120px / state 50px), row height 18px, sortable by command or shortcut; "Changed" rows use `Theme.text`, default rows `Theme.text-list`, "Conflict" `Theme.accent`.
- Edit area (90px high):
  - Line 1 is the full path of the selected command (11px bold);
  - Line 2 is the binding tags (each 18px high, corner radius `radius-control`, with `×` inside) and the capture box (140×21);
  - Line 3 is the buttons `Add Key…` `Restore Default` `Unbind All` (21px).
- Bottom button row: `Restore All Defaults…` `Export List…` on the left, `Close` on the right.

**Persistence**: binding data is written by the core to `ui-state.json` (the overlay); this plugin saves nothing extra. Only interface preferences (sort column, filters) are stored in `<dataDirectory>/hotkeys.json`.

**Core dependencies**
- [New core API] (refining the draft in feature-catalog.md):

  ```ts
  interface BindingRef { plugin: string /* manifest id; "host" for the host */; command: string }
  interface BindingInfo extends BindingRef {
    title: string;                 // menu item text or title
    path: string[] | null;         // menu path; null when not in the main menu
    scope: "global" | "panel";
    defaults: string[];            // declared default bindings (canonical form)
    current: string[];             // bindings actually in effect (overlay or defaults, minus suppressed ones)
    overridden: boolean;           // this entry is in the overlay
    shadowed: { input: string; by: BindingRef }[]; // inputs declared but not in effect, and who holds them
    labels: string[];              // menu display text of current ("Ctrl+O", "←")
  }
  interface Bindings {
    list(): Promise<{ commands: BindingInfo[]; orphans: { ref: BindingRef; inputs: string[] }[] }>;
    set(ref: BindingRef, inputs: string[]): Promise<void>;   // writes the overlay; [] = unbind; validation failure invalid; command missing missing
    reset(ref?: BindingRef): Promise<void>;                  // deletes this override (restore default); omitted = clear the whole overlay
    onChange(listener: () => void): Disposable;              // any change to declarations, unregistration, overlay or ignoreDefaults
    readonly ignoreDefaults: boolean;                        // J03
    setIgnoreDefaults(on: boolean): Promise<void>;           // J03
  }
  // plugin.bindings: Bindings, needs the capability bindings.manage (can change other plugins' shortcuts)
  ```

  Core implementation notes: `set` and `reset` write `Table.overlay`, then call `rebuild()` and mark `ui_state` as needing a save (with the existing debounced write); the shortcut text on the right of menus updates accordingly; `orphans` are overlay entries whose keys match no declared command, and `set` may write `[]` to an orphan entry to delete it.
  Reason: the binding table and the overlay are in the core, and plugins cannot change each other; this is exactly what the overlay was reserved for.

**Acceptance**
- Open the window: the list contains all commands with shortcuts in menus.md, plus the host's "Fullscreen", "Exit Fullscreen" and "Quit".
- Add `K` to "Play/Pause": pressing K pauses; the context menu still shows `Space` to the right of "Play/Pause" (the first active key); K still works after a restart.
- Capture `Space` for "Save Current Frame": a conflict confirmation appears; after confirming the replacement, Space takes a screenshot, and "Play/Pause" only has K left (if K was never added, it is unbound with the state "Changed").
- After "Restore Default", that row is back to `Default`, and Space is play/pause again.
- Disable audio: the Shift+E row disappears within 1 second; after re-enabling, the earlier rebinding of it still applies (the overlay is saved by manifest id).
- Press ⌘O in the capture box: `Ctrl+O` is recorded (Ctrl accepts ⌘ on macOS); Shift+/ gives `?`.
- Unit test: TestHost gains `host.bindings` (overlay simulation); after `set`, `host.press("K")` triggers the matching command.

**Notes**: editing is per command, so one command shows all its bindings; there is no separate "user shortcuts" list layered over the default shortcuts; multimedia key settings are not done.

### J02 Mouse action settings

**Requirement**
- One row per mouse input, each with a drop-down to choose the command that input triggers, or "(None)":
  - Left click ([New core API], none by default)
  - Left double-click
  - Left double-click (left half)
  - Left double-click (right half)
  - Middle click
  - Back button (X1)
  - Forward button (X2)
  - Wheel up
  - Wheel down
  - Wheel left
  - Wheel right
- A "Modifier" radio at the top: `None` / `Ctrl` / `Alt` / `Shift` / `Cmd`. After switching, the list shows the bindings for "that modifier + mouse input", for example Ctrl+wheel.
- The drop-down lists all global-scope commands, sorted by menu path, filterable by typing.
- Each row has a "Default" button on the right (small, 18px) that restores the default owner of that input.

**Entry point**: Settings ▸ Mouse…

**Behavior details**
- When command X is chosen for an input:
  1. First remove this input from the current holder's bindings (`set(holder, holder.current − input)`);
  2. Then `set(X, X.current + input)`.
  Choosing "(None)" runs only the first step.
- "Default" button: if the input has a default owner (a command whose `defaults` contain it), restore this input in that command's override; if the current holder is not the default owner, remove this input from the holder.
- **Left click**:
  - Only when a command is bound does the core delay the single-click decision: it triggers only when no second click comes within the system double-click interval (macOS `NSEvent.doubleClickInterval`, about 0.5 seconds by default) and the pointer moves no more than 4px between press and release.
  - With no binding, things stay as now: focus immediately, no delay.
  - The interface explains "Once bound, a single click waits for the double-click interval, so it responds a little later".
- Right click always opens the menu and does not appear in the list.
- Wheel input is counted in steps of 30px (host `WHEEL_STEP`); this behavior is not set here.
- Changes take effect immediately, with the on-screen message `Mouse action updated: Wheel up = Audio ▸ Volume +`.

**Interface**: a settings page in the options window (id `hotkeys.mouse`, category "Keys and Mouse", 440 wide). Modifier radio row (21px); below it an 11-row table, each row a label 110px right-aligned + command selection 250×21 + "Default" 41×18, row spacing 26px (opening the command selection shows a filterable command list in place of the table).

**Persistence**: same as J01, saved by the core overlay.

**Core dependencies**
- Same as J01: `plugin.bindings.*`.
- [New core API] bindable input `Mouse:LeftClick`: added to `bindings.rs::MOUSE`, with single-click detection added to the video area TouchArea in `host.slint` (delayed only when `has_click` is true, handled the same way as `has_wheel`).
  Reason: a single click is a pointer event in the video area; only the host can recognize it and tell single from double clicks; UI-MOUSE-03 also lists it as pending.

**Acceptance**
- Change "Wheel up" to "Playback ▸ Seek ▸ Forward 5 s": wheel up seeks forward 5 seconds, and "Audio ▸ Volume +" only has ↑ left.
- Set "Left click" to "Play/Pause": a click on the video pauses after about 0.5 seconds; a double click still toggles fullscreen without an extra pause.
- Choose modifier Ctrl and set Ctrl+wheel up to "Video ▸ Brightness +1%": verify it works.
- The "Default" button sets "Back button" back to "Previous".

**Notes**: right click / double click is always the menu and cannot be set; middle double-click is not done; left/right-half double clicks are provided by the host.

### J03 Ignore default shortcuts

**Requirement**
- When the toggle is on, none of the default shortcuts and default mouse bindings declared by plugins take effect; only bindings in the user overlay do.
- The host's "Quit" (⌘Q) and "Exit Fullscreen" (Esc) are not affected, as safe exits.
- The menu itself is not affected; the user can turn the toggle off at any time from "Settings ▸ Ignore Default Shortcuts".
- The toggle is off by default and saved across launches.

**Entry points**: Settings ▸ Ignore Default Shortcuts (checkbox); the J01 window also has the same checkbox at the bottom (same state).

**Behavior details**
- Turning it on asks first: "After this, only the keys you changed in the shortcut settings work; default keys such as Space and the arrow keys will stop working. Continue?". After confirming, the message is `Ignore default shortcuts: On`.
- When on, the default-sourced bindings in the "Shortcut" column of the J01 list are shown struck through (`current` is empty), with the state "Ignored".
- The shortcut text on the right of menus disappears accordingly (naturally, from the core's `label()`).
- Newly installed plugins are covered too: their default bindings do not take effect.

**Persistence**: the core writes it to `ui-state.json` (new field `ignoreDefaultBindings: boolean`).

**Core dependencies**
- [New core API] `plugin.bindings.ignoreDefaults` / `setIgnoreDefaults(on)` (signature in J01). In `rebuild()`, when the toggle is on, the core skips default declarations with `owner != HOST`, and keeps the default bindings of `quit` and `fullscreen` (Toggle Fullscreen, Escape; `exit-fullscreen` before player 0.6.0) among host commands.
- Reason: a plugin could simulate this by writing every command as `[]`, but that would mix the overlay with "the user's real changes" and make it impossible to restore when turned off; newly loaded plugins would also race (active first, unbound later). This is just one boolean, and judging it in the core during resolution is the most reliable.

**Acceptance**
- When on, Space, the arrow keys and Tab do nothing; K (play/pause) changed in J01 still works; ⌘Q still quits.
- When off, all default keys come back immediately.
- The state is kept after a restart.
- With it on, install a new plugin with default shortcuts: its shortcuts do not take effect.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| `plugin.bindings.list(): Promise<{ commands: BindingInfo[]; orphans }>` | The binding table is in the core; plugins cannot see other plugins' commands |
| `plugin.bindings.set(ref, inputs[])` | Write the user overlay (already in the core, not exposed) |
| `plugin.bindings.reset(ref?)` | Restore defaults (one entry or all) |
| `plugin.bindings.onChange(listener)` | Refresh when plugins load or are disabled and when the overlay changes |
| `plugin.bindings.ignoreDefaults` / `setIgnoreDefaults(on)` | A switch in the resolution rules; simulating it in a plugin would pollute the overlay |
| Capability `bindings.manage` | Can change other plugins' shortcuts, so it must be declared explicitly |
| Bindable input `Mouse:LeftClick` (decision delayed only when bound) | Pointer events in the video area can only be recognized by the host |

---

## Preferences (`preferences`)

**Location**: `src/plugins/preferences/` (created in round 3b)

### Scope

The preferences plugin provides a unified settings entry point:

- **Main preferences window** (J04): shows the settings of all plugins in one place.
- **Appearance**: color scheme switching (H10).

The hard part is J04: settings pages are spread across plugins, and plugins cannot communicate. After comparing three approaches, this chapter recommends "the core provides settings page mount slots": pages still belong to their plugins and only talk to their own plugin; the preferences window is a container composed by the core. The preferences plugin only provides its own "Appearance" page and the entry command.

Color schemes (H10) require the core's `Theme` to be switchable at runtime. Today the colors in `theme.slint` are `out property` constants taken from the brand logo: amber → magenta → purple, on a purple-tinted near-black background.

Manifest capabilities: `commands`, `events`, `ui.osd`, `ui.settings`, `ui.theme` (once approach A lands, it no longer needs its own tool window and dialog).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| J04 | Main preferences window (settings pages of all plugins together) | ✗ | Settings page mount slots `plugin.settings.page/show` (recommended approach) |
| H10 | Theme / color scheme switching | ✗ | Theme becomes settable by the host; `plugin.appearance.*` |

### Menus and shortcuts

| Menu path | order | group | Default binding |
| --- | --- | --- | --- |
| Settings ▸ Options… | 830 | settings-main | F5 |
| Settings ▸ Appearance ▸ Vestra Dark (radio, default) · Vestra Light | 840, 841 | settings-look | |

Number range allocation for the "Settings ▸" submenu (across the seven chapters of this document): Options… 830 (preferences); Keyboard Shortcuts… 832, Mouse… 833, Ignore Default Shortcuts 834 (hotkeys); Plugin Manager… 836 (plugins); Appearance ▸ 840–849 (preferences color schemes 840–841; visualizer's "Show Logo When Idle" 845). F5 is not taken. On macOS laptops F5 needs the fn key.

### J04 Main preferences window

**Requirement**
- F5 opens the "Options" window:
  - On the left a category tree in a fixed order: General / Playback / Subtitles / Video / Audio / Keys and Mouse / Appearance / Plugins / Other;
  - Under each category, the pages registered by plugins (page titles given by the plugins, sorted within a category by `order`, then by plugin id);
  - On the right, the selected page.
- Page changes take effect immediately, with no "OK / Cancel / Apply" (UI-TOOL-04); each page provides its own "Default" buttons.
- A fixed button at the bottom of the window: `Close` on the right.
- The last opened page is remembered; the next F5 shows it directly (the first page if it no longer exists).
- Plugins can open their own page in the preferences window directly from their own menu items; for example subtitles' "Subtitle Settings…" could later open the page under the "Subtitles" category.

**Entry point**: Settings ▸ Options… (F5). Other plugins can open it with `plugin.settings.show(<own page id>)` too.

**Approach comparison**

| Approach | How | Inter-plugin communication | Core work | Experience |
| --- | --- | --- | --- | --- |
| **A. Core settings page mount slots (recommended)** | A plugin registers a Slint page with `plugin.settings.page(file, {category, title})` (exporting a component + global like `panels.mount`, with properties and callbacks connected only to the registering plugin); the core provides the "Options" container window (category tree + page area) and composes the plugins' pages by category | None: each page only talks to its own plugin, and the container is provided by the core | Medium: reuses the composition and callback forwarding of panel mounting (`composition.rs`), adding a window-type mount slot and a category tree | All settings in one window, opened with F5; new plugins' pages appear automatically |
| B. Preferences only as entry navigation | The preferences window lists "<plugin> settings…" entries that open each plugin's own tool window | Needs the preferences plugin to run other plugins' commands, **violating the hard constraint**; if the core drew the entry list instead, it would essentially be a menu | Small | Jumping between windows, settings scattered |
| C. Menu convention only | Every plugin puts its settings entry in the "Settings ▸" submenu | None | Zero | No main window, so J04 is not actually implemented |

**A is recommended**. Reasons:
- Only A truly achieves "settings in one place" without communication;
- All the core adds is a general mount slot, with no business logic and no knowledge of plugin identities;
- Whichever plugin a page is in holds its state and saving, consistent with today's panel model.

**Transition**: before A lands, preferences implements approach C first. "Options…" opens preferences' own tool window containing only the appearance page; the other plugins' settings are still reached from their own menus. After A lands, preferences registers that page with `settings.page` and changes "Options…" to `settings.show()`.

**Behavior details (approach A)**
- The container window is created and owned by the core, with the window title "Options". Calling `show()` again brings the window to the front and switches to the given page.
- When a page's plugin is disabled, the core removes the page; if it was showing, it switches to the first page of the same category, or to the first "General" page when the category is empty; with no pages at all it shows "Nothing to set here". When the plugin is re-enabled, the page comes back (the plugin registers it again on activation).
- Categories with no pages are not shown in the category tree.
- When a page is taller than the page area, the core provides vertical scrolling; the width is fixed, and pages must be designed for 440px width.
- Keyboard: input boxes in pages work as usual; bindings do not take effect inside the window (the same as tool windows); ⌘W closes the window.
- `show(pageId)` can only name the plugin's own pages (to prevent plugins from navigating to each other and forming hidden coupling); without an argument it opens the last page.
- Opening the window does not affect playback (interaction requirements §3).

**Interface (core container, approach A)**: window 640×480, minimum 600×420, resizable. Category tree on the left, 160px wide (background `Theme.bg-list`, categories 12px, pages 11px indented 12px, row height 20px, selected background `Theme.selected-bg`); page area on the right with 14px padding (`dialog-padding`), page title at the top 13px bold, 24px high; bottom button row 40px high, buttons 21px.

**Persistence**: the last opened page is stored by the core in `settings.lastPage` of `ui-state.json`. Each page's settings are saved by its own plugin.

**Core dependencies**
- [New core API]

  ```ts
  type SettingsCategory = "general" | "playback" | "subtitles" | "video" | "audio" | "input" | "appearance" | "plugins" | "other";
  interface Settings {
    page(file: string, options: { id: string; category: SettingsCategory; title: string; order?: number }):
      View & Disposable & { readonly ready: Promise<void> }; // needs ui.settings
    show(pageId?: string): Promise<void>;  // opens the options window; pageId can only be this plugin's page
  }
  // plugin.settings: Settings
  ```

  Rules: `id` is unique within the plugin; `title` 1–24 characters; at most 8 pages per plugin; page file rules are the same as for panels (export one component and one global, with names inferred by the SDK; no `component` / `global` options; pages have no `visible`, ruling R-S1); callbacks have no "bind before show" restriction.
  Reason: plugins cannot embed another plugin's interface in their own window; only the core can compose Slint components from several plugins (as it already does for the bottom bar and sidebar).

**Acceptance**
- Press F5 to open "Options": the left side has at least "Appearance" (preferences) and "Keys and Mouse" (hotkeys, if it provides pages); clicking a page switches the right side, and playback is not affected.
- Disable hotkeys: the "Keys and Mouse" category disappears within 1 second; it comes back after re-enabling.
- Switch the color scheme on the appearance page: the main window changes immediately (see H10), with no "Apply" button.
- Close and reopen: the last page is shown.
- Transitional form (approach C only): F5 opens preferences' own window, containing the appearance page.

**Notes**: no "Apply" (changes take effect immediately); "initialize" and "export current configuration" are not done.

### H10 Theme / color scheme switching

**Requirement**
- Color schemes are distributed by the core and can be switched at runtime; all interfaces (main window, bottom bar, sidebar, context menu, all plugin tool windows, the preferences window) update **immediately**, without restarting or reopening windows.
- Two color schemes (2026-10 visual identity round, UI spec §2.1): **Vestra Dark** (`vestra`, default) and **Vestra Light** (`vestra-light`), both Aurora schemes taken from the full color spectrum of the logo: magenta accent, cyan for now playing, progress gradient amber → magenta → purple → cyan. The light scheme's video area stays dark.
- Plugin std-widgets are compiled with `fluent-dark`; when the light scheme is chosen, the host switches the std-widgets color scheme to light at runtime (`Palette.color-scheme`), and input boxes, drop-downs, checkboxes and scroll bars become light accordingly.
- "Appearance" page content:
  - Color scheme radio items, each with a 120×40 preview showing the video area and bottom bar backgrounds, progress fill, accent color and text color;
  - A "Restore Default" button (back to Vestra Dark, grey when already default).
  - No corner radius option: corner radii, font sizes and control sizes do not change with the color scheme and cannot be changed separately (ruling R-S2).

**Entry points**: Settings ▸ Appearance ▸ Vestra Dark / Vestra Light (radio, 840, 841); Options ▸ Appearance. Each color scheme added later gets one more radio item here.

**Behavior details**
- On switching, the on-screen message is `Palette: <scheme name>`.
- The choice is saved in the core and kept across launches. When preferences is disabled, the color scheme stays: the core holds the current value, and there is just no way to switch it for the time being.
- A slider being dragged or an open menu should not be interrupted by switching; if updating some view fails, it is retried at the next refresh without affecting other views.
- A "custom accent color" could be opened up in the future, but not this round; scheme ids are fixed by the core (`vestra`, `vestra-light`), and there is no scheme list interface (ruling R-S2).

**Persistence**: the core writes `appearance.theme` in `ui-state.json` (`"vestra"` or `"vestra-light"`; an unknown value falls back to `"vestra"`), and stores only this.

**Core dependencies**
- [New core API] Theme becomes a global settable by the host.
  - The colors and `accent-fill` in `theme.slint` are distributed by the host instead (with the current Vestra defaults), while `radius-*`, font sizes and dimensions stay constants;
  - The host calls `set_global_property` on the `Theme` global of every instance when each window, panel and menu instance is created and when the color scheme changes;
  - The color scheme tables live in the host (`src/host/ui/vestra/palettes/*.json` or Rust constants), each listing all color tokens;
  - Plugins cannot write `Theme`: the SDK's `set` / `bind` only act on the plugin's own components and bridge global.
- [New core API]

  ```ts
  type ThemeId = "vestra" | "vestra-light";                 // extended when schemes are added
  interface Appearance {
    readonly theme: ThemeId;                                 // synchronous, read-only, no capability needed
    onChange(listener: (theme: ThemeId) => void): Disposable;
    set(theme: ThemeId): Promise<void>;                      // needs ui.theme; other values invalid
  }
  // plugin.appearance: Appearance (reading needs no capability)
  ```

  Reason: Theme is a global shared by all plugins and the host interface. A plugin can only change its own components and cannot change the colors of other plugins' windows; color schemes must be distributed by the core to stay consistent.

**Acceptance**
- "Settings ▸ Appearance" has only one item, "Vestra", and it is selected; `set("vestra")` succeeds, other values return invalid.
- Temporarily add a second color scheme in a test and switch: within 1 second the bottom bar seek bar, the selected menu item and an open Audio Workbench window all change color; kept after a restart.
- grep for hard-coded colors in plugin `.slint` files (theme files excepted): the result is 0; otherwise leftover color blocks remain after switching, and the plugins have to be checked (already required by UI-THEME-01).
- After disabling preferences, the color scheme does not change.
- Switch the color scheme while dragging the volume slider: the slider is not interrupted.

**Notes**: no hue slider for overall tinting and no custom color schemes this round; no skins.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| `plugin.settings.page(file, { id, category, title, order? }): SettingsPage`; `plugin.settings.show(pageId?)`; capability `ui.settings` | Only the core can compose Slint pages from several plugins into one window |
| `Theme` color tokens become settable by the host and are distributed to all view instances | Theme is shared globally, and plugins can only change their own components |
| `plugin.appearance.theme/onChange/set` (fixed ids); capability `ui.theme` | The color scheme choice is core state |

---

## Plugin management (`plugins`)

> **Superseded (2026-10-26):** the Plugin Manager `vestra.plugins` and "Settings ▸ Plugin Manager…" were removed; plugins are managed in the store's Installed tab ([project history](../process/rounds/history.md#plugin-platform)). This chapter is kept as history.

**Location**: `src/plugins/plugins/` (removed)

### Scope

The plugins plugin provides the "Plugin Manager" window: it lists all plugins and can disable, enable, uninstall, roll back to the previous version, and open the data folder.

Current host state (read from `src/host/src`):

| Capability | Current state |
| --- | --- |
| Development loading | `--plugin <directory>`, repeatable; the README starts the product plugins this way |
| Managed install store | `package_store.rs`: `~/Library/Application Support/Vestra/packages` (`VESTRA_PACKAGE_STORE`); each id records two versions, `current` and `previous`, with a `revision`; journaled atomic commit and recovery; `gc` cleans up unreferenced directories |
| Startup | `package_manager.rs`: loads all committed `current` packages at startup |
| Exposed in the SDK | `packages.validate` → `trial` / `install`, and `rollback` (needs `plugins.manage`, used by workshop) |
| Wire method (not exposed in the SDK) | `packages.installed` (look up one record by id) |
| Disable | The host has an internal `disable(owner)` (withdraws resources, stops the process) but no external interface; `plugins.list/load/activation/disable` were removed on 2026-10-20 (architecture overview §3); the core UI rework kept no external disable entry point |
| Uninstall | Does not exist: the store has no operation to delete a record |
| Persisting the disabled state | Does not exist |

So J07 needs the core to redesign four interfaces: list, disable/enable, uninstall, change notification. The plugins plugin is only the interface.

Manifest capabilities: `commands`, `ui.window`, `ui.osd`, `plugins.manage` (an existing capability, extending the interfaces it covers).

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| J07 | Plugin list, disable, enable, uninstall (plus roll back, open data folder) | ✗ | `packages.list/onChange/setEnabled/uninstall` |

### Menus and shortcuts

| Menu path | order | group | Default binding |
| --- | --- | --- | --- |
| Settings ▸ Plugin Manager… | 836 | settings-plugins | (none) |

Once approach A lands, a "Plugin Manager" page with the same content as the window is also registered in the preferences "Plugins" category.

### Round 3a implementation notes

- `PluginInfo` dropped `dataDirectory` and `self` under ruling R-G1: the data folder is derived as `dirname(plugin.dataDirectory)/<id>`, and self is decided by `info.id === plugin.id`; `list` / `onChange` need no capability. The type draft under "Core dependencies" below is superseded by reference §12.
- The core does not create data folders in advance for disabled plugins: when the folder does not exist, "Open Data Folder" shows "The data folder does not exist", and the plugin does not create other plugins' folders.
- "Reload" and "Roll Back" are also disabled on its own row (rolling back or reloading itself would restart the manager window in the middle of the operation).

### J07 Plugin management

**Requirement**
- **List**: one row per plugin, five columns:
  - Plugin id
  - Version ("Dev" for development-loaded plugins without a version)
  - Source: installed / development / trial
  - State: running / starting / disabled / failed / stopped
  - Data size
- **What is listed**: running plugins, installed but disabled plugins, and plugins that failed to start must all be listed.
- **Details area** (when a row is selected):
  - The declared capabilities, each with a plain-language description, for example `playback.control Control playback`, `plugins.manage Manage plugins`, `bindings.manage Change shortcuts`;
  - The package directory and data folder paths;
  - The previous version number (when there is one);
  - The most recent failure reason (when there is one).
- **Action buttons**:
  - `Disable` / `Enable` (switching with the state)
  - `Uninstall…`
  - `Roll Back` (available when there is a previous version)
  - `Open Data Folder` (Finder)
  - `Reload` (disable and immediately enable, to recover after a plugin error)
- The disabled state is kept across launches: a disabled installed plugin is not loaded at the next launch.

**Entry point**: Settings ▸ Plugin Manager…

**Behavior details**
- **Disable**:
  - The core withdraws all of the plugin's resources (menus, bindings, windows, panels, filters, subtitle layers, resolvers, playback sequence provider, mark layers, settings pages) and stops the process.
  - Media it opened **keep playing** (API reference §3.2); its processing effects (filters, subtitle styles) are withdrawn.
  - Confirm before disabling: "After disabling vestra.audio, its equalizer and other effects are withdrawn immediately."
  - Message `Disabled vestra.audio`.
- **Enable**: the core starts the plugin again; the state goes through "starting" to "running"; when activation fails, the state is "failed" and the details area shows the reason.
- **Cannot disable itself**: the "Disable" and "Uninstall" buttons on the plugins plugin's own row are disabled, with the hover explanation "Plugin Manager cannot disable itself". The core refuses too (`invalid`), so users cannot turn off the management entry point and then be unable to recover.
- **Development-loaded plugins** (`--plugin`):
  - Can be disabled and enabled, but only for this run (the next launch is decided by the command line); the details area explains "Development loading: disabling only applies to this run";
  - Cannot be uninstalled or rolled back (buttons disabled).
- **Uninstall**:
  - Only for installed plugins. The confirmation box has the checkbox "Also delete its settings and data" (unchecked by default).
  - The core disables it first, then removes the `current` and `previous` records from the install store. The package directory is reclaimed by `gc`. When deleting data is checked, `plugin-data/<id>/` is deleted as well.
  - The plugin's shortcut entries in the user overlay are kept (becoming orphans, which can be cleared in J01), so they come back after reinstalling.
  - Message `Uninstalled <id>` (e.g. `Uninstalled zysilm.vestranes`).
- **Roll back**: calls the existing `packages.rollback(id)`, and the interface follows the states of `PackageChange` (preparing / activating / done / failed). On failure the message is shown.
- **Plugins on trial**: shown only, with no actions (managed by workshop).
- **Liveness**: subscribe to `packages.onChange`; installs, uninstalls and state changes (including new plugins installed by workshop) show up in the list within 1 second.
- While an install or rollback is in progress (`PackageChange` not finished), all of that plugin's buttons are disabled with `reason: "Updating"`.
- **Open Data Folder**: an `open <path>` child process; when the folder does not exist, the core creates it first; on failure the message is "Data folder unavailable".
- **Data size**: the plugin walks the data folder itself to compute it, for at most 2 seconds, showing "—" on timeout. Reading the size of other plugins' folders is only a statistic and does not read their contents; if review considers this overstepping too, `PluginInfo.dataBytes` will be provided by the core instead.

**Interface**: since round 3b this is a settings page in the options window (id `plugins.manage`, category "Plugins", 440 wide; the round 3a tool window 600×420 has been removed, and closing uses the options window's "Close"). The column widths below are for the original window form; the page form uses id adaptive (about 189) / version 50 / source 56 / state 56 / data 50, detail labels 64, and the button row has no "Close".
- Top list (220px high): five columns (id adaptive / version 70 / source 70 / state 70 / data 60), row height 18px; "Disabled" rows use `Theme.text-dim` text, and the state column of "failed" uses `Theme.accent`.
- Middle details area: a two-column table, labels 80px, one capability per line.
- Bottom button row: buttons 21px; on the left `Disable/Enable` `Reload` `Roll Back` `Uninstall…`, on the right `Open Data Folder` `Close`.

**Persistence**: the disabled set is persisted by the core (for installed plugins, written to the catalog in the install store directory, or to `disabledPlugins` in `ui-state.json`, as the core decides). The plugins plugin saves nothing extra.

**Core dependencies**
- Existing: `packages.rollback`, `PackageChange`.
- [New core API]

  ```ts
  interface PluginInfo {
    id: string; version: string | null;
    origin: "installed" | "development" | "trial";
    state: "starting" | "running" | "disabled" | "failed" | "stopped";
    error: string | null;                 // most recent failure reason
    capabilities: string[];
    directory: string; dataDirectory: string | null;
    previousVersion: string | null;       // previous in the install store
    self: boolean;                        // is the caller itself
  }
  interface Packages {
    // existing: validate, rollback
    list(): Promise<PluginInfo[]>;                                         // needs plugins.manage
    onChange(listener: (plugins: PluginInfo[]) => void): Disposable;
    setEnabled(id: string, enabled: boolean): Promise<void>;               // installed: persistent; development: this run; self: invalid; trial: unsupported
    uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>; // installed only; self: invalid
  }
  ```

  Reasons:
  - The plugins' running state, install records and disabled state are all in the core;
  - Disabling needs to withdraw all of the plugin's resources on the host side, which plugins cannot do;
  - Uninstalling needs to change the journaled install store (atomic commit in `package_store.rs`), and the core is the only writer of that store.
  `PluginInfo` is the same type as I09's `AppInfo.plugins` (I09 needs no capability to read it).

**Acceptance**
- Open the window: it lists the 10 product plugins and their states, matching I09 "About".
- Disable audio: "Audio ▸ Audio Workbench…" disappears from the menu, Shift+E does nothing, and the music playing does not stop; everything comes back after enabling.
- Disable an installed plugin and restart Vestra Player: it is still disabled and was not loaded.
- Uninstall a test plugin installed by workshop (without deleting data): it disappears from the list and the install store has no record; after reinstalling, its settings are still there. With "Also delete data" checked, `plugin-data/<id>` does not exist.
- The disable and uninstall buttons on the plugins plugin's own row are disabled; calling `setEnabled(self, false)` directly returns `invalid`.
- Development-loaded plugins: the uninstall button is disabled; after disabling and restarting Vestra Player, it is loaded as usual.
- Roll back: for a plugin with a previous version, after clicking "Roll Back" the version number becomes the previous version.

**Notes**: designed for Vestra's plugin model; the list and button layout follow the common "add / remove / settings / info" list dialog form.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| `plugin.packages.list(): Promise<PluginInfo[]>` | The plugins' running and install state is in the core |
| `plugin.packages.onChange(listener)` | Refresh on install, disable and failure |
| `plugin.packages.setEnabled(id, enabled)` (persisting the disabled set) | Disabling must withdraw host-side resources; the external disable interface was removed and has to be redesigned |
| `plugin.packages.uninstall(id, { removeData? })` | Only the core can write the install store, and there is no delete operation yet |

---

## Album art and idle screen (`visualizer`)

**Location**: `src/plugins/visualizer/` (new plugin, to be created)

### Scope

The visualizer plugin owns "what the video area shows when there is no picture":

- **During audio-only playback** (J11): album art and track information (a placeholder when there is no cover);
- **When idle** (H12): with no source open, the brand logo centered;
- **While loading** (interaction-spec.md G-16b): while the source has `phase === "loading"`, a spinner and "Opening: <source name>".

> The loading behavior has been superseded by G-16a / G-16b in the [interaction spec](interaction-spec.md): the core makes the backdrop layer effective during `loading`, and visualizer draws the loading screen (74×74, 12 ticks, a spinner advancing one tick every 50 ms, center moved up 30 px; below it bold 15 px `Theme.text-dim` text, moved down 30 px); on error it goes back to the picture from before loading; it is mutually exclusive with the logo and cover, and not affected by the two toggles. "`loading` keeps the previous state" and "the logo stays during `loading`" below are void.

All of these have to be drawn in the video area, which the host draws (`Image { source: root.video-frame }` in `host.slint`), while plugins can currently only mount into the bottom bar and the sidebar. So the core needs a new "video area backdrop layer" mount slot: when there is no video picture, the core shows a plugin panel in the video area. In addition, the cover is an image obtained at runtime, which needs the image property `plugin.ui.image` (first proposed in I07 of the playback chapter).

Boundaries:
- The backdrop layer is display only and does not receive mouse or keyboard input. Double-click fullscreen, the context menu, the wheel and drag and drop are still handled by the host video area.
- Covers and tags are read by the plugin itself: extracted with ffprobe and ffmpeg, or by reading `cover.jpg` etc. in the same folder.
- No animations that follow the audio, such as spectrum or waveform, so the core does not need to provide live audio data.

Manifest capabilities: `commands`, `playback.read`, `events`, `ui.mount`, `ui.dialog` (choosing a custom logo), `ui.osd`.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| J11 | Show album art for audio-only media (placeholder without a cover) | ✓ (3b) | Backdrop mount slot `panels.mount("backdrop")`; image property `plugin.ui.image` |
| H12 | Show the logo when idle | ✓ (3b) | Backdrop mount slot (the logo uses compile-time `@image-url`; a custom logo needs the image property) |

### Menus and shortcuts

| Menu path | order | group | Default binding |
| --- | --- | --- | --- |
| Audio ▸ Show Album Cover for Audio Only (checkbox) | 530 | visual | |
| Settings ▸ Appearance ▸ Show Logo When Idle (checkbox) | 845 | settings-logo | |
| Settings ▸ Appearance ▸ Choose Logo Image… | 846 | settings-logo | |
| Reset ▸ Idle Logo (moved to Reset ▸ 2026-11-01, see menus.md) | 850 | reset-appearance | |

In the "Audio ▸" submenu, playback uses 501–503, audio uses 510–529, and this plugin uses 530–539. The album cover goes in the audio domain because it only appears when audio is playing. The logo goes in "Settings ▸ Appearance", next to the color schemes. No default shortcuts.

### J11 Show album art for audio-only media

**Requirement**
- When the source is ready, the toggle is on (on by default), and there is no video picture (an audio-only file, or audio with only a cover image), the video area shows the album cover:
  - The cover is centered, with a side length of 60% of the video area's short side, at most 480px, keeping its ratio, corner radius `radius-panel`, with a 1px `Theme.line` border;
  - 3 lines of text below the cover: title (13px bold, `Theme.text`) / artist (11px, `Theme.text-list`) / album · year (11px, `Theme.text-dim`);
  - With no tags, the title is the file name, and the other two lines are left out.
- With no cover, a placeholder is shown: the brand logo at 40% opacity in the cover position, with the text shown as usual.
- No spectrum, waveform or other animation.

**Entry point**: "Audio ▸ Show Album Cover for Audio Only" (checkbox).

**Behavior details**
- **When to show**: whether the backdrop layer shows is decided by the core (`panels.backdrop.state.effective`); what to draw is decided by the plugin (ruling R-U3, no `mode`): draw the cover when the source is `ready` / `ended` and `stats().video === null`.
- **Known behavior** (ruling 3.2 item 8): for audio with an embedded cover, or with a `cover.jpg` / `folder.jpg` etc. in the same folder that mpv loads by default, mpv shows the cover as a video track (`stats().video` non-empty, `effective = false`), so what is seen is the cover rendered by mpv, without tag text. This plugin's cover screen therefore only appears for audio with no cover at all (placeholder + tags); extracting embedded and folder covers is kept and takes effect once the core hands covers to the backdrop layer.
- **Cover sources**, the first available one in order:
  1. Embedded cover: `ffmpeg -v error -nostdin -i <path> -map 0:v:0 -frames:v 1 -an -f image2 <cache>/cover.jpg`; extracted only when ffprobe shows `disposition.attached_pic = 1`, timeout 10 seconds;
  2. `cover.jpg|png`, `folder.jpg|png`, `front.jpg|png` in the same folder, case-insensitive;
  3. The placeholder.
- **Network sources**: only the placeholder and the file name from `source`, without running ffmpeg.
- **Reading tags**: `ffprobe -v error -show_entries format_tags=title,artist,album,date -of json <path>`.
- **Cache**:
  - The key is `sha1(realpath + size + mtime)`;
  - Directory `<dataDirectory>/cache/covers/<key>.jpg`, with tags stored in `tags.json` in the same directory;
  - Limit 200 MB or 30 days, evicted by `lastUsed`.
- **Track change**: when `sourceId` changes, running extraction processes are aborted. Until the new cover is ready, the placeholder is shown, then it switches with a 300 ms fade-in (Slint `animate`, not pushed frame by frame from Node).
- **Turning the toggle off**: takes effect immediately. When off, the video area during audio-only playback is the plain `bg-video` color (as now). On toggle, message `Album cover for audio only: On/Off`.
- **In fullscreen**: shown the same way, sized for the fullscreen video area.
- Failure to read the cover image: the placeholder is shown, without a message.

**Interface**: the backdrop component fills the video area and has no interactive controls. Layout as above, with text at most 80% of the video area width, truncated when too long. At the minimum video area (about 318×120), only the cover (shrunk) or only the title is shown, whichever does not overflow: below 200px height the artist and album lines are hidden.

**Persistence**: `<dataDirectory>/visualizer.json`: `{ "audio": { "on": true }, "idle": { "on": true, "logo": null } }`.

**Core dependencies**
- [New core API] video area backdrop mount slot:

  ```ts
  plugin.panels.mount("backdrop", file, options?): Panel           // needs ui.mount; only one backdrop layer at a time, a second fails with conflict
  plugin.panels.backdrop: {
    readonly state: { effective: boolean } | null; // null: no backdrop layer (ruling R-U3 removed mode)
    onChange(listener: (state) => void): Disposable;
  }
  ```

  Rules:
  - The core shows the backdrop layer in two cases: `phase` is `empty` (nothing opened, or closed); `ready` / `ended` with no video output (a cover video track counts as output). The plugin uses `phase` and `stats().video` to tell idle from audio-only;
  - Shown during `loading` (G-16a; the former "keep the previous state" is void); keeps the previous state on `error`;
  - Hidden when there is a video picture, `effective = false`;
  - The backdrop layer is drawn where the video image is, **below** the host input layer (TouchArea), and receives no pointer or keyboard events;
  - On-screen messages and the info block are drawn above it;
  - Unmounted when the plugin is disabled, and the video area goes back to a plain color.
  Reason: the video area is drawn by the host, and plugins can only mount into the bottom bar and sidebar, with no way to show content in the video area. This is a general mount slot with no business logic, and the core knows best how to judge "idle" and "audio only" (whether there is video output).
- [New core API] image property `plugin.ui.image(path): ImageRef` (defined in I07 of the playback chapter), used to show covers. The placeholder uses compile-time `@image-url` and does not need it.

**Acceptance**
- Play an MP3 with an embedded cover: the video area shows the cover and "title / artist / album", matching Finder's "Get Info"; for a FLAC folder without a cover that has `cover.jpg`, that is shown.
- Audio with no cover at all: the semi-transparent brand logo placeholder and the file name are shown.
- Double-clicking the cover screen still toggles fullscreen, right-click still opens the menu, and the wheel still changes the volume.
- The cover is not shown while playing a video file; after switching from audio to a video file, the video picture is back within 1 second.
- With the toggle off, the video area is a plain color during audio-only playback.
- After disabling visualizer, the video area goes back to a plain color and playback is not interrupted.

**Notes**: only "show the album image"; no visualization animations and spectrum, floating ball, image folder slideshow, or downloading visualizations.

### H12 Show the logo when idle

**Requirement**
- When no source is open (after startup, after closing the source, after the playlist ends and the media is closed), the video area shows the logo centered:
  - By default the brand logo (`src/assets/brand/logo.png`), with a side length of 30% of the video area's short side, limited to 64–256px, keeping its ratio;
  - "Vestra Player" 12px below it (13px, `Theme.text-dim`);
  - Background `Theme.bg-video`.
- A custom logo image (PNG/JPEG) can be chosen; restoring the default goes back to the brand logo.
- The toggle "Show Logo When Idle" is on by default. When off, the idle screen is a plain color (as now).

**Entry points**: Settings ▸ Appearance ▸ Show Logo When Idle / Choose Logo Image…, Reset ▸ Idle Logo (Settings ▸ Appearance ▸ Restore Default Logo until 2026-11-01); also a "Logo and Album Cover" page registered in the "Appearance" category of the options window (`settings.page`, id `logo-cover`, order 20; with the J11 toggle, the logo preview with choose/restore, and the cover cache size with clearing).

**Behavior details**
- When to show: the source has `phase === "empty"` (decided by the plugin). During `loading` it changes to the loading screen (G-16b; the former "keep the logo" is void); once the source is ready, the video picture or the J11 layer takes over.
- **Built-in logo**: at build time `src/assets/brand/logo.png` is copied into the plugin package and used in Slint with `@image-url("logo.png")`, without depending on the runtime image capability. So H12's default form depends only on the backdrop mount slot.
- **Custom logo**:
  - After choosing an image with `dialogs.openFile`, check the extension and file header (PNG/JPEG); reject files over 16 MB or larger than 4096×4096: "Image too large (max 4096×4096, 16 MB)";
  - After the check passes, copy it to `<dataDirectory>/logo.<ext>` (not under `cache/`, so it is not cleaned up as cache), and show it with `plugin.ui.image()`;
  - Deleting or moving the original file later has no effect.
- When the custom image cannot be read, it falls back to the built-in logo with a one-time message `The logo image cannot be read; using the default logo`.
- When the window size changes, the logo scales proportionally in real time; below 120px video area height the words "Vestra Player" are hidden.
- On toggle from the menu, message `Logo when idle: On/Off`.
- The backdrop layer takes no input: when idle, double-click still toggles fullscreen and right-click still opens the menu.

**Interface**: in the backdrop component, the idle layout is as above; it shares one component with J11, and the plugin switches the content by source phase, with a 200 ms cross-fade.

**Persistence**: `idle` in `visualizer.json`: `{ "on": true, "logo": null | "logo.png" }`.

**Core dependencies**
- [New core API] the backdrop mount slot (same as J11).
- [New core API] the image property (needed only for a custom logo).

**Acceptance**
- Start Vestra Player without arguments: the brand logo and "Vestra Player" are shown in the center of the video area, clear and undistorted on the dark background.
- After opening a file the logo disappears; after closing with F4 it reappears.
- Drag the window down to the minimum: the logo shrinks to 64px without overflowing; in fullscreen at most 256px.
- Choose a custom PNG: it replaces the display immediately; after deleting the original and restarting it is still shown; after Reset ▸ Idle Logo it is back to the brand logo.
- With the toggle off, the idle screen is a plain color.
- When idle, double-clicking the video area toggles fullscreen.

**Notes**: no "logo during playback" (a flash when playback starts), no random choice from a logo folder, no position and opacity adjustment; the size is computed automatically from the video area.

### Chapter core API request summary

| Interface | Reason |
| --- | --- |
| `plugin.panels.mount("backdrop", file, options?)`; `plugin.panels.backdrop.state/onChange` (only `effective`, no input; delivered in round 3b) | The video area is drawn by the host, and plugins have nowhere to show content; only the core knows whether there is video output |
| `plugin.ui.image(path): ImageRef` (same as I07 in the playback chapter) | Show covers obtained at runtime and custom logos |

---

## Online subtitles (`subtitle-online`)

**Location**: `src/plugins/subtitle-online/` (new plugin, to be created)

### Scope

A new plugin for searching and downloading subtitles online, which needs network services (D16). Why it is a plugin of its own: it has to handle provider accounts, API keys, quotas, privacy notices and network failures, none of which have to do with local subtitle operations; users can choose not to install it.

Boundaries:
- Downloaded subtitles are loaded with the core's `subtitles.load` as tracks owned by this plugin. The subtitles plugin (`subtitles`) sees these tracks in the subtitle state and selects tracks and adjusts styles as usual; the two plugins do not communicate.
- This plugin does not use any paid service as a default. Every service that needs a key has to be filled in by the user; when nothing is configured, the feature stays off and leads to the settings window.
- Network requests are made with `fetch` inside the plugin's Node process, not through the core.
- Keys are stored in the system's secret store (`plugin.secrets`, reference §16.6: the macOS login keychain, with service name `vestra.subtitle-online` and the key name as account name; Windows Credential Manager); when the system has no usable store (`unsupported` / `unavailable`), it falls back to `secrets.json` in the plugin data directory (permissions 0600), and with no data directory they are only remembered for this run; the settings window states the storage location for the system ("macOS Keychain", "Windows Credential Manager", a file path, or "This run only").

Manifest: `commands`, `events`, `ui.window`, `ui.dialog`, `ui.osd`, `playback.read`, `subtitles.read`, `subtitles.control`.

### Feature overview

| ID | Feature | Current | Needs new core API |
| --- | --- | --- | --- |
| D16 | Online subtitle search and download | ✗ | Uses `subtitles.load` from the subtitles chapter (`format`, `title`) |

This plugin proposes no new core interfaces.

### Menus and shortcuts

| Menu path | order | Group | Default shortcut | ID |
| --- | --- | --- | --- | --- |
| Subtitles ▸ Online Subtitles ▸ Search and Download Subtitles… | 360 | subtitle-online | — | D16 |
| Subtitles ▸ Online Subtitles ▸ Online Subtitle Settings… | 361 | subtitle-online | — | D16 |

No default shortcuts.

### D16 Online subtitle search and download

**Requirement**: search online subtitles for the current video, list the results (language, release name, format, frame rate, download count, match method, source), download one or more, load and select them automatically after download, and optionally save them next to the video.

#### Service choice and trade-offs

| Service | State (at time of writing) | What it needs | Conclusion |
| --- | --- | --- | --- |
| OpenSubtitles.com REST API (`https://api.opensubtitles.com/api/v1`) | In operation, official documentation public; the old opensubtitles.org XML-RPC has been shut down | Every request carries an `Api-Key` (obtained by the application developer by registering an "API consumer" for free on the OpenSubtitles website) and a custom `User-Agent`; downloads need `POST /download`, login optional (`/login` gives a JWT; free accounts get more daily downloads than anonymous use); there is a daily download quota per user/IP, as given by `remaining` in the `/download` response | **Default service**. Supports file hash matching (moviehash), many languages, best result quality |
| Shooter (射手网, shooter.cn) | Closed in 2014, the original API is unavailable | — | Not done |
| Assrt (射手网(伪)) assrt.net API (`https://api.assrt.net/v1`) | In operation, many Chinese subtitles | Users register for free to get a token; rate-limited; downloads are single files or archives | **Optional service**, off by default; can be enabled after the user fills in a token |
| Xunlei (迅雷) subtitle interface (`api-shoulei-ssl.xunlei.com`) | Non-public interface, no documentation, no terms, may stop working at any time | No key needed (queried by file CID) | Not done (no undocumented interfaces) |
| SubDB, Addic7ed, Podnapisi, etc. | SubDB has closed; Addic7ed can only be scraped; Podnapisi has no formal public API | — | Not done |

**The default service's key question (for the maintainer to decide)**: the OpenSubtitles `Api-Key` identifies the **application**, and by their rules it is registered by the application developer and shipped with the application. Two options:
- (a) The project registers a "Vestra" consumer, and the key ships with the plugin (`config.json`; to keep it out of the public history of the code repository, it can be injected at build time); users can search anonymously and download a little without doing anything, with login optional.
- (b) No key ships; users fill in their own key in the settings the first time they use it.

(a) is recommended, as it works out of the box. Until the maintainer completes the registration, implement (b): without a key, "Search and Download Subtitles…" opens the settings window and explains how to apply for one. In both cases users may fill in their own key to override.

**Round 2 implementation (ruling on (a) pending)**: implemented as (b). The plugin ships no key; with no key or token filled in by the user, it makes no network requests at all.

#### Entry points

- `Subtitles ▸ Online Subtitles ▸ Search and Download Subtitles…`: opens the "Download Subtitles" window and immediately searches for the current video.
- `Subtitles ▸ Online Subtitles ▸ Online Subtitle Settings…`: services, accounts, languages, automatic search, save location.

#### Behavior details

1. **Search criteria**:
   - Local file: compute the OpenSubtitles moviehash (file size + the sum of the first and last 64 KiB as 64-bit little-endian integers, computed by reading the file in Node, < 50 ms), and send file name keywords as well (with the extension, resolution, codec, release group and other common markers removed).
   - Network source: only title keywords (the last path segment of `player.state.source` or the title given by the resolver).
   - The user can change the keywords in the "Title" box at the top of the window and click "Search by Title".
   - Languages: the preferred languages from the settings (default `zh-CN, zh-TW, en`, at most 5).
2. **Result list**: merge the results of all enabled services, sorted: hash matches first → preferred language order → download count descending. Machine-translated / AI-translated subtitles (OpenSubtitles fields `machine_translated`, `ai_translated`) go to the end and are marked "MT". Subtitles for the hearing impaired are marked "HI". 50 per page; "More" loads the next page.
3. **Download**:
   - Double-click or "Open Subtitle": download the selected one → detect format and encoding by the D18 rules → `subtitles.load(bytes, { format, title: "<language> · <release name>", select: "primary" })`.
   - Check several and click "Add": download and load them one by one with `select: "none"` (only added to the track list); when done, OSD "Subtitles added: 3".
   - "Save Only": download the checked items to the save location without loading them.
   - "Save As…": save the selected one to a location the user chooses.
   - "Open in Browser": open the subtitle's web page. **Not implemented in round 2**: the general capability `plugin.system.openPath` only accepts existing local paths, and the plugin may no longer call `open` itself; a request for "`plugin.system.openUrl(url)`" has been filed, and the button will be added once it is accepted.
4. **Save location** (setting, default "Beside the video when possible"): for local videos, saved as `<video name>.<language>.<extension>`, without overwriting (the `(1)` rule); when the directory is not writable or the source is a network source, saved to `<plugin data directory>/downloads/`. Loading uses the downloaded bytes and does not depend on the save succeeding; on save failure, OSD "Subtitle loaded, but it could not be saved to the video's folder".
5. **Archives** (common on assrt): zip is unpacked inside the plugin (Node `zlib`, no npm dependency); when the archive has several subtitle files, they are listed for the user to choose; rar/7z are not supported, with the message "This archive format is not supported yet (rar, 7z); download a zip or a single subtitle file." (opening the web page waits for `system.openUrl`, see above).
6. **Quotas and failures**:
   - Quota used up (OpenSubtitles 406/429 + `remaining: 0`): OSD "Today's downloads are used up (OpenSubtitles). Log in to an account for more"; the window status line shows the reset time (`reset_time` in the response).
   - 401/403: "Invalid API key; check it in Online Subtitle Settings".
   - Network error / timeout (15 seconds per request): "Cannot connect to <service name>"; other services' results are shown as usual.
   - No results: "No subtitles found; change the title and search again".
   - Closing the window or changing the source while a request is in flight: aborted (`AbortController`), nothing is loaded.
7. **Automatic search** (setting, default "Never"; option "When opening a video without subtitles"): when a local video is opened and `state.tracks` has no text track, search in the background; when there is a hash-matched result in the first preferred language, handle it by the setting "After search: show the download window (default) / load the best result automatically". When loaded automatically, OSD "Online subtitle loaded automatically: <title>".
8. **Privacy**: a one-time notice in the settings window and before the first search: "Searching sends file name keywords and the file hash to the selected service"; after the user confirms, it is not shown again (recorded in the settings).
9. No media: the search window can still search by title; "Open Subtitle" is disabled with the reason "No media to load subtitles into".

#### Interface

- "Download Subtitles" window (560×360, minimum 480×280): at the top a "Title:" input box + "Search by Title" and "Search Again"; in the middle the result list (checkbox 20, language 60, release name adaptive, format 40, FPS 50, downloads 60, match 40, source 80), row height 20; buttons at the bottom (21px high): "Open Subtitle", "Add", "Save Only", "Save As…", "Open in Browser", "Settings…"; a status line.
- "Online Subtitle Settings" window (420 wide): group "Services" (list: enable checkbox + name + status; "Move Up", "Move Down", "Settings…", "Test"); group "Search" (preferred languages, automatic search, action after search, only exact file name matches, excluded keywords); group "Save" (save location). A service's "Settings…" pops up a small window: API key, user name, password (login optional); "Test" shows the remaining downloads.

#### Persistence

`<plugin.dataDirectory>/settings.json`: enabled services and their order, preferred languages, automatic search, action after search, excluded keywords, save location, privacy notice confirmed. Keys and passwords are stored in the system's secret store (see Scope). The JWT from login is kept only in memory and renewed by logging in again when it expires.

#### Core dependencies

- Existing: `player.state` (`source`, `kind`), `subtitles.state/load/select`, `windows.create`, `dialogs.saveFile`, `osd.show`.
- Uses the subtitles chapter's [New core API] `format` (when ASS/VTT is downloaded), `title` and `select` of `subtitles.load`. This plugin proposes no new core interfaces.

#### Acceptance

- With a valid key filled in, open a common film file: the search results include hash-matched items, sorted first.
- Double-click a result: the subtitle loads and is selected, with the track title "language · release name"; `<video name>.zh.srt` appears next to the video.
- With a wrong key, the status line wording is correct; with the network down, "Cannot connect" is given within 15 seconds.
- Simulate a 429 quota exhaustion: the message wording and reset time are correct.
- Change the source during a search: the results are not loaded into the new source.
- With no key filled in, opening search jumps straight to the settings window.

#### Notes

No subtitle upload, and no custom list of "web subtitle search links".

### Chapter core API request summary

This chapter adds no core interfaces. It relies on the `format` / `title` / `select` options of `subtitles.load` already listed in the subtitles chapter.

---

---

## Appendix A: Core API request summary

> **Note**: this table is the authors' request draft from before negotiation (community workflow §4.3). The final outcome of each item (rejected with an alternative / accepted after rewrite / accepted), the final signature and the round it lands in were settled in the negotiation of 2026-10-20 ([project history](../process/rounds/history.md#1-timeline); full table: `git show 03c75d2e:doc/process/rounds/2026-10-features-negotiation.md`); the authoritative description of implemented interfaces is the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html).

Date: 2026-10-20. Source: the "Core dependencies" and "Chapter core API request summary" of each chapter of this document; interfaces needed only by removed features (A04 A05 A06 A14 B10 B13 B14 B15 B18 B19 B20 B24 B27 B28 C08 C13 C19 C20 C22 C24 C25 C26 D08 D12 D13 D14 D15 D19 D21 E07 E14 E15 F06 F08 F10 F12 G06 G07 G12 G13 G14 H02 H06 H11 I04 J05 J10, and the three plugins files, scenes and timer) are not listed. An interface used by several chapters is listed once.

Column notes: "New capability" is the capability name the manifest has to add; "existing X" means an existing capability is reused; "—" means no capability is needed. Items marked "optional" or "conditional" can be skipped without blocking the matching feature.

### 1. Interface summary

#### 1.1 Player

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| P1 | `player.stepFrame(direction: "backward" \| "forward", opts?: Conditional): Promise<void>`; `PlayerState.can.frameStep: boolean`, `can.frameBackStep: boolean` | existing `playback.control` | playback B07; core basic actions `frame-back` / `frame-forward` | Frame stepping is an mpv command (`frame-step` / `frame-back-step`), and plugins cannot reach mpv |
| P2 | `player.stepKeyframe(direction: "backward" \| "forward", opts?: Conditional): Promise<void>` | existing `playback.control` | playback B08 | "Strictly pass the current keyframe" can only use mpv's `seek ±… relative+keyframes`; plugins do not know keyframe positions |
| P3 | `player.setStepSeek(options: { seconds: number }): Promise<void>` (0.1–3600, default 5; back to the default after the calling plugin is disabled) | existing `playback.control` | playback B06, basic action hand-over | After ←/→ are handed to the core, the step 1 size still has to be configurable, with the setting saved by playback |
| P4 | `plugin.host.run(action: HostAction): Promise<void>`; `HostAction = "play-pause" \| "stop" \| "previous" \| "next" \| "frame-back" \| "frame-forward" \| "seek-backward" \| "seek-forward" \| "rate-down" \| "rate-up" \| "rate-normal" \| "volume-up" \| "volume-down" \| "mute"` (fails with `unavailable` when the action is disabled) | existing `commands` | playback bottom bar buttons (basic action hand-over) | Bottom bar buttons and shortcuts share one behavior and message of the core actions; "running a core action" is not communication between plugins |
| P5 | `player.preload(source: string, options: { after: Media; signal?: AbortSignal }): Preload`; `interface Preload extends Disposable { readonly source: string; readonly started: Promise<Media> }` | existing `playback.control` | playlist B26 | Gapless switching can only be done by preloading inside mpv (`loadfile … append`, `prefetch-playlist`, `gapless-audio`), and local files also need the host to set up an fd lease in advance |
| P6 | `PlayerState.videoSize: { width: number; height: number } \| null` (display size, including DAR and rotation) | existing `playback.read` | window H03, H04, H07 | The display size has to be pushed with the state; `stats()` only has the decoded size and has to be queried |
| P7 | Extended `PlaybackStats` (all may be `null`): `video.decoder`, `hwdec`, `pixelFormat`, `colorspace`, `displayWidth/Height`, `displayFps`, `bitrate`; `audio.decoder`, `format`, `channelLayout`, `bitrate`; `avSyncMs`, `cacheSeconds` | existing `playback.read` | info I05; window H07 acceptance refers to `avSyncMs` | Live decoding state exists only in mpv, and ffprobe cannot see the session being decoded |
| P8 | `PlaybackStats.video.transfer: "sdr" \| "pq" \| "hlg" \| null` | existing `playback.read` | video E16 | Decide whether the source is HDR (mpv `video-params/gamma`) |

#### 1.2 Tracks and subtitles

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| T1 | `player.addAudioTrack(path: string, opts?: { select?: boolean; ifCurrent?: PlayerState }): Promise<AudioTrack>` | existing `playback.control` | audio A11 | Adding a track to the playback session is an operation inside the player (mpv `audio-add`) |
| T2 | `player.selectTrack(kind: "audio", id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>` (`kind` is only `"audio"` for now, kept for extension) | existing `playback.control` | audio G03, G04 | Switching audio tracks is an operation inside the player (mpv `aid`) |
| T3 | `subtitles.selectSecondary(track: SubtitleTrack \| null, opts?: { ifCurrent?: SubtitlesState }): Promise<SubtitlesState>`; `SubtitlesState.secondary: SubtitleTrack \| null`, `secondaryConfirmation: "waiting" \| "applied" \| "uncertain"` | existing `subtitles.control` | subtitles A08 | mpv `secondary-sid` can only be set by the host |
| T4 | `subtitles.load(bytes: Uint8Array, opts?: { format?: "srt" \| "ass" \| "ssa" \| "vtt"; title?: string; select?: "primary" \| "secondary" \| "none"; ifCurrent?; signal? }): Promise<SubtitleTrack>` (bounded ASS validation: UTF-8, ≤ 2 MiB, has `[Script Info]` and `[Events]`, Dialogue ≤ 20 000) | existing `subtitles.control` | subtitles A08, A09, D17, D18; subtitle-online D16 | The host only validates SRT; accepting ASS/VTT needs host-side validation and the correct temporary file extension; the track title and whether it is selected are decided by the host |
| T5 | `subtitles.cues(track: SubtitleTrack, opts?: { signal?: AbortSignal }): Promise<SubtitleCue[]>`; `SubtitleCue { start: number; end: number; text: string }` (text tracks only, at most 20 000 lines) | existing `subtitles.read` | subtitles B11 | Only the host can reliably extract the line-by-line content of embedded subtitles (the host already has `native_subtitle.rs`) |
| T6 | `SubtitleTrack.codec: string \| null`, `SubtitleTrack.text: boolean` | existing `subtitles.read` | subtitles A08, B11, D18 | Tell text and image tracks apart; the host already reads codec and only needs to expose it |
| T7 | New optional `SubtitleStyle` fields: `bold?: boolean`, `shadow?: number` (0–4), `box?: boolean`, `boxOpacity?: number` (0–100), `assOverride?: "keep" \| "force"`, `secondaryPosition?: "top" \| "aboveMain"` | existing `subtitles.control` | subtitles D09, D11, D17, A08 | mpv style properties (`sub-bold`, `sub-shadow-offset`, `sub-border-style`, `sub-ass-override`, `secondary-sub-pos`) can only be written by the host |
| T8 | (optional) widen the `SubtitleStyle.offsetMs` range to −600 000–600 000 | — | subtitles D05 | ±10 seconds is not enough to correct intro differences; mpv `sub-delay` has no such limit |

#### 1.3 Picture and rendering

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| V1 | `VideoNode` gains `{ kind: "hue"; degrees: number }` (−180–180, runtime) | existing `processing.video` | video E02 | FFmpeg eq has no hue; plugins cannot write filter strings themselves |
| V2 | `VideoNode` gains `{ kind: "sharpen"; luma; chroma }`, `{ kind: "cas"; strength }` | existing `processing.video` | video E04 | Sharpen, edge sharpen |
| V3 | `VideoNode` gains `{ kind: "denoise3d"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }`, `{ kind: "temporalDenoise"; threshold }` | existing `processing.video` | video E05 | 3D / progressive / temporal denoise |
| V4 | `VideoNode` gains `{ kind: "deblock"; strength: "weak" \| "strong" }`, `{ kind: "deband"; threshold; range }` | existing `processing.video` | video E06 | Deblock, deband |
| V5 | `VideoNode` gains `{ kind: "flip"; horizontal; vertical }`, `{ kind: "transpose"; degrees: 90 \| 180 \| 270 }` | existing `processing.video` | video E08, E09 | The software renderer does not support `video-rotate`; the host already handles orientation with filters |
| V6 | `VideoNode` gains `{ kind: "deinterlace"; method: "bwdif" \| "yadif"; when: "interlaced" \| "all"; output: "frame" \| "field" }` | existing `processing.video` | video E10 | The node is withdrawn with the plugin's lifecycle, which the global `deinterlace` property cannot do |
| V7 | `VideoNode` gains `{ kind: "crop"; aspect; x; y }`, `{ kind: "pad"; aspect; x; y }` (sizes computed by the core from the input) | existing `processing.video` | video E12 | The plugin does not know the filter input size |
| V8 | `VideoNode` gains `{ kind: "frameBlend"; fps: number }` (24–120) | existing `processing.video` | video E18 (frame doubling) | Blending interpolation needs a new filter node |
| V9 | `plugin.video.view(): VideoView`; `set(Partial<{ aspect: number \| null; fit: "keep" \| "stretch" \| "fill"; zoom; scaleX; scaleY; panX; panY }>)`, `status`, `onStatus` | existing `processing.video` | video E11, E13 | These are mpv vo-level properties that the processing chain cannot express, and plugins cannot write mpv properties directly; the core restores the defaults when the plugin is disabled |
| V10 | `plugin.video.render(): RenderOptions`; `set(Partial<{ toneMapping; targetPeak; peakDetect; shaders: string[]; interpolation: boolean }>)` (before C-GPU, `status` is always `failed / unsupported`) | existing `processing.video` | video E16, E17, E18 (motion interpolation) | Corresponds to mpv GPU render properties; prerequisite is host behavior H3 (C-GPU) |
| V11 | (optional) `Snapshot.export({ format: "jpeg"; destination; quality?; sampleTime? })` | existing `media.control` | capture F07 | Delivered in portability round P1 (`jpeg`, `bmp`), replacing the plugin's macOS `sips` conversion |

Every new `VideoNode` has to support both the live mpv graph and the offline export graph (shared `render_lavfi`), and is bound by the limit of "64 nodes for all plugins together"; the video plugin uses about 15 nodes in the worst case.

#### 1.4 Audio

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| A1 | `AudioNode` gains `{ kind: "channels"; mode: "stereo" \| "left" \| "right" \| "mono" \| "swap" }` (with more than 2 channels the core downmixes to stereo first; adjustable at runtime) | existing `processing.audio` | audio G05 | Plugins cannot add filters themselves; downmixing is the core's job |
| A2 | `player.audioDevices(): Promise<AudioDevice[]>`; `AudioDevice { id: string; name: string }` | existing `playback.read` | audio G16 | The device list comes from the player's audio output |
| A3 | `player.setAudioDevice(id: string \| null): Promise<void>` (`null` = system default) | existing `playback.control` | audio G16 | Covers both audio paths, mpv and generated streams (cpal), switching back automatically after a disconnect |
| A4 | `PlayerState.audioOutput: { requested: string \| null; active: string \| null; devicesRevision: number }` | existing `playback.read` | audio G16 | The plugin shows the fallback state from it and re-reads the device list after plug / unplug |
| A5 | (optional) widen the audio filter `delayMs` range to −10000–10000 | — | audio G09, A11 | External audio tracks are often off by more than 2 seconds |

#### 1.5 Window and application

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| W1 | `window.state: WindowState` (`fullscreen`, `videoArea`, `onTop`, `pictureInPicture: { supported; active }`); `window.onChange(listener): Disposable` | `window.control` | window H03–H05, H07 | Decide whether commands are available and their check state |
| W2 | `window.setVideoArea(size: { width; height }): Promise<{ width; height; clamped: boolean }>` | `window.control` | window H03, H04 | Only the host knows the bottom bar, sidebar and visible screen area |
| W3 | `window.setAlwaysOnTop(onTop: boolean): Promise<void>` | `window.control` | window H05 | The window level (NSWindow level) can only be set by the host, and has to apply to the floating sidebar and newly opened tool windows |
| W4 | `window.setCursorAutoHide(mode: "fullscreen" \| "playing"): Promise<void>` | `window.control` | window B30 | Pointer visibility and the idle timer live in the host event loop |
| W5 | `window.setPictureInPicture(on: boolean): Promise<void>` | `window.control` | window H07 | macOS picture-in-picture can only be started by the host through AVKit, and the frames exist only in the host; needs host behavior H8 |
| W6 | `plugin.app.info(): Promise<AppInfo>`; `plugin.app.onChange(listener): Disposable`; `AppInfo { version; build; apiVersion; components: { mpv; ffmpeg; slint; node }; plugins: PluginInfo[] }` | — (read-only public information) | info I05, I09 | The versions of the libraries the host links and the list of loaded plugins are core state |

All `window.*` setters are last-write-wins; when a plugin is disabled, the core withdraws the effects that plugin caused.

#### 1.6 Interface and input

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| U1 | `plugin.ui.image(path: string): ImageRef`; panel / window property value type `PropValue = Json \| ImageRef` (`ImageRef` is opaque and can only be created by that function; PNG/JPEG, ≤ 16 MB and ≤ 4096×4096, the path must be under this plugin's `dataDirectory`) | — | playback I07; visualizer J11, H12 (custom logo) | The UI bridge only accepts JSON (`ui_values.rs` rejects image), so plugins cannot show images generated or obtained at runtime |
| U2 | `plugin.drops.provide({ kinds: string[]; drop(e: DropEvent): unknown }): Disposable`; `Panel.onDrop(listener)`, `Panel.onDragOver(listener: (at \| null) => void)`; `DropEvent { paths: string[]; at: { x; y }; modifiers: { alt; shift; cmd } }` | `ui.drop` | playlist A15 | System drag and drop only reaches the host's native window; the video area belongs to no plugin, so the host has to dispatch by drop point and extension |
| U3 | `plugin.panels.mount("backdrop", file, options?): Panel`; `plugin.panels.backdrop: { state: { effective; mode: "idle" \| "audio" \| null } \| null; onChange }` (takes no input, one at a time) | existing `ui.mount` | visualizer J11, H12 | The video area is drawn by the host, and plugins have nowhere to show content; only the core knows whether there is video output |
| U4 | (conditional) a `PopupWindow` inside a panel that can extend beyond the panel and follow the pointer | — | playback I06, I07 | Only raised if the implementation finds the hover tooltip clipped by the panel |
| U5 | (optional) `plugin.commands.popup(path: string[]): Promise<void>` | existing `commands` | video E17 (the `S` key pops up the "Pixel Shaders" submenu) | Plugins cannot pop up main menu submenus; without it, `S` is not bound |

#### 1.7 Timeline and marks

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| M1 | `PlayerState.chapters: readonly Chapter[] \| null`; `Chapter { index: number; title: string \| null; start: number }` | existing `playback.read` | marks B21, B22 | Chapters are media information that mpv already reads; plugins cannot read them for network sources; shared by several plugins |
| M2 | `player.setLoop(loop: { a: number; b: number; count?: number } \| null, opts?: { ifCurrent?: PlayerState }): Promise<void>`; `PlayerState.loop: { a; b; count: number \| null; remaining: number \| null } \| null` (cleared by the core on source change) | existing `playback.control` | marks B23; playback draws the range (B22) | Precise looping can only be done by mpv `ab-loop-a/b/count`; with the state public, the seek bar can draw it |
| M3 | `plugin.timeline.marks`, `onChange(listener)`, `layer(): TimelineLayer`; `TimelineLayer.set(marks: { time; kind: "chapter" \| "bookmark" \| "marker"; title? }[], opts?: { ifCurrent? })` | `timeline.marks` (when publishing a layer) | marks B22, B23 (A point marker); playback draws | The seek bar belongs to another plugin, so marks can only be relayed through the core; a general data layer with no business logic |

#### 1.8 Bindings

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| K1 | `plugin.bindings.list(): Promise<{ commands: BindingInfo[]; orphans }>`, `set(ref, inputs: string[])`, `reset(ref?)`, `onChange(listener)` | `bindings.manage` | hotkeys J01, J02 | The binding table and the user overlay are both in the core; the overlay exists but writing is not exposed |
| K2 | `plugin.bindings.ignoreDefaults: boolean`, `setIgnoreDefaults(on: boolean): Promise<void>` | `bindings.manage` | hotkeys J03 | A switch in the resolution rules; unbinding entry by entry in a plugin would pollute the overlay and race |
| K3 | Bindable input `Mouse:LeftClick` (single-click decision delayed only when bound) | — | hotkeys J02 | Only the host can recognize video area pointer events and tell single from double clicks |

#### 1.9 Settings and appearance

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| S1 | `plugin.settings.page(file, { id; category: SettingsCategory; title; order?; component?; global? }): SettingsPage`; `plugin.settings.show(pageId?: string): Promise<void>` | `ui.settings` | preferences J04, H10; hotkeys J01, J02 (page form); plugins J07 (page form); visualizer H12 (page form) | Only the core can compose Slint pages from several plugins into one window |
| S2 | `plugin.appearance.themes`, `state`, `onChange`, `set(Partial<{ theme; corners }>)`; `ThemeInfo { id; title; preview; custom: false }` | `ui.theme` (for `set`; reading needs no capability) | preferences H10 | The color scheme choice is core state and has to be distributed by the core to all views; needs host behavior H6 |

#### 1.10 Plugin management

| # | Interface (signature) | New capability | Plugins and IDs using it | Reason |
| --- | --- | --- | --- | --- |
| G1 | `plugin.packages.list(): Promise<PluginInfo[]>`; `onChange(listener)`; `PluginInfo { id; version; origin; state; error; capabilities; directory; dataDirectory; previousVersion; self }` (the same type as W6's `AppInfo.plugins`) | existing `plugins.manage` (wider coverage) | plugins J07; info I09 (read through W6) | The plugins' running and install state is in the core |
| G2 | `plugin.packages.setEnabled(id, enabled): Promise<void>` (persisting the disabled set; self → `invalid`; trial → `unsupported`) | existing `plugins.manage` | plugins J07 | Disabling must withdraw all host-side resources; the former external disable interface was removed and has to be redesigned |
| G3 | `plugin.packages.uninstall(id, { removeData?: boolean }): Promise<PackageChange>` | existing `plugins.manage` | plugins J07 | Only the core can write the journaled install store, and there is no delete operation yet |

New capabilities in total: `window.control`, `ui.drop`, `timeline.marks`, `bindings.manage`, `ui.settings`, `ui.theme`.

### 2. Host behavior changes (not API)

| # | Change | Affects | Notes |
| --- | --- | --- | --- |
| H1 | **Basic playback actions handed over to the core** | playback (whole chapter); hotkeys J01 | The core registers 14 basic actions (play/pause, stop, previous/next, previous/next frame, step 1 back/forward, slower/faster/normal, volume ±, mute), with menu positions and default bindings unchanged; behavior and message text are moved over from `controller.ts`; bindings for `vestra.playback/<old id>` in the user overlay are mapped once to the new ids on upgrade. Works with P3 and P4. Whether `rate-10` is handed over too is for the maintainer to decide |
| H2 | **C-SIZE: render size follows the video area** | video E11, E13; window H07 (floating window resolution) | Software rendering is now fixed at 960×540 (`WIDTH/HEIGHT` in `backend.rs`) and scaled by Slint `image-fit: contain`; "Ignore Aspect Ratio", "Fill Window" and zoomed-in quality all depend on the render target matching the video area size (or ratio). Does not depend on C-GPU |
| H3 | **C-GPU: libmpv switches to GPU rendering** | video E16, E17, E18 motion interpolation; window H07 (optional zero copy) | Change from `RenderContext::new_software` to the libmpv OpenGL render interface + gpu-next / libplacebo (or an equivalent Metal path) embedded in the Slint video area; mpv's `tone-mapping`, `glsl-shaders` and `interpolation` only work in the GPU renderer |
| H4 | **Numpad key names in the binding syntax** | video E13 | `Numpad0`–`Numpad9`, `NumpadPlus`, `NumpadMinus`, `NumpadMultiply`, `NumpadDivide`, `NumpadEnter`, `NumpadDecimal`, distinct from the main keyboard |
| H5 | **macOS Option key recognition** | All default bindings with Alt: playback B06 (Ctrl+Alt+←/→); subtitles A09 (Ctrl+Alt+Y), D02 (Alt+L), D06 (Alt+PageUp/Down), D07 (Alt+Home), D09 (Alt+B), D10 (Alt+F); video E09 (Alt+K), E13 fallback keys (Alt+arrow keys, Alt+Plus/-, Alt+Backspace); audio G04 (Alt+A); capture F03/F04 (Ctrl+Alt+E/C); marks B23 (Alt+[, Alt+], Alt+\, Ctrl+Alt+[ / ]); the existing Alt+O, Alt+H | `bindings.rs::from_slint_key` recognizes keys by `event.text`; Option+key types a different character on macOS (⌥[ → `“`), and dead keys (⌥E, ⌥N, ⌥U, ⌥I, ⌥`) produce no character at all; the core should also recognize "the physical key name without modifiers". It also needs to be verified whether `Ctrl+[` arrives as `\u{1b}` and is recognized as `Ctrl+Escape` |
| H6 | **Theme becomes settable by the host** | preferences H10 | The colors, `accent-fill` and `radius-*` in `theme.slint` change from `out property` constants to `in property`; the host distributes them when each window, panel and menu instance is created and when the color scheme changes; the color scheme tables live in the host; plugins cannot write `Theme` |
| H7 | **Options container window** | preferences J04 | The core owns the "Options" window (category tree + page area), composes the pages registered by plugins with `settings.page`, reusing the composition and callback forwarding of panel mounting (`composition.rs`); remembers `settings.lastPage` |
| H8 | **Picture-in-picture frame output** | window H07 | The host creates `AVSampleBufferDisplayLayer` and `AVPictureInPictureController` (sample buffer content source, macOS 12+) and implements the playback delegate; during picture-in-picture each frame (RGB24 → BGRA) is wrapped in a timestamped `CMSampleBuffer` and enqueued; rendering keeps being driven at the video frame rate when the main window is covered or minimized; playback pauses when the floating window is closed |
| H9 | **Generated stream audio follows the output device** | audio G16 | `producer_audio.rs` (cpal) currently only opens the system default device; it has to use the device chosen with `setAudioDevice`, and switch back when a disconnected device reappears |
| H10 | **Persisted disabled state and external disable** | plugins J07 | Expose the internal `disable(owner)` (through G2); the disabled set of installed plugins is saved across launches, and they are not loaded at startup; the install store gains an operation to delete records (through G3) |
| H11 | **Single-click detection** | hotkeys J02 | The video area TouchArea in `host.slint` delays the single-click decision only when `Mouse:LeftClick` is bound (no second click within the system double-click interval, movement ≤ 4px); without a binding it keeps focusing immediately |
| H12 | **Configurable ASS override policy** | subtitles D17 | The `sub-ass-override=force` the host hard-codes at startup is controlled by `SubtitleStyle.assOverride` instead (default keep → `scale`) |
| H13 | **A-B loop reset across files** | marks B23 | mpv's `ab-loop-*` is kept across files by default, so the core clears it explicitly on source change (the M2 rule) |

### 3. Cross-chapter decisions

1. **Runtime image property**: unified as `plugin.ui.image(path): ImageRef`, with property values written as `ImageRef` (`PropValue = Json | ImageRef`, an opaque value that only this function can create). The original `interface ImageRef { $image: string }` of playback I07 has been rewritten; playlist C25's `{ "$image": path }` was removed with C25; visualizer already used `plugin.ui.image`, and its reference "see scenes" became "see I07 in the playback chapter". The limits are unified as PNG/JPEG, ≤ 16 MB and ≤ 4096×4096, with the path under the plugin's `dataDirectory`. The original I07 said ≤ 2 MB, which contradicted the 16 MB / 4096×4096 allowed for the H12 custom logo, so it was widened to the latter.
2. **Loop range**: unified as the marks chapter's `player.setLoop({ a, b, count? } | null, opts?)` and `PlayerState.loop`. The `{ start, end, count? }` signature originally in the subtitles chapter only served B24 and was removed with B24; the wording "reused by B24" in B22 and B23 has been removed too.
3. **Audio sync and subtitle sync shortcuts**: the audio chapter's proposal is adopted. Audio sync uses `>` earlier, `<` later, `?` reset (the physical keys Shift+. / Shift+, / Shift+/); subtitle sync changes from `>` `<` `/` to `.` earlier, `,` later, `/` reset. The subtitles chapter's menu table, conflict check, D05 wording and acceptance have been updated to match; the audio chapter dropped the alternative "use Ctrl+. / Ctrl+, / Ctrl+/ if subtitles do not change". Old overlay bindings are not migrated (there is no rebinding interface yet).
4. **Rendering terms**: "the render buffer changes with the video area size (now fixed at 960×540)" is uniformly called **C-SIZE: render size follows the video area**; "libmpv switches to GPU rendering" is uniformly called **C-GPU: libmpv switches to GPU rendering**. Both are marked as "host behavior changes", not API. The video chapter introduces both names in its Scope section, and window H07 uses the same names. The subtitles chapter's original "mpv render size follows the video area" only served D19 and was removed with D19.
5. **Audio track selection**: `player.selectTrack` keeps only `kind: "audio"` (E15 video streams removed); the signature moved from the video chapter to be defined in audio G03, and the `kind` parameter is kept for later extension.
6. **Channels**: G06 "swap left and right channels" is merged into G05 as the fifth option of the channel radio group, "Swap Left and Right". The `channels` node changed from `{ mode; swap: boolean }` to `{ mode: "stereo" | "left" | "right" | "mono" | "swap" }`, and the separate swap toggle in the "Processing" submenu and the window was removed.
7. **Chapter skipping**: B20 was originally to be merged into B21, but the B21 that remains only has the chapter / bookmark menu, jumping and bookmark management, without "skip by chapter name", so B20's content was removed entirely. The `PlayerState.chapters` definition originally written in B20 moved to B21's core dependencies.
8. **Single-list playlist**: after C19 (multiple tabs) was removed, "current tab" throughout the playlist chapter became "list"; C17's save format changed from `tabs[]` to a single `list`; C18 "Open List File" originally put the list in a new tab and now replaces the current list (with confirmation when the list is not empty), while dropping list files still appends or inserts per A15.
9. **Chapter titles adjusted after the cut**: "Capture and recording" became "Capture (`capture`)", "Playback history and favorites" became "Playback history (`history`)", and "Visualization and idle screen" became "Album art and idle screen (`visualizer`)"; the plugin ids are unchanged.
10. **Mini size became picture-in-picture**: H07 became macOS picture-in-picture, keeping the original shortcut `` ` `` and menu position 710; `setVideoArea` is no longer used by H07, which uses the new interface W5 and host behavior H8 instead.

## Appendix B: Common conventions

Basis for the current state: the plugin sources and host code of `main` (1a149a3), and the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html).

### B.1 Current host facts (confirmed by reading the code)

- **Rendering**: the host uses libmpv software rendering at a fixed 960×540 (`WIDTH/HEIGHT` in `backend.rs`), which Slint then scales to the video area with `image-fit: contain`. The black bars caused by the window shape are drawn by Slint, and mpv does not know about them. C-SIZE and C-GPU in Appendix A will change this.
- **Subtitles**: at startup the host sets `sub-auto=no`, `sub-font-provider=none`, `embeddedfonts=no`, `sub-ass-override=force` (see `backend.rs`, `subtitle_style.rs`).
- **External subtitles**: currently only UTF-8 SRT can be uploaded through `subtitles.load(bytes)`, with a 2 MiB limit.
- **Screenshots and export**: go through an offline ffmpeg worker process; `Snapshot.export` only supports `png` and `mp4`.

### B.2 Menus

- A new top-level menu "Settings ▸" (`windows` group, order 830–859) holds preferences, shortcuts, appearance and plugin management. The plugin workshop stays at the top level, order 820, and does not move.
- Top-level numbers of the `windows` group:

  | Menu item | order |
  | --- | --- |
  | Playlist | 800 |
  | Playback Info | 810 |
  | Playback Properties… | 811 |
  | Plugin Workshop… | 820 |
  | Settings ▸ | 830–859 |
  | About Vestra Player… | 880 |

- When implementing, [menus.md](menus.md) has to be updated to match.

### B.3 Shortcuts

- **Notation**: per [Binding syntax](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#binding-syntax), symbol keys are written as the typed character, without Shift. For example `Shift+[` is written `{`, and `Shift+'` is written `"`.
- **Ctrl and Cmd**: on macOS `Ctrl+X` matches both ⌃X and ⌘X. So do not bind `Ctrl+Q`, or it would take ⌘Q (Quit).
- **Option key risk**: on macOS, Option plus a letter or symbol usually types a different character, for example Option+[ types `“`, and Option+E and Option+N are dead keys. `bindings.rs::from_slint_key` recognizes keys by `event.text`, so `Alt+…` bindings may not trigger. Every binding with Alt has to be accepted on real hardware. If it does not trigger, the core switches to recognizing physical keys (see Appendix A); the default key is not changed to something else.

### B.4 External tools and system commands

- ffprobe uses `VESTRA_FFPROBE`, or `ffprobe` on PATH when it is not set; ffmpeg likewise uses `VESTRA_FFMPEG`.
- No call goes through a shell; all have timeouts, and processes are terminated when the plugin is disabled.
- A missing tool is only reported in the feature that uses it, and is not an error.
- Clipboard, preventing sleep, calling external tools, reading and writing settings files, showing in the file manager and opening paths/URLs, secret storage, system information and screenshot formats all use core capabilities (`plugin.clipboard`, `plugin.power`, `plugin.tools`, `plugin.storage`, `plugin.system`, `plugin.secrets`, `plugin.app.system()`, `Snapshot.export`). Plugins do not call any system command directly (community workflow §1.5), and the portability check script `npm run portability` must report zero findings.

### B.5 Playlist and files

- **Media extensions** (case-insensitive):
  - Video: `mp4 m4v mkv webm mov avi wmv asf flv f4v ts m2ts mts mpg mpeg vob ogv 3gp rm rmvb`
  - Audio: `mp3 m4a aac flac wav ogg oga opus wma ape alac aif aiff dsf dff mka wv tta ac3 dts`
  - List files: `m3u m3u8 pls`
  - Subtitles: `srt ass ssa vtt sub idx sup`
- **Natural sorting**: `Intl.Collator("zh", { numeric: true, sensitivity: "base" })`, consistent with the existing `queue.ts`.
- **Directory scanning**:
  - Skip files and directories starting with `.`, and macOS bundles (`.app`, etc.).
  - Follow symbolic links, but de-duplicate by `realpath` to prevent loops.
  - Unreadable subdirectories are skipped and counted.

### B.6 Messages and data

- On-screen messages always use `plugin.osd.show`, truncated beyond 200 characters. In the text, "OSD:" marks the original message wording.
- Rebuildable caches, such as thumbnails and covers, go under `<plugin.dataDirectory>/cache/`.
