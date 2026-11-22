# Feature catalog: implemented and not implemented

Date: 2026-10-20. Implementation status checked against the current Vestra `main` (1469a3c).

**Usage**: every item has an ID (letter = group). The IDs are for filtering and for references from requirement and specification documents.

**Follow-up**: the detailed requirements for the unimplemented items that survived the cut review (one chapter per plugin) are in [Feature requirements](feature-requirements.md).

**Status**: ✅ implemented · ◐ partly implemented (the note states the gap) · ✗ not implemented · ⊘ won't do (Windows-only, hardware devices, live-streaming services and other things unrelated to a desktop player, plus the items removed in the cut review)

**Owner**: who is responsible when this item is built. `core` = needs a new host or SDK capability (requested bottom-up under the community workflow); a plugin name = can be done inside that existing plugin; `new plugin` = suggest building a separate plugin.

---

## A. Opening and sources

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| A01 | Open file | ✅ | playback |
| A02 | Open link (generic URL) | ◐ | Only the dedicated YouTube / Bilibili windows and the playlist's "Add Link…"; no generic "Open URL…". playback |
| A03 | Open folder (add the whole directory and play) | ✗ | playlist |
| A04 | Open remote FTP / WebDAV / HTTP server | ⊘ | new plugin. Dropped in the cut review (2026-10-20). |
| A05 | Preferred open action (what the open button does) | ⊘ | playback. Dropped in the cut review (2026-10-20). |
| A06 | Quick file browser | ⊘ | new plugin. Dropped in the cut review (2026-10-20). |
| A07 | Open subtitles | ✅ | subtitles |
| A08 | Add secondary subtitles (two subtitles shown at once) | ✗ | core (dual subtitle tracks) + subtitles |
| A09 | Reload subtitles | ◐ | There is "Refresh Subtitle Tracks", which does not re-read the file. subtitles |
| A10 | Reopen the current / last file | ◐ | "Reopen" for the current source exists; no "last file" (needs history). playback |
| A11 | Load an external audio track | ✗ | core |
| A12 | Recent history (list of opened files) | ✗ | new plugin (uses the plugin data directory) |
| A13 | When opening a file, also add similar files / all files in the same folder | ✗ | playlist |
| A14 | Favorites / albums (bookmark the current file or folder) | ⊘ | new plugin. Dropped in the cut review (2026-10-20). |
| A15 | Drag and drop files onto the window to open | ✗ | core (window drag-and-drop events) |
| A16 | Online video: YouTube | ✅ | youtube |
| A17 | Online video: Bilibili | ✅ | bilibili |
| A18 | Screen capture / camera / analog and digital TV | ⊘ | Hardware and Windows devices |
| A19 | DVD / Blu-ray devices and menus | ⊘ | Optical drive |

