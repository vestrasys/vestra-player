# 画面工具 · revision 1.0.1

This revision was reconstructed from the author's current session after all earlier source files became FileProvider dataless. It is a new revision and is not asserted byte-identical to the earlier candidate.

Two canonical full menu paths, `视频 → 画面调节…` and `视频 → 高斯模糊…`, open the same independent Slint Window. Neither entrance occupies the bottom or sidebar slot. Brightness, contrast, saturation, gamma, blur, presets and reset share one package-owned `picture` processing resource. Hide preserves settings/effect; disabling empties only this resource; host owner revocation removes only this package's contribution. Presets and transformations remain package code.

The queue keeps one request in flight and only the latest pending snapshot. Each immutable revision uses the observed media generation CAS. A failed transport is resolved using the same operation token; pending/unknown outcomes block new replacements. No source reopen, pause, legacy global picture property, sibling resource mutation, or effect-specific core API is used. Empty/loading media retains desired values until ready. Paused acceptance explicitly warns that the frame may update on resume.

Development manifest deliberately has no version and loads `main.ts`. Production `dist/manifest.json` is version `1.0.1` and loads `main.js`. Only erased type imports reference `../../sdk`; production has no repository SDK runtime dependency.

From the repository package directory, `node build.mjs 1.0.1` uses the repository compiler and SDK. For the approved local build, use `VESTRA_SDK_DIR`, `VESTRA_TSC`, and `VESTRA_TYPE_ROOTS` to point to the current runtime SDK and toolchain. Build validates a strict three-component u32 version without leading zeros.

Validation for this revision: real official TypeScript compiler exit 0, five logic/behavior tests exit 0, and build/tests disk logs. Tests exercise parameter mapping, queue/error behavior, actual plugin activation against a test double, canonical menu paths, shared tool state, hide/disable ownership, empty-to-ready source transition and unresolved-token recovery. These tests do not establish native Slint or actual media frame output. Native Compiler/activation and media testing belong to the shared maintainer; their result is reported separately.
