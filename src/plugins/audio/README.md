# 音频工作台 — revision 1.1.0

This is a new author reconstruction of the current audio product, written locally on 2026-10-19 because the previous workspace files became FileProvider dataless. It is not claimed to be byte-identical to revision 1.0.0.

The independent nonmodal Window opens at **音频 → 音频工作台…**. The Audio domain also contains EQ presets and reset; every command path includes its leaf. Menu and window share TypeScript settings. No bottom/sidebar mount is used, and no pause, restart, device volume or mute action is issued.

Six EQ bands support ±12 dB; presets change EQ only. Audio delay supports ±2000 ms; positive values delay audio. Loudness normalization uses a typed dynaudnorm node and clarity uses typed treble gain (0–6 dB). Reset clears all settings by replacing this plugin's owned `workbench` resource with an empty chain and zero offset. Host revocation owns resource cleanup.

One request runs at a time; subsequent edits retain only the latest queued settings. Every request has a fixed increasing revision and current source generation. A source or selected-track change advances a local epoch and queues neutral settings. All success/failure UI outcomes use the epoch, so even same-generation track changes cannot let an old failure block the new neutral update. Unknown outcomes remain a resource barrier and are checked using the exact original token; confirmed old failures release the barrier without restoring old edits.

Commands:

- `node --test tests/*.test.ts` runs the behavior suite.
- `node check.mjs` checks with this session's local TypeScript toolchain/current SDK cache and removes its temporary `.typecheck` directory afterward.
- `node build.mjs [1.1.0]` generates a self-contained production `dist/` with strict three-part u32 version, main.js and Slint UI. No runtime repository SDK import or typecheck symlink is packaged.

Actual checks on 2026-10-19: TypeScript check exit 0; nine model/controller tests passed; production build exit 0. Native Slint Compiler/activation is handed to the runtime maintainer. Native callback behavior, audible EQ/dynamics/delay effect, device behavior, close/reopen, and disable cleanup have not been verified by this author. Full design document recheck remains unavailable because those files are dataless; this revision follows the current task and SDK contract.