## B. Playback control

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| B01 | Play / pause | ✅ | playback |
| B02 | Stop (close the current source) | ✅ | playback |
| B03 | Previous / next | ✅ | playback (through the core play sequence, provided by playlist) |
| B04 | Back / forward 5 seconds | ✅ | playback |
| B05 | Back / forward 30, 60, 300 seconds | ✅ | playback |
| B06 | Configurable seek steps (time span settings) | ✗ | playback (stored in the plugin data directory) |
| B07 | **Previous frame / next frame** | ✗ | **core** (`player.frameStep`; mpv has frame-step) + playback |
| B08 | Previous keyframe / next keyframe | ✗ | core + playback |
| B09 | Jump to start | ✗ | playback (the SDK already has `restart`) |
| B10 | Jump to the midpoint / 30 seconds before the end | ⊘ | playback. Dropped in the cut review (2026-10-20). |
| B11 | Previous / next line of dialogue (seek by subtitles) | ✗ | core (read the subtitle timeline) + subtitles |
| B12 | Jump to a time manually | ✅ | playback |
| B13 | Jump to a given frame number | ⊘ | core (frame-accurate seek) + playback. Dropped in the cut review (2026-10-20). |
| B14 | Seek by keyframe (fast but imprecise) | ⊘ | core. Dropped in the cut review (2026-10-20). |
| B15 | Time / chapter / subtitle scene browser (thumbnail grid) | ⊘ | new plugin + core (thumbnail generation). Dropped in the cut review (2026-10-20). |
| B16 | Speed: normal / −0.1 / +0.1 | ✅ | playback |
| B17 | Speed: fixed steps and custom | ✅ | playback (0.5×/1.5×/2× + custom) |
| B18 | Pitch handling when the speed changes (high quality / no pitch correction) | ⊘ | core. Dropped in the cut review (2026-10-20). |
| B19 | Skip intro / outro | ⊘ | new plugin or playback (stores settings). Dropped in the cut review (2026-10-20). |
| B20 | Skip chapters | ⊘ | core (chapter information). Dropped in the cut review (2026-10-20). |
| B21 | Chapter / bookmark menu and jump | ✗ | core (read chapters) + new plugin (bookmarks) |
| B22 | Show chapter / bookmark marks on the seek bar | ✗ | core + playback |
| B23 | **A-B loop** (set start / end, fine-tune, toggle) | ✗ | core (loop range) + new plugin or playback |
| B24 | Loop the current subtitle line | ⊘ | core + subtitles. Dropped in the cut review (2026-10-20). |
| B25 | Remember playback position (resume where you left off) | ✗ | new plugin (plugin data directory) or playback |
| B26 | Fast advance to the next item (formerly "gapless playback"; preloading was dropped) | ✗ | `playlist` |
| B27 | Action after playback ends (quit, shut down, sleep) | ⊘ | new plugin; shutting down needs system permission, suggest doing only "quit". Dropped in the cut review (2026-10-20). |
| B28 | Quit on a timer / after inactivity | ⊘ | new plugin. Dropped in the cut review (2026-10-20). |
| B29 | Prevent the screen saver / sleep while playing | ✗ | core |
| B30 | Hide the mouse pointer while playing | ◐ | Hidden only in fullscreen. core |

## C. Playlist

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| C01 | Show / hide the playlist | ✅ | playlist |
| C02 | Attach / detach as a floating window | ✅ | playlist + core |
| C03 | Add file / add link | ✅ | playlist |
| C04 | Add folder | ✗ | playlist |
| C05 | Play with double-click / Enter / P | ✅ | playlist |
| C06 | Remove selected / remove all | ✅ | playlist |
| C07 | Remove unselected / remove invalid items | ✗ | playlist |
| C08 | Delete the file from disk | ⊘ | playlist (needs a confirmation dialog). Dropped in the cut review (2026-10-20). |
| C09 | Move up / down / to top / to bottom, drag to reorder | ✅ | playlist |
| C10 | Sort: by name / path / reverse | ✅ | playlist |
| C11 | Sort: by extension / size / duration / date / random / folder | ✗ | playlist |
| C12 | Select all / multi-select | ✅ | playlist |
| C13 | Invert selection | ⊘ | playlist. Dropped in the cut review (2026-10-20). |
| C14 | Loop: off / all / single | ✅ | playlist |
| C15 | Shuffle | ✅ | playlist |
| C16 | Show item durations and the total duration | ✅ | playlist (ffprobe) |
| C17 | Remember the playlist (restore after restart) | ✗ | playlist (plugin data directory) |
| C18 | Open / save playlist files (M3U etc.) | ✗ | playlist |
| C19 | Multiple albums (several list tabs) | ⊘ | playlist. Dropped in the cut review (2026-10-20). |
| C20 | Play queue (play next) | ⊘ | playlist. Dropped in the cut review (2026-10-20). |
| C21 | Search the list | ✗ | playlist |
| C22 | Edit title / rename file | ⊘ | playlist. Dropped in the cut review (2026-10-20). |
| C23 | Show the file's location (Finder) | ✗ | playlist |
| C24 | Copy / paste list items | ⊘ | playlist. Dropped in the cut review (2026-10-20). |
| C25 | Thumbnail view (a thumbnail per item) | ⊘ | core (thumbnails) + playlist. Dropped in the cut review (2026-10-20). |
| C26 | File information in the list (codec, size, etc.) | ⊘ | playlist (ffprobe). Dropped in the cut review (2026-10-20). |
| C27 | Do not add the same file twice | ✅ | playlist (already de-duplicated) |

