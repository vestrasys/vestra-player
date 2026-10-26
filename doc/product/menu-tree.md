# Menu tree (generated)

Generated from main on 2026-10-24: 18 built-in plugins (17 since the Plugin Manager `vestra.plugins` and its "Settings ▸ Plugin Manager…" item were removed on 2026-10-26; this tree has been edited to match) + the third-party plugins zysilm.youtube and zysilm.bilibili + host commands, ordered by the host's sorting rules (order; built-in top-level menus have fixed positions). `‹…›` is the plugin that registers the item, `‹core›`/`‹host›` is the player itself; `[…]` is the default shortcut; `────` is a group separator; the 32 commands that have only a shortcut and no menu item are not in the tree.

```text
Context menu
├─ Open File…  [Ctrl+O / F3]   ‹playback›
├─ Open ▸
│  ├─ Open URL…  [Ctrl+U]   ‹playback›
│  ├─ Open Folder…  [F2]   ‹playlist›
│  ├─ YouTube Video…   ‹youtube›
│  ├─ Bilibili Video…   ‹bilibili›
│  ├─ Open Subtitle…  [Alt+O]   ‹subtitles›
│  ├─ Load External Audio…   ‹audio›
│  ├─ ────────
│  ├─ Recently Played ▸
│  │  ├─ (No Entries)   ‹history›
│  │  ├─ ────────
│  │  ├─ Remove Invalid Entries   ‹history›
│  │  ├─ Clear Entries   ‹history›
│  │  └─ Record Playback History   ‹history›
│  ├─ ────────
│  └─ Add Secondary Subtitle…   ‹subtitles›
├─ Close  [F4]   ‹core›
├─ Reopen Current/Last File  [Ctrl+Y]   ‹playback›
├─ Copy to Clipboard ▸
│  ├─ File Name   ‹playback›
│  ├─ Folder Path   ‹playback›
│  └─ Full Path   ‹playback›
├─ ────────
├─ Playback ▸
│  ├─ Play/Pause  [Space]   ‹core›
│  ├─ Previous  [PageUp / Mouse:X1]   ‹core›
│  ├─ Next  [PageDown / Mouse:X2]   ‹core›
│  ├─ ────────
│  ├─ Seek ▸
│  │  ├─ Previous Frame  [D]   ‹core›
│  │  ├─ Next Frame  [F]   ‹core›
│  │  ├─ Previous Keyframe  [Ctrl+Shift+ArrowLeft]   ‹playback›
│  │  ├─ Next Keyframe  [Ctrl+Shift+ArrowRight]   ‹playback›
│  │  ├─ ────────
│  │  ├─ Back 5 s  [ArrowLeft]   ‹core›
│  │  ├─ Forward 5 s  [ArrowRight]   ‹core›
│  │  ├─ Back 30 s  [Ctrl+ArrowLeft]   ‹playback›
│  │  ├─ Forward 30 s  [Ctrl+ArrowRight]   ‹playback›
│  │  ├─ Back 60 s  [Shift+ArrowLeft]   ‹playback›
│  │  ├─ Forward 60 s  [Shift+ArrowRight]   ‹playback›
│  │  ├─ Back 300 s  [Ctrl+Alt+ArrowLeft]   ‹playback›
│  │  ├─ Forward 300 s  [Ctrl+Alt+ArrowRight]   ‹playback›
│  │  ├─ ────────
│  │  ├─ Previous Dialogue  [Home]   ‹subtitles›
│  │  ├─ Next Dialogue  [End]   ‹subtitles›
│  │  ├─ Current Subtitle Start  [Ctrl+Home]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ Jump to Start  [Backspace]   ‹playback›
│  │  ├─ ────────
│  │  └─ Seek Steps…   ‹playback›
│  ├─ ────────
│  ├─ Jump to Time…  [G]   ‹playback›
│  ├─ Speed ▸
│  │  ├─ (radio) Normal  [Z]   ‹core›
│  │  ├─ Slower 0.1  [X]   ‹core›
│  │  ├─ Faster 0.1  [C]   ‹core›
│  │  ├─ (radio) 0.5×   ‹playback›
│  │  ├─ (radio) 1.5×   ‹playback›
│  │  ├─ (radio) 2×   ‹playback›
│  │  └─ Custom…   ‹playback›
│  ├─ Playlist ▸
│  │  ├─ Add Files…   ‹playlist›
│  │  ├─ Add Link…   ‹playlist›
│  │  ├─ Add Folder…   ‹playlist›
│  │  ├─ Repeat ▸
│  │  │  ├─ (radio) Off   ‹playlist›
│  │  │  ├─ (radio) All   ‹playlist›
│  │  │  └─ (radio) One   ‹playlist›
│  │  ├─ Shuffle   ‹playlist›
│  │  ├─ ────────
│  │  ├─ Open List File…   ‹playlist›
│  │  ├─ Save List…   ‹playlist›
│  │  ├─ ────────
│  │  ├─ When Opening a File ▸
│  │  │  ├─ (radio) Open Only That File   ‹playlist›
│  │  │  ├─ (radio) Add Similar Files   ‹playlist›
│  │  │  └─ (radio) Add All Files in the Folder   ‹playlist›
│  │  ├─ ────────
│  │  ├─ Fast Advance to Next   ‹playlist›
│  │  ├─ Remember List   ‹playlist›
│  │  └─ Confirm Before Removing All   ‹playlist›
│  ├─ ────────
│  ├─ Disable Screen Saver While Playing   ‹playback›
│  ├─ Hide Pointer While Playing   ‹window›
│  ├─ Show Time on Seek Bar Hover   ‹playback›
│  ├─ Show Thumbnail on Seek Bar Hover   ‹playback›
│  ├─ ────────
│  ├─ Resume Playback ▸
│  │  ├─ (radio) Don't Resume   ‹history›
│  │  ├─ (radio) Resume Automatically   ‹history›
│  │  ├─ (radio) Ask Every Time   ‹history›
│  │  ├─ ────────
│  │  ├─ Also Remember Audio Files   ‹history›
│  │  └─ Clear All Saved Positions   ‹history›
│  ├─ ────────
│  ├─ Chapters and Bookmarks ▸
│  │  ├─ Chapter and Bookmark List…  [H]   ‹marks›
│  │  ├─ ────────
│  │  ├─ Add Bookmark  [P]   ‹marks›
│  │  ├─ Previous Bookmark or Chapter  [Shift+PageUp]   ‹marks›
│  │  ├─ Next Bookmark or Chapter  [Shift+PageDown]   ‹marks›
│  │  ├─ Previous Chapter  [Ctrl+PageUp]   ‹marks›
│  │  ├─ Next Chapter  [Ctrl+PageDown]   ‹marks›
│  │  ├─ ────────
│  │  └─ Show Marks on Seek Bar   ‹marks›
│  ├─ ────────
│  └─ A-B Loop ▸
│     ├─ Set Start Point  [[]   ‹marks›
│     ├─ Clear Start Point  [{]   ‹marks›
│     ├─ Start Point Back 0.1 s  [Ctrl+[]   ‹marks›
│     ├─ Start Point Forward 0.1 s  [Alt+[]   ‹marks›
│     ├─ ────────
│     ├─ Set End Point  []]   ‹marks›
│     ├─ Clear End Point  [}]   ‹marks›
│     ├─ End Point Back 0.1 s  [Ctrl+]]   ‹marks›
│     ├─ End Point Forward 0.1 s  [Alt+]]   ‹marks›
│     ├─ ────────
│     ├─ Move Range Back 0.1 s  [Ctrl+Alt+[]   ‹marks›
│     ├─ Move Range Forward 0.1 s  [Ctrl+Alt+]]   ‹marks›
│     ├─ ────────
│     ├─ Loop Current Chapter or Bookmark Range  [Alt+\]   ‹marks›
│     ├─ A-B Loop On/Off  [\]   ‹marks›
│     ├─ ────────
│     └─ Loop Count ▸
│        ├─ (radio) Endless   ‹marks›
│        ├─ (radio) Once   ‹marks›
│        ├─ (radio) 2 Times   ‹marks›
│        ├─ (radio) 3 Times   ‹marks›
│        ├─ (radio) 5 Times   ‹marks›
│        └─ (radio) 10 Times   ‹marks›
├─ Subtitles ▸
│  ├─ Show Subtitles  [Alt+H]   ‹subtitles›
│  ├─ ────────
│  ├─ Subtitle Track ▸
│  │  ├─ (radio) Off   ‹subtitles›
│  │  ├─ ────────
│  │  └─ Cycle  [Alt+L]   ‹subtitles›
│  ├─ ────────
│  ├─ Secondary Subtitle ▸
│  │  ├─ (radio) Off   ‹subtitles›
│  │  ├─ ────────
│  │  └─ Position ▸
│  │     ├─ (radio) Top   ‹subtitles›
│  │     └─ (radio) Above Main Subtitle   ‹subtitles›
│  ├─ ────────
│  ├─ Subtitle Style ▸
│  │  ├─ Bold  [Alt+B]   ‹subtitles›
│  │  ├─ Size +  [Alt+PageUp]   ‹subtitles›
│  │  ├─ Size −  [Alt+PageDown]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ Move Up  [Alt+ArrowUp]   ‹subtitles›
│  │  ├─ Move Down  [Alt+ArrowDown]   ‹subtitles›
│  │  ├─ Reset Position  [Alt+Home]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ Shadow   ‹subtitles›
│  │  └─ Opaque Box   ‹subtitles›
│  ├─ ────────
│  ├─ Original ASS/SSA Styles   ‹subtitles›
│  ├─ ────────
│  ├─ Sync ▸
│  │  ├─ Earlier 0.5 s  [.]   ‹subtitles›
│  │  ├─ Later 0.5 s  [,]   ‹subtitles›
│  │  ├─ Reset  [/]   ‹subtitles›
│  │  ├─ ────────
│  │  └─ Adjust Sync…   ‹subtitles›
│  ├─ ────────
│  ├─ Online Subtitles ▸
│  │  ├─ Search and Download Subtitles…   ‹subtitle-online›
│  │  └─ Online Subtitle Settings…   ‹subtitle-online›
│  ├─ ────────
│  ├─ Reload Subtitles  [Ctrl+Alt+Y]   ‹subtitles›
│  ├─ Refresh Subtitle Tracks   ‹subtitles›
│  ├─ Subtitle Tuning…   ‹subtitles›
│  └─ Subtitle Settings…   ‹subtitles›
├─ ────────
├─ Video ▸
│  ├─ Pixel Shaders ▸
│  │  ├─ Use Shaders  [Shift+P]   ‹video›
│  │  ├─ Shader List…   ‹video›
│  │  ├─ Open Shader Folder   ‹video›
│  │  ├─ Reload Shaders   ‹video›
│  │  ├─ ────────
│  │  ├─ HDR Tone Mapping ▸
│  │  │  ├─ (radio) Auto   ‹video›
│  │  │  ├─ (radio) Off   ‹video›
│  │  │  ├─ (radio) BT.2390   ‹video›
│  │  │  ├─ (radio) Hable   ‹video›
│  │  │  ├─ (radio) Mobius   ‹video›
│  │  │  ├─ (radio) Reinhard   ‹video›
│  │  │  ├─ (radio) Clip   ‹video›
│  │  │  ├─ ────────
│  │  │  └─ Dynamic Peak Detection   ‹video›
│  │  └─ HDR Settings…   ‹video›
│  ├─ Deinterlace ▸
│  │  ├─ Cycle  [Ctrl+I]   ‹video›
│  │  ├─ (radio) Auto   ‹video›
│  │  ├─ (radio) Always   ‹video›
│  │  ├─ (radio) Never   ‹video›
│  │  ├─ ────────
│  │  └─ Output ▸
│  │     ├─ (radio) By Frame   ‹video›
│  │     └─ (radio) By Field (Double Rate)   ‹video›
│  ├─ ────────
│  ├─ Aspect ▸
│  │  ├─ (radio) Keep Aspect Ratio   ‹video›
│  │  ├─ (radio) Ignore Aspect Ratio   ‹video›
│  │  ├─ (radio) Fill Window (Crop Excess)   ‹video›
│  │  ├─ ────────
│  │  ├─ Cycle Aspect Ratio  [Shift+F6]   ‹video›
│  │  ├─ (radio) Default (Source)   ‹video›
│  │  ├─ (radio) 4:3   ‹video›
│  │  ├─ (radio) 16:9   ‹video›
│  │  ├─ (radio) 16:10   ‹video›
│  │  ├─ (radio) 1.85:1   ‹video›
│  │  ├─ (radio) 2.35:1   ‹video›
│  │  ├─ (radio) Custom (x:y)   ‹video›
│  │  └─ Custom Aspect Ratio…   ‹video›
│  ├─ Crop/Stretch ▸
│  │  ├─ (radio) Never   ‹video›
│  │  ├─ (radio) Stretch to 4:3   ‹video›
│  │  ├─ (radio) Stretch to 16:9   ‹video›
│  │  ├─ (radio) Stretch to 16:10   ‹video›
│  │  ├─ (radio) Stretch to 1.85:1   ‹video›
│  │  ├─ (radio) Stretch to 2.35:1   ‹video›
│  │  ├─ (radio) Crop to 4:3   ‹video›
│  │  ├─ (radio) Crop to 16:9   ‹video›
│  │  ├─ (radio) Crop to 16:10   ‹video›
│  │  ├─ (radio) Crop to 1.85:1   ‹video›
│  │  ├─ (radio) Crop to 2.35:1   ‹video›
│  │  ├─ ────────
│  │  ├─ Reset Position   ‹video›
│  │  ├─ Move Left   ‹video›
│  │  ├─ Move Right   ‹video›
│  │  ├─ Move Up   ‹video›
│  │  └─ Move Down   ‹video›
│  ├─ Framing ▸
│  │  ├─ Reset Framing  [Numpad5 / Alt+Backspace]   ‹video›
│  │  ├─ Zoom In  [Numpad9 / Alt+Plus]   ‹video›
│  │  ├─ Zoom Out  [Numpad1 / Alt+-]   ‹video›
│  │  ├─ ────────
│  │  ├─ Pan →  [Ctrl+Numpad6 / Alt+ArrowRight]   ‹video›
│  │  ├─ Pan ←  [Ctrl+Numpad4 / Alt+ArrowLeft]   ‹video›
│  │  ├─ Pan ↑  [Ctrl+Numpad8 / Alt+ArrowUp]   ‹video›
│  │  ├─ Pan ↓  [Ctrl+Numpad2 / Alt+ArrowDown]   ‹video›
│  │  └─ Center Frame  [Ctrl+Numpad5]   ‹video›
│  ├─ ────────
│  ├─ Image Processing ▸
│  │  ├─ Flip Vertically  [Ctrl+P]   ‹video›
│  │  ├─ Flip Horizontally  [Ctrl+Z]   ‹video›
│  │  ├─ ────────
│  │  ├─ Gaussian Blur  [Ctrl+B]   ‹video›
│  │  ├─ Sharpen  [Ctrl+R]   ‹video›
│  │  ├─ Edge Sharpen   ‹video›
│  │  ├─ ────────
│  │  ├─ Deblock  [Ctrl+H]   ‹video›
│  │  ├─ Deband   ‹video›
│  │  ├─ 3D Denoise  [Ctrl+N]   ‹video›
│  │  ├─ Gradual Denoise   ‹video›
│  │  ├─ Temporal Denoise  [Ctrl+M]   ‹video›
│  │  ├─ ────────
│  │  ├─ Frame Doubling   ‹video›
│  │  └─ Motion Interpolation   ‹video›
│  ├─ Rotation ▸
│  │  ├─ Cycle  [Alt+K]   ‹video›
│  │  ├─ (radio) No Rotation   ‹video›
│  │  ├─ (radio) 90°   ‹video›
│  │  ├─ (radio) 180°   ‹video›
│  │  └─ (radio) 270°   ‹video›
│  ├─ ────────
│  ├─ Snapshot ▸
│  │  ├─ Save Current Frame  [Ctrl+E]   ‹capture›
│  │  ├─ Save Current Frame As…   ‹capture›
│  │  ├─ Copy Current Frame  [Ctrl+C]   ‹capture›
│  │  ├─ ────────
│  │  ├─ Save Rendered Frame  [Ctrl+Alt+E]   ‹capture›
│  │  ├─ Save Rendered Frame As…   ‹capture›
│  │  ├─ Copy Rendered Frame  [Ctrl+Alt+C]   ‹capture›
│  │  ├─ ────────
│  │  ├─ Burst…  [Ctrl+G]   ‹capture›
│  │  ├─ Export Clip…   ‹capture›
│  │  ├─ Export Tasks…   ‹capture›
│  │  ├─ ────────
│  │  ├─ (radio) Capture as PNG   ‹capture›
│  │  ├─ (radio) Capture as JPEG   ‹capture›
│  │  ├─ (radio) Capture as BMP   ‹capture›
│  │  ├─ ────────
│  │  ├─ Include Subtitles in Rendered Frame   ‹capture›
│  │  ├─ Snapshot Settings…   ‹capture›
│  │  ├─ ────────
│  │  ├─ Snapshot Folder…   ‹capture›
│  │  └─ Open Snapshot Folder   ‹capture›
│  ├─ ────────
│  ├─ Picture Adjustments…   ‹video›
│  ├─ Reset  [Q]   ‹video›
│  ├─ Brightness -1%  [W]   ‹video›
│  ├─ Brightness +1%  [E]   ‹video›
│  ├─ Contrast -1%  [R]   ‹video›
│  ├─ Contrast +1%  [T]   ‹video›
│  ├─ Saturation -1%  [Y]   ‹video›
│  ├─ Saturation +1%  [U]   ‹video›
│  ├─ Hue -1%  [I]   ‹video›
│  └─ Hue +1%  [O]   ‹video›
├─ ────────
├─ Audio ▸
│  ├─ Volume +  [ArrowUp / Mouse:WheelUp]   ‹core›
│  ├─ Volume −  [ArrowDown / Mouse:WheelDown]   ‹core›
│  ├─ Mute  [M]   ‹core›
│  ├─ ────────
│  ├─ Audio Workbench…   ‹audio›
│  ├─ Audio Track ▸
│  │  ├─ No Audio Tracks   ‹audio›
│  │  ├─ ────────
│  │  ├─ Cycle Tracks  [Alt+A]   ‹audio›
│  │  ├─ ────────
│  │  ├─ (radio) Stereo   ‹audio›
│  │  ├─ (radio) Left Channel   ‹audio›
│  │  ├─ (radio) Right Channel   ‹audio›
│  │  ├─ (radio) Mono Mix   ‹audio›
│  │  └─ (radio) Swap Left and Right   ‹audio›
│  ├─ Output Device ▸
│  │  └─ No Output Devices   ‹audio›
│  ├─ ────────
│  ├─ Equalizer  [Shift+E]   ‹audio›
│  ├─ Preset ▸
│  │  ├─ (radio) Flat   ‹audio›
│  │  ├─ (radio) Voice   ‹audio›
│  │  ├─ (radio) Music   ‹audio›
│  │  └─ (radio) Cinema   ‹audio›
│  ├─ Audio Sync ▸
│  │  ├─ 50 ms Earlier  [>]   ‹audio›
│  │  ├─ 50 ms Later  [<]   ‹audio›
│  │  ├─ Reset  [?]   ‹audio›
│  │  ├─ ────────
│  │  ├─ (radio) Don't Keep   ‹audio›
│  │  ├─ (radio) Keep for This Run   ‹audio›
│  │  └─ (radio) Always Keep   ‹audio›
│  ├─ Processing ▸
│  │  ├─ Normalize  [Shift+N]   ‹audio›
│  │  └─ Crystalizer  [Shift+C]   ‹audio›
│  ├─ Reset   ‹audio›
│  ├─ ────────
│  └─ Show Album Cover for Audio Only   ‹visualizer›
├─ Emulator ▸
│  ├─ Open NES ROM…   ‹nes›
│  ├─ Stop NES   ‹nes›
│  └─ NES Tools…   ‹nes›
├─ ────────
├─ Toggle Fullscreen  [Enter / F11]   ‹host›
├─ Screen ▸
│  ├─ Picture-in-Picture   ‹window›
│  ├─ Mini Size  [`]   ‹window›
│  ├─ ────────
│  ├─ (radio) 0.5×  [1]   ‹window›
│  ├─ (radio) 1×  [2]   ‹window›
│  ├─ (radio) 1.5×  [3]   ‹window›
│  ├─ (radio) 2×  [4]   ‹window›
│  ├─ Fit to Video Aspect Ratio  [8]   ‹window›
│  ├─ ────────
│  ├─ When Playback Starts ▸
│  │  ├─ (radio) Don't Resize   ‹window›
│  │  ├─ (radio) 0.5×   ‹window›
│  │  ├─ (radio) 1×   ‹window›
│  │  ├─ (radio) 1.5×   ‹window›
│  │  ├─ (radio) 2×   ‹window›
│  │  ├─ (radio) Fit to Video Aspect Ratio   ‹window›
│  │  ├─ ────────
│  │  └─ Only Once   ‹window›
│  ├─ ────────
│  └─ Always on Top ▸
│     ├─ Cycle Mode  [Ctrl+T]   ‹window›
│     ├─ ────────
│     ├─ (radio) Never   ‹window›
│     ├─ (radio) Always   ‹window›
│     ├─ (radio) While Playing   ‹window›
│     └─ (radio) While Playing in Fullscreen   ‹window›
├─ ────────
├─ Playlist  [F6]   ‹playlist›
├─ Playback Info  [Tab]   ‹info›
├─ Playback Properties…  [Ctrl+F1]   ‹info›
├─ Workshop…   ‹workshop›
├─ Plugin Store…   ‹store›
├─ ────────
├─ Settings ▸
│  ├─ Options…  [F5]   ‹preferences›
│  ├─ ────────
│  ├─ Keyboard Shortcuts…   ‹hotkeys›
│  ├─ Mouse…   ‹hotkeys›
│  ├─ Ignore Default Shortcuts   ‹hotkeys›
│  ├─ ────────
│  └─ Appearance ▸
│     ├─ (radio) Vestra   ‹preferences›
│     ├─ ────────
│     ├─ Show Logo When Idle   ‹visualizer›
│     ├─ Choose Logo Image…   ‹visualizer›
│     └─ Restore Default Logo   ‹visualizer›
├─ ────────
├─ About Vestra Player…  [F1]   ‹info›
├─ ────────
└─ Quit  [Cmd+Q on macOS; no shortcut shown on Windows/Linux]   ‹host›
```
