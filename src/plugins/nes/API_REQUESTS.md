# Current SDK usage and boundary

No new host API requested for this slice. The package consumes `commands.register/onExecute`, `dialogs.open`, `ui.open/set/show/close/onCallback`, `producer.prepare/connect/state/ack/release`, and `source.submit/operation/state`. Manifest capabilities are the actual current finite names confirmed by the package maintainer: producer.av, commands, events, playback.read, playback.control, ui.window, ui.dialog. Menu domain is canonical 模拟器.

The producer sample cursor uses the epoch's absolute sample coordinate. Control transitions reset the cursor to baseSample; packets wait for acknowledged epoch state. Source submission remains pending until the operation terminal result is observed. Stream payloads are RGBA8 and 48 kHz stereo normalized f32, with <=4800 stereo frames per packet.

No input capture is declared in this first slice. If future controller support is added it will use generic declareInput/ackInput and plugin-owned physical-key-to-NES mapping. Save-state serialization, ROM classification, controller layout and NES quick actions require no NES-specific shared host semantics. Input ACK handling exists defensively in the pump but no keyboard feature is claimed.

Unverified here: native Slint compilation, package validation/install, actual producer socket/device consumption, physical sound, host shutdown during an unresolved OS file dialog and system device recovery. Native verification belongs to the current package/runtime maintainer. Author tests must not be presented as those checks.