## D. Subtitles

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| D01 | Choose a subtitle track | ✅ | subtitles (listed dynamically in the menu) |
| D02 | Cycle through subtitle tracks | ✗ | subtitles |
| D03 | Show / hide subtitles | ✅ | subtitles |
| D04 | Subtitle sync: earlier / later / reset | ✅ | subtitles |
| D05 | Enter a sync value manually | ◐ | The subtitle tuning window has slower / faster buttons but no number field. subtitles |
| D06 | Font size +/− | ◐ | In the window, not bound to a shortcut. subtitles |
| D07 | Move the subtitle position up / down | ◐ | In the window, not bound to a shortcut. subtitles |
| D08 | Move subtitles left / right, left / right alignment | ⊘ | core (style fields). Dropped in the cut review (2026-10-20). |
| D09 | Bold | ✗ | core (style fields) |
| D10 | Font, color, outline, bottom margin | ✅ | subtitles (subtitle settings window) |
| D11 | Shadow, background box (opaque box) | ✗ | core (style fields) |
| D12 | Subtitle browser (line-by-line list, click to jump, edit) | ⊘ | core (read subtitle content) + subtitles. Dropped in the cut review (2026-10-20). |
| D13 | Copy the current subtitle to the clipboard | ⊘ | core + subtitles. Dropped in the cut review (2026-10-20). |
| D14 | Save subtitles / save a copy with the sync applied | ⊘ | subtitles (the plugin reads and writes SRT itself). Dropped in the cut review (2026-10-20). |
| D15 | Subtitle frame-rate conversion | ⊘ | subtitles. Dropped in the cut review (2026-10-20). |
| D16 | Online subtitle search and download | ✗ | new plugin (network service; a provider must be chosen) |
| D17 | ASS/SSA styled subtitles | ◐ | Embedded ASS is rendered by mpv; external files support only SRT. core |
| D18 | External subtitle formats: ASS, VTT, etc. | ✗ | core |
| D19 | Subtitles outside the picture (on the black bars) / forced to the bottom | ⊘ | core. Dropped in the cut review (2026-10-20). |
| D20 | 3D subtitles | ⊘ | Related to 3D output |
| D21 | Subtitle translation | ⊘ | new plugin (a translation service must be chosen). Dropped in the cut review (2026-10-20). |

## E. Video picture

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| E01 | Brightness, contrast, saturation ±1% and reset | ✅ | video |
| E02 | Hue ±1% | ✗ | core (add hue to the eq node) + video; Gamma already exists |
| E03 | Gaussian blur (soften) | ✅ | video |
| E04 | Sharpen | ✗ | core (processing node) + video |
| E05 | Denoise (3D / temporal / progressive) | ✗ | core + video |
| E06 | Deblock / deband / edge sharpening | ✗ | core + video |
| E07 | Levels control, auto levels | ⊘ | core + video. Dropped in the cut review (2026-10-20). |
| E08 | Flip vertical / flip horizontal | ✗ | core + video |
| E09 | Rotate 90 / 180 / 270 | ✗ | core + video |
| E10 | Deinterlace (auto / on / off) | ✗ | core |
| E11 | Aspect ratio: original / 4:3 / 16:9 / 1.85 / 2.35 / custom | ✗ | core + new plugin or video |
| E12 | Stretch / crop to a given ratio | ✗ | core |
| E13 | Picture zoom and pan (frame position + / −, offset) | ✗ | core |
| E14 | Bottom margin (black bar left for subtitles) | ⊘ | core. Dropped in the cut review (2026-10-20). |
| E15 | Choose a video stream | ⊘ | core. Dropped in the cut review (2026-10-20). |
| E16 | Automatic HDR correction / gamut correction | ✗ | core (mpv tone-mapping) |
| E17 | Pixel shaders (custom shaders) | ✗ | core (mpv glsl-shaders) |
| E18 | Frame doubling / motion compensation | ✗ | core (mpv interpolation) |
| E19 | 3D video output | ⊘ | |
| E20 | Video renderer choice / AviSynth / VapourSynth | ⊘ | Windows renderer stack |
| E21 | Picture presets (original / vivid / cinema / soft) | ✅ | video |

