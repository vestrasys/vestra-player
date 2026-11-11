# Picture Adjustments

Adjusts the picture's color, effects, aspect ratio and how it is shown.

## Features

- **Color**: brightness, contrast, saturation, hue, gamma; four presets.
- **Image processing**: Gaussian blur, sharpen, edge sharpen, 3D / gradual / temporal denoise, deblock, deband, frame doubling.
- **Flip and rotation**, **deinterlace** (auto, always, never; Auto is the player's own deinterlacer, only on interlaced frames and always by field (double rate); Always outputs by frame or by field).
- **Aspect ratio and crop**: keep or ignore the aspect ratio, fill the window; common and custom ratios; crop or stretch to a ratio.
- **Framing**: zoom and pan the picture.
- **GPU render options**: HDR tone mapping, pixel shaders, motion interpolation.
- **GPU rendering switch**: GPU Rendering at the top of the panel (Rendering) chooses GPU or software rendering for the next start and offers a restart (a Restart button while one is pending); a status line says what draws now (on the GPU, restart to apply, software rendering and why); greyed out with the player's reason where the GPU cannot be chosen, but always usable while checked.
- **AI Upscaling** (NVIDIA RTX Video Super Resolution, `plugin.video.upscaling`): a switch under GPU Rendering and the palette command Picture: AI Upscaling; saved and asked for again at each start. It needs Windows, an NVIDIA RTX GPU, GPU rendering and Super Resolution turned on in the NVIDIA app; elsewhere it is greyed out with the reason. The status line says what it does (Upscaling 1080p → 2160p on the GPU) or why it waits (the picture effects keep the frames in memory, which the upscaler cannot use).
- Color, effect and aspect settings carry over between files and restarts; flip, rotation and framing apply to the current file only.

## Entries

- **Picture…** (Alt+1; in the command palette, and the right-click menu's Adjustments and Effects opens the column at its last tab) opens the **Picture** tab of the adjust column beside the video, and closes it when that tab shows. One scrolling column: Rendering, Colour (with the presets in its heading), Effects (Frames' Doubling and Interpolate among the chips), Geometry and Advanced, a disclosure row closed by default that sums up its values. HDR Settings… opens Advanced and scrolls to it. Each control shows its key; a slider's double-click returns it to its default.
- Every other command (colour steps, effects, aspect, crop/stretch, framing, rotation, deinterlace, HDR, shaders, the resets) is in the command palette (Ctrl+Shift+P) with its keys, named "Category: Title".
- Brightness W/E, contrast R/T, saturation Y/U, hue I/O; Q resets the picture (the panel's Reset All).
- **Effects** in the panel are on/off chips; a strength slider shows for each effect that is on. The Denoise chip switches the three denoisers together (on: the ones that were on last, or 3D); the Denoise card has each one's own switch and strength.
- **Geometry** in the panel is an editor: a still picture inside a screen frame, placed by the real settings. Drag it to pan, drag a corner or scroll to zoom, double-click to centre; crop mode (the crop button) sets a crop ratio and drags the crop box. Every change applies to the video at once.

## Third-party content

- `assets/still.jpg`, the geometry editor's picture, is a frame of Big Buck Bunny, © 2008 Blender Foundation | www.bigbuckbunny.org, CC BY 3.0; see `licenses/still/NOTICE.md`.

## Limits

- HDR, pixel shaders and motion interpolation need GPU rendering; they are unavailable under software rendering.
- Colours at their defaults add no filter, and Deinterlace Auto is not a filter (the player deinterlaces only frames flagged interlaced, at double rate: 25i plays at 50 fps), so the default settings keep zero-copy decoding where the renderer allows it. While any colour, effect, flip, rotation, crop/stretch or Deinterlace Always is active, the player decodes with copy-back (hardware decoding still, but frames pass through system memory), which costs zero-copy decoding and makes exact seeks on long-GOP files slower. Deinterlace Output (by frame or by field) applies to Always only. On macOS Auto deinterlaces on the CPU: from the first interlaced frame of a file on, frames are copied from the GPU.
- Frame doubling runs on the CPU and is heavy for high-resolution video.
- The output is always SDR.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
