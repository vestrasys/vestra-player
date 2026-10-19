# NES plugin 0.1.0

Fresh implementation against the current process-plugin SDK. The production package is `manifest.json` → `dist/main.js` → `dist/core.js`, with pinned `node_modules/jsnes` 2.1.0 (Apache-2.0, license included upstream). No Rust compiler or native emulator dependency is needed by users.

The 模拟器 menu opens an operating-system ROM file picker or the independent native Slint NES tool window. An iNES ROM is run by JSNES's 6502/PPU/APU; each 256×240 frame is converted to RGBA8 and each stereo sample to normalized 48 kHz f32. Public source/producer APIs deliver both streams to the host canvas and shared audio device. The plugin performs pending source activation and producer control ACK handling before writing packets, observes acknowledged epoch boundaries, and keeps a finite audio lead. Stop/replacement closes its socket and releases its own producer. ROM parsing and all future NES-specific controls/state remain plugin business.

This first slice intentionally contains no keyboard binding or save-state UI. It does not declare producer input capture. The generic input API remains unchanged. It does not distribute game ROMs.

## Validation

Run `node build.mjs`, then `node --test tests/*.test.mjs` in the local package. The build uses the fixed current toolchain and copies the current SDK into an isolated typecheck directory; production JS contains no SDK runtime import. Standard package users need neither that authoring toolchain nor the SDK.

Verified on 2026-10-18: strict TypeScript against the current SDK; production ESM imports; 3 tests pass. An original generated mapper-0 6502 ROM runs for 120 real core frames twice. Each run yields 95,844 stereo sample frames, finite normalized PCM, nonzero pulse-wave energy (1345.9468), correct RGBA dimensions/alpha, and a different final frame hash for different CPU-selected PPU palette colors. Tests also cover malformed/unsupported ROMs, pixel channel order, pending activation ACK ordering, source replacement and deferred cleanup preserving the newer session.

These checks prove actual core execution and plugin orchestration. The lifecycle test uses a mock transport and does not prove real host AV delivery, native window rendering, physical audible output, or device recovery. Those require the package maintainer's native runtime verification. Commercial mapper coverage and PAL timing are limited to JSNES support, not certified by these fixtures.

Upstream primary sources: https://registry.npmjs.org/jsnes/2.1.0 and https://github.com/bfirsh/jsnes . Source and production artifact hashes plus source time are recorded in PROVENANCE.json. The local candidate at `/tmp/vestra-current-products/nes` preserves this package when workspace iCloud/FileProvider files are reclaimed.

## Native startup correction

The first real ROM-open UI attempt failed with `producer first AV buffers are preparing`. Startup now writes a sample-zero video frame and at least 2048 contiguous stereo PCM frames using the prepared epoch, then waits for device ACK, at least 1024 received frames and video availability before selecting the producer. Source selection preserves these buffers. Later control ACKs remain separate. The build and updated lifecycle/core checks pass; real native ROM-open recheck is pending.

## Pending-open cancellation correction

Independent review reproduced Stop during producer preparation still selecting the ROM afterward. Each open attempt now has a revocable token, checked after asynchronous boundaries. Stop or deactivation invalidates it; cancellation after an accepted source submission closes only that selected generation and releases its prepared producer. Two controlled pending-boundary regressions cover prepare and source submission. Five core/lifecycle tests pass; these protocol tests do not replace native cancellation verification. The preceding native startup version did visibly render blue and red original ROMs in distinct host sessions; device sound was not independently established.