## F. Screenshots and recording

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| F01 | Save the current frame (save directly) | ✅ | capture |
| F02 | Save the current frame as | ✅ | capture |
| F03 | Copy the current frame to the clipboard | ✗ | core (clipboard image) + capture |
| F04 | Capture the picture as displayed (with effects, subtitles) | ◐ | Burst capture and export can include effects; a single shot captures only the source frame. capture |
| F05 | Burst screenshots | ✅ | capture |
| F06 | Thumbnail sheet (several frames in one image) | ⊘ | capture + core. Dropped in the cut review (2026-10-20). |
| F07 | Screenshot format JPEG / BMP, quality | ◐ | PNG only. core + capture |
| F08 | File name with system time / playback time | ⊘ | Fixed as "title_playbacktime". capture. Dropped in the cut review (2026-10-20). |
| F09 | Screenshot folder setting and open | ◐ | Can be set and is remembered; no "Open Folder". capture |
| F10 | Delete the last screenshot | ⊘ | capture. Dropped in the cut review (2026-10-20). |
| F11 | Export a clip (start and end time) | ✅ | a store plugin since 2026-11-09 (was the built-in capture plugin); offline export, not live recording |
| F12 | Live video / audio recording | ⊘ | core. Dropped in the cut review (2026-10-20). |

## G. Audio

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| G01 | Volume +/−, mouse wheel volume | ✅ | playback |
| G02 | Mute | ✅ | playback |
| G03 | Choose an audio track | ✗ | core (choose audio track) + playback or audio |
| G04 | Cycle through audio tracks | ✗ | Same as above |
| G05 | Channels: stereo / left / right | ✗ | core + audio |
| G06 | Swap left and right channels | ⊘ | core + audio. Dropped in the cut review (2026-10-20). |
| G07 | Equalizer toggle and adjustment | ✅ | 6 bands. Extending to 10 bands was dropped in the cut review (2026-10-20). audio |
| G08 | Equalizer presets | ✅ | audio (4 presets) |
| G09 | Audio sync: earlier / later / reset | ◐ | The window has a delay slider, not bound to a shortcut. audio |
| G10 | Normalization (loudness leveling) | ✅ | audio |
| G11 | Crystalizer (clarity) | ◐ | There is a "clarity enhancement" (high-frequency boost), not bound to a shortcut. audio |
| G12 | Noise reduction | ⊘ | core + audio. Dropped in the cut review (2026-10-20). |
| G13 | Reverb | ⊘ | core + audio. Dropped in the cut review (2026-10-20). |
| G14 | Vocal removal / vocal boost | ⊘ | core + audio. Dropped in the cut review (2026-10-20). |
| G15 | Volume boost (above 100%) | ✅ | playback (up to 200%) |
| G16 | Audio output device choice | ✗ | core |
| G17 | Output channels / sample rate / passthrough (SPDIF) | ⊘ | Hardware-related, not needed for now |
| G18 | System master volume / wave volume | ⊘ | Windows system volume |

