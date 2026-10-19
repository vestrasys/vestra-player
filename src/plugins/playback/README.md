# Basic playback production candidate 0.1.1

Independent TypeScript interaction policy and ordinary native Slint bottom playback bar. Developer manifest has no version and loads main.ts. Production dist contains ESM JavaScript, package.json, Slint, and strict x.y.z manifest version 0.1.1. It has no runtime SDK dependency and requires no user Rust compiler. No sibling plugin import or provider identity special case exists.

## Build and verification

Run `node build.mjs [x.y.z]` from this directory, or pass its absolute path. It uses the repository SDK/compiler when available; local rebuild defaults use `/tmp/vestra-current-runtime-cache/sdk` and `/tmp/vestra-rebuild-toolchain-20261018/node_modules/typescript/lib/_tsc.js`. Explicit overrides are VESTRA_SDK, VESTRA_TSC, VESTRA_TYPE_ROOTS. SDK copies exist only in temporary type-check build input, which is removed after success; dist receives only plugin JS and Slint. The compiler checks source and package tests under strict/erasableSyntaxOnly. No npm install is required.

Actual results for this revision (2026-10-19):

- `node /tmp/vestra-current-products/playback/build.mjs`: exit 0, emitted production 0.1.1; type-check includes main, interaction, and both test files against current cached SDK.
- `node --test /tmp/vestra-current-products/playback/tests/*.test.ts`: exit 0, 6 passed. Covers own seek receipt advancement, external restart/open invalidation, cancel dropping unsent tail, final release sample, late error isolation, time parsing, and actual consumer output request ordering/mute exclusion. The output test uses a recording RPC double; it proves consumer ordering only, not audio or backend playback.
- `node` dynamic import of dist/main.js: exit 0 with exported activate and no repository SDK runtime import.
- Invalid production versions 01.0.0, 1.0.0-beta, and 4294967296.0.0: all rejected before mutating dist.

The original source provenance is recorded in SNAPSHOT-SOURCE.json: `/tmp/vestra-product-slices-QKa2uO/packages/0`, a current-rebuild artifact. That earlier debug source passed real Slint compilation/activation alongside playlist. Root actually verified file opening and pause/resume in that native combined slice. The first earlier Slint failure (conditional editor IDs) was repaired with EditorRow-local focus aggregation, not by weakening the compiler. Those native results do not certify this new production revision or its new callbacks.

## Behavior

One bottom mount; file selector; source close; pause/restart; progress drag with latest-only requests; accumulated ±5-second seek; time input; speed 0.25–4; volume 0–200%; real mute preserving volume; error/retry; view-menu preferred visibility. All menu paths include their Chinese leaf. Shared source facts come from source.state; source actions use submit/operation and stamps. Output uses playback.set and public output state. No pause-for-seek, backend Debug parsing, queue progression, or global pause/reset on dispose.

SeekFence advances only through this gesture's own completed operation ticket. External source/restart/control invalidates drafts and queued points. New explicit pointer cancellation discards unsent values, clears the local draft, and reads current state; already accepted seeks are not undone. Pointer-up submits its final point before releasing the drag lock. Volume Slider.released submits a final sample; latest-only merging keeps the last sample, and revision tokens stop an older equal-value completion from unlocking a newer request. Pending volume disables mute; pending mute disables and guards volume. Volume target is set before unmute, preserving the existing no-loud-restore ordering.

Only public unmodified canvas Space/ArrowLeft/ArrowRight are routed by the current host. Open has shortcut:null because modified Ctrl/Cmd+O is not implemented. See API_REQUESTS.md; declaration text is not advertised as a functioning shortcut.

## Remaining real boundaries

Author did not run Host/Cargo/GUI/physical audio. Maintainer must compile the revised Slint and activate the production dist, then check native pointer cancel/release, editor keyboard/focus restoration, 720px layout, fullscreen idle/focus locks, paused-seek frame update, and audio/video continuity. Volume/mute request ordering is tested; real speaker effects remain unverified. Native file selection/pause/resume results above belong to the earlier named debug snapshot only. Cmd/Ctrl+O remains a specific public API gap. Generic site-resolution business is outside this package.

CANDIDATE.json is written last as the publication marker. Consumers should copy the complete revision matching its hashes; they must not copy files while this marker is absent or being prepared.
