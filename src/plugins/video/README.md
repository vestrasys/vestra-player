# Picture Adjustments

Adjusts the picture's color, effects, aspect ratio and how it is shown.

## Features

- **Color**: brightness, contrast, saturation, hue, gamma; four presets.
- **Image processing**: Gaussian blur, sharpen, edge sharpen, 3D / gradual / temporal denoise, deblock, deband, frame doubling.
- **Flip and rotation**, **deinterlace** (auto, always, never; Auto is the player's own deinterlacer, only on interlaced frames and always by field (double rate); Always outputs by frame or by field).
- **Aspect ratio and crop**: keep or ignore the aspect ratio, fill the window; common and custom ratios; crop or stretch to a ratio.
- **Framing**: zoom and pan the picture.
- **GPU render options**: HDR tone mapping, pixel shaders, motion interpolation.
- **GPU rendering switch**: GPU Rendering (Experimental) in the panel's Advanced section chooses GPU or software rendering for the next start and offers a restart (a Restart button while one is pending); greyed out with the player's reason where the GPU cannot be chosen (Windows for now), but always usable while checked.
- Color, effect and aspect settings carry over between files and restarts; flip, rotation and framing apply to the current file only.

## Entries

- **Picture…** (Alt+1, top level of the right-click menu) opens the **Picture** tab of the adjust column beside the video, and closes it when that tab shows. One scrolling column: Colour, Effects, Geometry and a closed Advanced section; the bar at the top jumps to them. Each control shows its key; a slider's double-click returns it to its default.
- Every other command (colour steps, effects, aspect, crop/stretch, framing, rotation, deinterlace, HDR, shaders, the resets) is in the command palette (Ctrl+Shift+P) with its keys, named "Category: Title".
- Brightness W/E, contrast R/T, saturation Y/U, hue I/O; Q resets the picture (the panel's Reset All).

## Limits

- HDR, pixel shaders and motion interpolation need GPU rendering; they are unavailable under software rendering.
- Colours at their defaults add no filter, and Deinterlace Auto is not a filter (the player deinterlaces only frames flagged interlaced, at double rate: 25i plays at 50 fps), so the default settings keep zero-copy decoding where the renderer allows it. While any colour, effect, flip, rotation, crop/stretch or Deinterlace Always is active, the player decodes with copy-back (hardware decoding still, but frames pass through system memory), which costs zero-copy decoding and makes exact seeks on long-GOP files slower. Deinterlace Output (by frame or by field) applies to Always only. On macOS Auto deinterlaces on the CPU: from the first interlaced frame of a file on, frames are copied from the GPU.
- Frame doubling runs on the CPU and is heavy for high-resolution video.
- The output is always SDR.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
