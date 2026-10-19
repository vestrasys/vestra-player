# Incremental independent review — 2026-10-18

This is an incremental current-candidate review, not final acceptance. Read current local candidates and current SDK cache only. No Git history, old sources, repo temp, Host/Cargo/GUI or audio device was used. CLI harness exercises actual candidate entry code with simulated public Host calls; it does not prove native media or package operation results.

## Confirmed current defects

1. **P1 — NES Stop does not cancel preparation.** Reviewed `/tmp/vestra-current-products/nes/dist/main.js`, SHA256 `5c060fdf12fd233c27a499cd2b2c257eec05da2ef171369b5327b365f16de188`. In matching `main.ts:32–37`, stop returns if `active` is unset. `openRom` stores prepared producer only in locals (`main.ts:91–105`) and later submits it (`main.ts:133`) regardless of Stop. Harness paused `producer.prepare`, invoked Stop and observed “Stopped.”, resumed prepare, then observed an `openProvider` submission and “Running …”. A user can stop preparation and subsequently have NES replace current playback. Suggested fix: opening-session cancellation token, owned prepared resources, checks after every await and before source submission; await cleanup during disposal.

2. **P2 — Workshop metadata lookup failure permanently locks controls.** Reviewed `/tmp/vestra-current-products/workshop/main.ts`, SHA256 `9d1ec8ce291e5f47047104a1e7cac3ba4bfc272566035ba4a57ed8c81286e678` (draft, not frozen). `managed` sets `busy=true` at line 55, but `packages.installed` at line 56 is outside recovery. Injected transient failure leaves UI `busy=true`; a second rollback is rejected before another metadata request. Suggested fix: wrap the complete preflight in guarded cleanup, preserving busy only when a tracked accepted operation remains unresolved.

## Additional actionable source findings

- **P1 — Workshop loses accepted-but-response-lost install/rollback ownership.** `main.ts:60–61` clears busy when install/rollback RPC throws, without storing/querying an operation ID. The request ID is merely logged. The next user attempt can submit another mutation while the first continues. SDK exposes `packages.operation(operationId)`; use the deterministic public ID agreed with Host to query the original request before allowing another mutation. This is a source-proven missing recovery path; no real installation timeout was injected.
- **P2 — Workshop trial creation has no exclusion before its first await.** `main.ts:97–98` tests `busy/trial` then awaits `packages.trial` without setting either. Two rapid callbacks can create two trials, with only the last handle retained. Suggested fix: acquire exclusion synchronously before trial creation, retain/query the accepted handle and make stop/disposal wait for in-flight creation. Source review only.
- **P1 functional gap — YouTube and Bilibili Play remain extraction-only.** Current YouTube `main.ts` SHA256 `f8e773ffa8ac793a5f116841afa7a43b4d6e5ca6667c9d3fef7574a6aa049088` and Bilibili `src/main.ts` SHA256 `e695df6ff552ff6e968191fd765926cc2be8711020065a4b834bf889dae83f02` explicitly show missing-source-interface status after extracting, and never submit media. Existing extraction/download success cannot satisfy Play. SITE01 descriptor and real Host playback remain required.

## Capture and remaining acceptance

Read capture-complete-20261018.3 source, `main.ts` SHA256 `948a69d4954e291754e75c8edf1c700fcbcc83813e72c8097a4d4e2046c82718`. This review found no additional confirmed source defect in that revision's queue/freeze logic. That is not native acceptance. Maintainer independently reported Slint width/min-width and height/min-height compiler failures and owns the fix. Latest revision still requires compilation, real PNG/MP4/burst output, cancellation/publication outcome, source-change isolation and resource cleanup verification.

Playback, playlist, video/Gaussian, audio workbench, subtitles and combined ten-product operation were not independently exercised in this increment. Root's partial current GUI/CLI evidence remains partial. NES core/PCM evidence does not prove Host AV; latest prefill candidate still needs real Host AV acceptance. Full keyboard/quick operations/save interaction is explicitly deferred by the user's current NES scope. No paid model call was made. Workshop source does use real Pi Agent and real filesystem/compiler tools with a local mock model, but full generate/trial/install/rollback/native counter behavior is not accepted here.

## Reproduction artifacts

- `/tmp/vestra-acceptance-current/review-repro.mjs`
- `/tmp/vestra-acceptance-current/repro-results.json`
- `/tmp/vestra-acceptance-current/current-homebrew.nes` (generated from the current candidate's original homebrew generator)

CLI run: `node /tmp/vestra-acceptance-current/review-repro.mjs`, exit 0. Observations establish the two confirmed control-flow bugs above, not native Host correctness.

Author follow-up after this review: Workshop author reports a newer, not-yet-frozen revision fixes the metadata-failure busy latch and adds trialPending exclusion; timeout ownership recovery is being coordinated with package maintainer. Those changes have not been independently rerun here. Findings above remain tied to their recorded reviewed hash, and must not be attributed to later revisions without rechecking.
