# Shared host UI specimen

This is a stage D infrastructure consumer, not a product plugin. Host API remains experimental/0.1; see doc/rebuild/api-candidates.md.

Build once with `cd src/host && cargo build --bin vestra-host`. From repository root, `node src/scripts/stage-native-evidence.mjs /absolute/media.mp4` stages only the current binary, SDK, runtime and this example into a fresh temporary macOS app and opens it via LaunchServices. It does not install plugins or build Rust. Node and the linked native dependencies must already be available. The launcher prints the exact app path and log directory; it stops after 300 seconds. Use media longer than the observation window.

The Commands button and video right-click open the same command snapshot. The current projection flattens the path for a finite popup experiment; final nested navigation, keyboard/focus restoration and shortcut execution are pending. Disabled example shows a reason. Pause / play uses source controls and updates the same command's checked state. Open native file dialog only returns a path or cancellation; it never opens media as a side effect.

Processing specimen opens an ordinary independent Slint window. Brightness uses typed eq/gblur; audio gain uses fixed resident volume/equalizer/treble/dynaudnorm after its first submission. The first change prepares its graph; subsequent gain changes use supported runtime commands. Hold and drag for at least five seconds while observing the *main* video/time, then pause and adjust brightness to check actual paused pixels. The sample's event handler is a simple experiment, not the final latest-only/single-in-flight product scheduler. No speaker continuity claim follows from command acceptance.

For native evidence, record the exact app/run, action, visible time/frame/pixels before/after and duration. Keep UI observation distinct from background renderer counters and JSON state. Close this app after observation; do not operate other applications.
