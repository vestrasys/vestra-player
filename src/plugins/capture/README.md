# Capture product candidate

Current `development-2026-10-18.1` SDK; `activate(host)` uses only public source/snapshot/jobs/processing/subtitles/commands/UI calls. Runtime imports only Node builtins, never the repository SDK or another plugin. Source types are type-only imports.

## Implemented product path

Four canonical commands share one nonmodal Window: 视频 → 截图与导出 → 保存当前画面… / 连拍… / 导出片段… / 查看导出任务…. Every command declares the complete path including its distinct leaf. First activation failure from the earlier candidate was a genuine command-path collision; the earlier Slint had passed. The original failure remains in the maintainer's `/tmp/vestra-current-candidates-Dw7rOE/result.json` and was not rewritten as a pass.

PNG, finite burst PNG and finite MP4 each support raw/processed and optional processed subtitles. Current-time screenshots/bursts omit the capture time so the host fixes actual media time at acceptance; explicit-time and clip ranges are validated. Burst count is 2–20, spacing .1–10 media seconds; the entire sequence must fit before duration, checked again against actual frozen time. The final confirmation shows source, generation/intent, time/range/sample preview, processing/subtitle/audio settings and destination. Save cancellation starts no snapshot/job. A changed source/intent while preparing rejects current-source work. Accepted/queued input is immutable and never silently refreshed after source/track/processing changes.

The form guard is acquired before the first await. At most five tasks are accepted: one runner and four queued immutable snapshots. Source/track selection is resolved before confirmation; clips include a specified internal track ID or none, including legal silent sources. Raw excludes effects/subtitles; processed requests confirmed processing/subtitle revisions. Device volume/mute are not baked into export options. No source open/seek/pause operation is called.

Burst creates an atomic unique child directory with Node `fs.mkdtemp` under the user-chosen parent and runs sequential public PNG jobs against one snapshot. Filenames include sequence and media time. Cancel stops further samples and requests active-job cancellation, retaining confirmed published PNGs. Only an empty plugin-created directory is removed; worker temporary files remain the public worker's cleanup responsibility. Single-file outputs suggest an unused numeric suffix with real filesystem checks. Publication still relies on the public worker's atomic no-overwrite operation to reject races.

The scrollable task model displays original source, creation time, fixed range, kind/mode, backend lifecycle, known published-count progress, destination, error and worker facts. Query reads real public job handles. Cancellation waits for backend terminal outcome; publication that was too late to cancel is reported from its actual outcome. Lost mutation acknowledgements query the same shared operation ID and validate recovered results; no new file-writing operation is sent to compensate for an unknown result.

Failed/cancelled input is retained up to five minutes; explicit retained-input retry transfers the same snapshot into a new task and chooses a new output path. Current-source retry creates a fresh snapshot after confirmation. Expired/released input requires current-source retry. The history retains at most twenty tasks and releases removed snapshot resources. Output opening verifies a completed original public job, matching destination and existing regular file, then calls the platform opener with `spawn(..., {shell:false})`. Burst opens its output directory. Closing/hiding the Window does not cancel work; deactivation requests active cancellation, releases snapshots and removes empty owned directories. Host owner cleanup remains the final process/lease fence.

## Actual author evidence

- Local `build.mjs` with TypeScript 5.9.3: exit 0, ordinary production JS, Slint and strict `version: 1.0.0` manifest emitted.
- Strict TypeScript: exit 0 with `--noEmit --target es2022 --module esnext --moduleResolution bundler --allowImportingTsExtensions --strict`, using a temporary copy with only SDK type-import paths remapped to this turn's current SDK cache. Check copy removed before publication.
- `node behavior.test.mjs`: exit 0. Range/burst bounds, generation/intent identity and no runtime SDK import; duplicate callbacks excluded before source read; one runner/four queued; sixth input refused; queued cancellation; source replacement does not refresh queued snapshots. The orchestration part uses an explicit public-contract stub and is not product output evidence.
- Real filesystem unit checks: actual existing files produce incrementing suggestions; actual `mkdtemp` calls reserve distinct child directories; temporary test directories removed. These files contain test text, not fabricated media.
- Earlier Slint validation passed; latest layout and activation go to `/root/rebuild_packages` for real compiler/host work. Author ran no Cargo, GUI, host, actual media worker or audio process.

## Boundaries still requiring real acceptance

Real system Save/directory panel cancel/confirm/overwrite races, native menu and array-model binding, focus restoration/Escape/close and constrained-size scrolling, actual PNG pixels/dimensions/SDR conversion, burst sample frames/partial cancellation, MP4 codec/duration/audio/subtitles/preroll, source-file replacement, retained retry after provider teardown, opener behavior, playback continuity and owner termination all need native/worker evidence. No percentages are invented where MediaJob exposes only lifecycle/publication/facts. Encoder availability is shown through actual worker failure; the SDK has no codec preflight query. SDR/encoding descriptions are contract expectations, not this author's output measurements.

Development manifest is versionless with `entry: dist/main.js`. Managed production root is `dist/` with `entry: main.js`, `version: 1.0.0`. Do not pass the production manifest to legacy `plugins.load`.

## FileProvider and synchronized candidate

Earlier source bodies were evicted after checks, blocking PID 36372 (build) and its successor 36367 (test); both were terminated. The author reconstructed the same slice from this turn in `/tmp/vestra-current-products/capture/`, then implemented the remaining paths there. This is a new byte revision, not equality with old evicted files. `publish.mjs` writes each src file atomically, immediately verifies bytes, records nonzero sizes/SHA256 and publishes `freeze-ready.json` last. Source remains authoritative. No old artifacts, Git history, sibling implementations or old tests were read.

## Compiler correction 2026-10-19

complete.3 failed the real Slint compiler because Window specified width/min-width and height/min-height simultaneously. Original result: `/tmp/vestra-current-candidates-ZY0JMg/result.json` (`code: slint`). Revision `capture-window-size-fix-20261019.4` changes only the root initial dimensions to preferred-width 680px / preferred-height 560px, retaining min-width 520px / min-height 400px and scrolling. Build and strict TypeScript rerun: exit 0. Behavior tests were not repeated for this layout-only correction; their prior result remains historical. New real Compiler result and all actual media acceptance remain pending.

## Native opening correction 2026-10-19

Revision .4 passed real manifest/digest/production-JS/Slint validation and managed install/activation (`Wb0Qce/result.json`), but root clicked 视频 → 截图与导出 → 保存当前画面 twice and no capture tool appeared. Activation readiness was therefore not evidence of a usable tool Window. Actual SDK ui.open accepts a list of declared callbacks; the plugin incorrectly included `section`, which is a Slint property. The host binds callbacks before showing the Window, so this undeclared callback blocked opening.

Revision `capture-open-callback-fix-20261019.5` removes that pseudo-callback and its dead event branch. Menu-driven open failures now write the concrete host error to stderr and request a small ordinary `capture-error.slint` Window with only a message property and no callbacks. Repeated failure reuses it; successful capture opening closes it. This uses existing ui.open/set/show/close only, without a host API or UI architecture change. Build, strict TypeScript and behavioral contracts reran: exit 0. New checks compare the exact requested callback names against the shipped Slint declarations and inject an opening rejection to assert a visible-error Window is requested; these checks still do not prove native window display. The maintainer/root must verify the actual menu opens the tool and the error case is visible. All capture media-output acceptance remains pending.
