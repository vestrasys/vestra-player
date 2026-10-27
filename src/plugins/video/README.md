# Picture Adjustments

Adjusts the picture's color, effects, aspect ratio and how it is shown.

## Features

- **Color**: brightness, contrast, saturation, hue, gamma; four presets.
- **Image processing**: Gaussian blur, sharpen, edge sharpen, 3D / gradual / temporal denoise, deblock, deband, frame doubling.
- **Flip and rotation**, **deinterlace** (auto, always, never; output by frame or by field).
- **Aspect ratio and crop**: keep or ignore the aspect ratio, fill the window; common and custom ratios; crop or stretch to a ratio.
- **Framing**: zoom and pan the picture.
- **GPU render options**: HDR tone mapping, pixel shaders, motion interpolation.
- Color, effect and aspect settings carry over between files and restarts; flip, rotation and framing apply to the current file only.

## Entries

- Right-click menu Video: Pixel Shaders, Deinterlace (Ctrl+I), Aspect (Shift+F6), Crop/Stretch, Framing, Image Processing, Rotation (Alt+K), Picture Adjustments…, Reset (Q).
- Brightness W/E, contrast R/T, saturation Y/U, hue I/O.
- The Picture Adjustments… window has four tabs: Color, Effects, Picture, Advanced.

## Limits

- HDR, pixel shaders and motion interpolation need GPU rendering; they are unavailable under software rendering.
- Colours at their defaults add no filter. While any colour, effect, flip, rotation, crop/stretch or deinterlace (Auto or Always) is active, the player decodes with copy-back (hardware decoding still, but frames pass through system memory), which costs zero-copy decoding and makes exact seeks on long-GOP files slower.
- Frame doubling runs on the CPU and is heavy for high-resolution video.
- The output is always SDR.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
