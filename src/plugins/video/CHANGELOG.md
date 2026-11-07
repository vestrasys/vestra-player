# Changelog

## 1.1.12

- Translations only: the category text "Reset" added in 1.1.11 is now marked `x-machine` in de, es, fr, ja and ko. Who must change: nobody. No text or behaviour change.

## 1.1.11

- Every command has a **category** for the player's command palette (Ctrl+Shift+P, SDK 1.6.4), which names a command "Category: Title": the item's submenu label (**Pixel Shaders**, **HDR Tone Mapping**, **Deinterlace** (its Output items too), **Aspect**, **Crop/Stretch**, **Framing**, **Image Processing**, **Rotation**), **Reset** for the items in the Reset menu, and **Picture** for the items directly under Video (Picture Adjustments…, the colour keys, GPU Rendering). The categories use the menu's translations; "Reset" uses the player's own word for its menu in every language.
- Commands whose control is also in a window name that window as their `home`, so the palette shows where the setting can be changed by hand: **Picture Adjustments…** for the colour keys, Image Processing, Rotation, Deinterlace, Aspect, the Crop/Stretch choices, Zoom In/Out and Reset ▸ Framing (the Zoom row's Default button); **HDR Settings…** (its Advanced page) for the tone mappings and Use Shaders; **Shader List…** for Open Shader Folder.
- No titles, ids, menu paths, keys or order changed. Who must change: nobody. Requires player 0.7.6 (`minPlayerVersion`), whose command declarations accept `category` and `home`.

## 1.1.10

- Code only: the Picture window handle (`picture.slint`) is named `pictureWindow` (was `window`). Who must change: nobody. No behaviour change.

## 1.1.9

- **Reset ▸ Picture (Q)** (was Video ▸ Reset) now returns the picture to normal: besides the colours and the Gaussian blur it already reset, it turns off every image-processing switch of this plugin (sharpen, edge sharpen, 3D/gradual/temporal denoise, deblock, deband, frame doubling, motion interpolation). Before, 3D Denoise and the other effects stayed on after Reset. Strengths are kept, as when a switch is turned off by hand, so turning one back on brings its strength back; a row's **Default** button in the window still resets that row's strength only, and the colour page's presets still replace colour and blur only. Flip, rotation, crop/stretch, aspect, framing, deinterlace and the pixel shaders / HDR options have their own items and are left alone, as are other plugins' filters. With nothing left on, the plugin's chain holds no filter, so zero-copy decoding resumes (deinterlace Auto is the player's own deinterlacer, not a filter). The reset is saved like any other change.
- The on-screen text says "Picture reset; image processing off" when Reset turned something off, and "Picture reset" as before otherwise.
- The video resets moved to the player's **Reset** menu (below Settings, player 0.6.3; user ruling 2026-11-01): **Reset ▸ Picture** (was Video ▸ Reset, Q), **Reset ▸ Framing** (was Video ▸ Framing ▸ Reset Framing, Numpad5 and Alt+Backspace) and **Reset ▸ Crop Position** (was Video ▸ Crop/Stretch ▸ Reset Position). Who must change: nobody. The command ids (`video.reset`, `video.frame.reset`, `video.reframe.reset`) and default keys are the same, so keys a user bound to them keep working. Framing ▸ Center Frame (Ctrl+Numpad5) and the "None", "Default (Source)" and "No Rotation" choices stay where they are. Requires player 0.6.3 (`minPlayerVersion`), whose menus have the Reset menu.

## 1.1.8

- New **Video ▸ GPU Rendering (Experimental)**, a check item at the end of the Video menu for the player's renderer choice (`plugin.rendering`, SDK 1.5.35). It is checked when the saved choice is the GPU, or Auto where the next start draws on the GPU (the macOS default), and greyed out with the player's reason where the GPU cannot be chosen: Windows (no GPU renderer yet), a graphics driver on the player's denylist, a GPU crash at an earlier start. A checked item always stays usable, so a user who chose the GPU can switch back to software rendering after a crash or a denylist rule. Picking it saves GPU or software rendering in the player's settings; the running player keeps its renderer and the next start uses the choice. While a restart would change the renderer, the item reads **GPU Rendering (Restart to Apply)** and the plugin asks "Restart Vestra Player now to apply?": Restart Now restarts the player (a local file continues where it is), Later keeps the choice for the next start; until then **Video ▸ Restart Player to Apply** stays in the menu (Chrome's Relaunch beside its hardware acceleration switch). Picking the switch again before a restart returns to the current renderer and removes both. When the GPU is chosen but the next start still draws in software (`VESTRA_RENDER`, a denylist rule), the on-screen text gives the player's reason instead of "on". While a choice is being saved or the restart question is open, the item is busy ("Switching…"). A refused choice or a failed save or restart is shown on screen with the player's message. The plugin stores nothing itself.
- The state is read at activation, before and after every switch and on each new source (the SDK has no menu-open event).
- New capability `rendering.manage` (highlighted in the store). Requires player 0.5.13 (`minPlayerVersion`).

## 1.1.7

- Software rendering (Windows, Linux, fallback) no longer writes `HostError: render options need GPU rendering` to the log at every start. The render options (HDR tone mapping, pixel shaders, motion interpolation) still report `unsupported` there, but that is the player's state, not a failure: their menu items stay disabled with the reason "Needs GPU rendering (currently software rendering)", as before, and other render failures (another plugin owns the options, a shader that does not compile) are logged as before.
- Under software rendering the plugin no longer sends the options again after the first refusal (on an edit). The saved options are kept unchanged and apply when the same settings run with GPU rendering (macOS).

## 1.1.6

- Deinterlace **Auto** (the default) now uses the player's own deinterlacer (`{ kind: "deinterlace", when: "auto", output: "field" }`, player 0.5.4) instead of a `bwdif` filter in the chain. The player decides per frame from the decoder's interlaced flag, so progressive files (almost all modern video) get no deinterlacer and no filter: with the default settings decoding stays zero-copy where the renderer allows it, instead of copy-back on every file. Interlaced files and the interlaced parts of mixed files are still deinterlaced, with the decoder's hardware deinterlacer where frames stay on the GPU.
- Auto now deinterlaces **by field** (double rate: 25i plays at 50 fps), as mpv and most players do; the player's deinterlacer has no frame-rate output. The Output setting (By Frame / By Field) applies to **Always** only: under Auto it is greyed in the Deinterlace ▸ Output menu and in the window, with a tip that says so. The stored Output is kept and used again when Always is chosen; settings saved by earlier versions (Auto + By Frame) load as Auto at field rate.
- Always (`bwdif` on every frame, with the Output setting) and Never (no node) are unchanged. The "many frames dropped with field output" hint now watches Always + By Field only, where switching Output is a choice.
- Requires player 0.5.4 (`minPlayerVersion`).

## 1.1.5

- The colour `eq` filter is in the chain only while brightness, contrast, saturation or gamma differs from its default; it was always there, at neutral values. Any filter in the chain makes the player decode with copy-back (GPU → system memory → GPU), so untouched colours cost zero-copy hardware decoding (`d3d11va` on the Windows GPU build became `d3d11va-copy`) and made exact seeks on long-GOP files 3–5× slower. Moving a colour away from its default adds the node while playing, and the player rebuilds its filter stage once (and switches the decoder to copy-back, which replays from the last keyframe): a short hiccup on the first move, acceptable. A drag keeps the node until the release, so dragging through 0 updates it in place; the release, a key step, Reset (Q) or the Original preset remove it once every colour is back at its default, with the same one-time hiccup. Settings restored at startup with a non-default colour still add the node.