## H. Window and interface

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| H01 | Fullscreen | ✅ | core |
| H02 | Stretched fullscreen (aspect ratio not kept) | ⊘ | core. Dropped in the cut review (2026-10-20). |
| H03 | Window size 0.5 / 1 / 1.5 / 2 × the video size | ✗ | core + new plugin |
| H04 | Fit to the video's aspect ratio (window without black bars) | ✗ | core |
| H05 | Stay on top: always / while playing / in fullscreen | ✗ | core |
| H06 | Move the window to a screen corner / center | ⊘ | core. Dropped in the cut review (2026-10-20). |
| H07 | Mini size | ✗ | core |
| H08 | Bottom control bar | ✅ | playback |
| H09 | Show / hide the control bar and the playlist | ✅ | playlist, core |
| H10 | Skin / color scheme switching | ✗ | core (switchable themes); currently the brand colors (taken from the logo) |
| H11 | Window opacity | ⊘ | core. Dropped in the cut review (2026-10-20). |
| H12 | Show the logo when idle | ✗ | core |
| H13 | Control panel (picture / audio / subtitles / playback adjusted in one place) | ⊘ | Decided to use separate tool windows (remediation document D2) |
| H14 | Interface language switching | ✗ | core + all plugins (currently Chinese only) |

## I. Information and feedback

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| I01 | On-screen messages (action feedback) | ✅ | core + each plugin |
| I02 | Full playback information overlay | ✅ | info |
| I03 | Brief information (file name, time, codec) | ✗ | info |
| I04 | Miscellaneous information (time, percentage, index) | ⊘ | info. Dropped in the cut review (2026-10-20). |
| I05 | Playback / system information dialog (codecs, filters, file information) | ✗ | info + core (more statistics) |
| I06 | Show the time when hovering over the seek bar | ✗ | playback |
| I07 | Show a thumbnail preview when hovering over the seek bar | ✗ | core (thumbnails) + playback |
| I08 | Copy the file name / path to the clipboard | ✗ | core (clipboard text) + playback |
| I09 | About | ✗ | core |

## J. Settings and plugins

