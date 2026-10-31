# Changelog

## 0.1.8

- About Vestra Player shows how the video is rendered, below the component versions: **Rendering: GPU**, or **Rendering: Software — <reason>** with the player's own reason (for example that this player has no GPU renderer on Windows yet, or that the graphics driver is on the player's denylist, with where to choose another GPU in Windows' graphics settings). The reason is the player's English text, shown as it is; it wraps and the window grows to fit. Under GPU rendering a reason is shown only when it says more than "nothing rules GPU rendering out". Copy Info includes the same line.
- Read from `plugin.app.system()` (`rendering`, `renderingReason`) each time the window opens; no capability. Players before 0.5.12 report no renderer and the row is left out (not guessed); players before 0.5.13 report no reason and the row shows the mode alone. A failed read leaves the row out.
- Built with SDK 1.5.35 (the `renderingReason` type); `minPlayerVersion` stays 0.4.3.