| ID | Feature | Status | Note / owner |
| --- | --- | --- | --- |
| J01 | **Shortcut settings** (rebind any command to keyboard or mouse) | ✗ | **new plugin + core**, see the next section |
| J02 | Mouse action settings (what click, double-click, wheel and side buttons are bound to) | ✗ | Same as above (the binding table already supports mouse input) |
| J03 | Ignore default shortcuts | ✗ | Same as above |
| J04 | Main preferences window (settings in one place) | ✗ | new plugin + core (how the plugins' settings pages are combined needs design) |
| J05 | Settings import / export | ⊘ | core. Dropped in the cut review (2026-10-20). |
| J06 | Decoder / filter management | ⊘ | DirectShow stack |
| J07 | Plugin management (list, disable, enable, uninstall) | ✅ | store: the plugin center's Installed tab (the separate Plugin Manager `vestra.plugins` was removed on 2026-10-26) |
| J08 | Plugin Workshop (AI-generated plugins) | ✅ | workshop |
| J09 | Emulator (NES) | ✅ | VestraNES, plugin repository (not built in since 2026-10-27) |
| J10 | Check for updates | ✅ | info 0.1.11: "Check for Updates…" after About, the Updates row in About Vestra Player (downloads this system's installer to the Downloads folder, checks size and SHA-256, Run Installer opens it) and Options ▸ General ▸ Updates (automatic check, at most once a day, one notice per version). Dropped in the cut review (2026-10-20); restored by the user ruling of 2026-11-01: built-in plugins ship with the player, so the player itself needs an update check. |
| J11 | Visualization (animation and album art for audio-only playback) | ✗ | new plugin + core (drawing in the video area) |
| J12 | Live-streaming, chat and broadcast tools | ⊘ | Third-party live-streaming services |

---

## Statistics

| Status | Count |
| --- | --- |
| ✅ Implemented | 49 |
| ◐ Partly implemented | 13 |
| ✗ Not implemented | 66 |
| ⊘ Won't do | 55 |
| Total | 183 |

Of the unimplemented items, about 60% first need a capability from the core (mostly things mpv already has and only need to be exposed: frame stepping, rotate and flip, sharpen and denoise, aspect ratio, audio track choice, A-B loop, chapters, stay on top, etc.). The rest can be done in existing plugins or new plugins.

---

## About the shortcut settings plugin (J01–J03)

**It can be done, and it needs no communication between plugins.** The reason is that all shortcuts are already kept in the core's binding table, not in the individual plugins:

- Each plugin only **declares** "which keys my commands are bound to by default" (`bindings`); the actual decision "which key triggers which command" is made by the core.
- The core's binding table already has a **user override** layer (bindings the user changed take precedence over plugin defaults, and can be unbound), and it is already saved across launches in `ui-state.json`. This layer was done in the previous round; only the read/write API for plugins is still missing.
- So the shortcut settings plugin only needs to talk to the core: "list all commands and their current bindings", "rebind a command to a key", "restore defaults". It does not need to know that playback or playlist exist, and it never calls them.

Having the features spread over many plugins does **not** make this harder; it is exactly the benefit of this design: any command registered by any plugin shows up in the shortcut settings automatically, including plugins installed later and plugins generated by the Plugin Workshop.

The API the core needs to add (one API request, bottom-up):

```ts
plugin.bindings.list(): BindingInfo[]     // per command: plugin, command id, menu path or title, default binding, current binding, conflicts
plugin.bindings.set(command, inputs[])    // writes the user override layer; [] means unbind
plugin.bindings.reset(command?)           // restore defaults (one command or all)
plugin.bindings.onChange(listener)
```

This needs a new core API (`plugin.bindings`), because it can change other plugins' shortcuts.

**Basic operations need core "actions" that do not depend on any plugin.** Operations such as previous / next frame, play / pause and previous / next should still be bindable by the shortcut plugin even if the user uninstalls the playback plugin. Current state:

| Operation | Who provides the command now | After playback is uninstalled |
| --- | --- | --- |
| Fullscreen, leave fullscreen only, quit | core | Still available |
| Play / pause, seek, speed, volume, mute, previous / next | playback | No command to bind |
| Previous / next frame | none (B07 not implemented) | — |

Suggested approach: like "fullscreen" and "quit" today, the core registers a set of **basic playback actions** itself (play / pause, stop, previous / next frame, ±5 seconds, previous / next, volume ±, mute, speed ±). The playback plugin is then only responsible for the bottom bar UI and its own extra commands, and no longer registers these basic actions again. This way:

- the basic actions always exist, and the shortcut plugin can always bind them;
- playback becomes a pure UI plugin that another bottom-bar plugin can replace;
- no communication between plugins is involved.

This is a change of core responsibilities (moving some commands from playback into the core). If it is done, it should be one round of core changes together with the shortcut API, following the top-down process.

---

## Principles for choosing plugin experiments

When choosing plugin experiment topics and dividing responsibilities between plugins and the core:

1. **Feature groups are not the menu structure.** For example, the entry points for loading subtitles are split between "Open" and "Subtitles", and audio recording, video recording and screenshots can also be regrouped by task; one plugin can contribute several menu entries.
2. **Controlling parameters does not mean running the algorithm in TS.** Panels for color, audio DSP, GPU enhancement and the like handle the settings; the computation is done by a mature playback backend or a native provider. The exact bridge is decided by experiment.
3. **Backend and platform settings do not all have to be plugins.** The lifecycle of low-level resources such as decoding, devices and renderers belongs to the host or the backend; plugins only provide policy and entry points.
4. **Consider the default bundle and optional extensions separately.** The NES emulator and the like are optional extensions and are not part of the default feature set (2026-10-27: the NES emulator left the player and is the store plugin VestraNES).
5. **New candidates** (not numbered in the tables above): white balance / color temperature adjustment; GPU picture enhancement such as super-resolution, as extra experiment candidates.
