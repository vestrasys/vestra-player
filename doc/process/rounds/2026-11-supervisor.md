# Round: plugin process supervisor (blackboard)

Coordinator record and blackboard for the supervisor project. Design: [supervisor.md](../../architecture/supervisor.md). Procedure: [supervisor-loop.md](../supervisor-loop.md). Started 2026-11-02 from main 0bc4d35d (host 0.6.5, #140, #141).

## User rulings (2026-11-02)

- One supervisor for all plugin processes; no lazy vs non-lazy modes; on-demand scheduling becomes the default (B6).
- It is process supervision modelled on systemd (not kernel CPU scheduling); module `supervisor`; our extra state is `standby`; wire values unchanged (`idle`).
- Limits and timings follow Chrome's precedent; process budget formula from Chrome; heap cap 512 MB per own process (ours, revisit if a legitimate plugin hits it).
- Built-ins never count against the installed package limit (#140).
- Build on the current implementation (no rewrite): keep the wire protocol, registries, replay and the 19 history amendments.
- All 16 open design questions answered by rulings, code, plan or precedent (supervisor.md §12.1).
- Run as a self-paced loop while the user is away; temporary role "spec test author" approved.

## State

| Batch | State | Branch / worktree | Agents | Notes |
| --- | --- | --- | --- | --- |
| B0 | approved (321ebd44), PR batched with B1 | `supervisor-b0` C:\vestra\wt-sup-b0 | — | reviewer 3× 10/10 (90 s each); for B2b: time the 20 s waits from the start of the wait (not `op.started`) and handle a stuck backend worker | fixes done; lifecycle_baseline 10/10 ×3; finding: the three 20 s timeout branches are unreachable today (reap ≤ 6 s, BackendCleaned unconditional) — pinned as "continues after grace, 20 s texts never appear"; B2b covers them in the core's simulation tests | 787 unit + lifecycle_baseline 6/6; known bugs pinned; not pinned: 20 s timeouts, member queue full, busy checks, per-tick budget (need a Host test constructor) |
| B0+B1 | approved (bb8530e1, B0 merged in); PR open | `supervisor-b1` C:\vestra\wt-sup-b1 | — | four fix rounds; 1164 workspace tests, supervisor 342, lifecycle_baseline 10/10 ×3 |
| B1 (history) | fix rounds done | spec tests 243/243; | B1 maintainer: F1–F4 fixed, rulings R5–R7 in (switch `BuiltinsFirst::{Queued, Installed}`), §17.6; 295 supervisor tests, 1074 host tests pass | `supervisor-b1` C:\vestra\wt-sup-b1 (a2dc0e95 core; a09036d7 merged spec tests) | API maintainer (B1): mod tests line + answers to the 27 questions in supervisor.md §17.2 + ListChanged check; spec test author: adapter to the real API | core: 825 host tests incl. 46 own; API deviations in supervisor.md §17.1; ruling: shared members (built-ins) stay outside max_active (A7), §4.4 to be corrected |
| B2a | **merged #144 (767fad55)** | `supervisor-b2a` C:\vestra\wt-sup-b2a (from 800e2bb1) | API maintainer | zero-diff evidence: B0 tests ×3 + VESTRA_TEST_REGISTRY dump diff (lazy on/off); flips A18 leak-1 test |
| B2b | **merged** (17114d53 + 2b0257fa) | `supervisor-b2b` C:\vestra\wt-sup-b2b (from 767fad55) | API maintainer | fixes A18 leak 2, D13; running-uninstall-reinstall test; narrow job-API dead_code allow |
| B2c | not started | | | |
| B2d | not started | | | |
| B3 | not started | | | |
| B4 | not started | | | release after |
| B5 | not started | | | |
| B6 | not started | | | release after |
| B7 | not started | | | |

## Design questions (from spec tests or reviews)

From the spec test author (2026-11-02, branch `supervisor-spec` 22fa2a0f; 238 tests, adapter layer in `tests/mod.rs`). Status: open — to be answered by the B1 maintainer against the design and precedent, contested ones ruled by the coordinator; answers recorded in supervisor.md.

1. §5 has no Rust signatures; test adapter guesses names (`UnitSpec`, `Tunables`, `Trigger`, `JobResult`, `by_pid`).
2. How B1's "every class Restart=no and revoke" and B3's limits are expressed (tunables vs switches).
3. Event inputs and `wake` carry no `now`: timers start from the last `run_jobs` time?
4. `job(u)` result after the job is no longer installed: `job_result(JobId)`?
5. No input tells the core that start-up has settled (event wakes muted until then).
6. T5: verify failure → own process, or start fails?
7. T8 in B1: Failed + Revoke, or Inactive?
8. T9: order of `ui_begin`/`ui_finish` vs `process_ready`.
9. T18 says a stuck reload ends Failed; §4.5 says it stays at `then`.
10. T18: how is the wake-stuck text observed (`wake` returns no JobId)?
11. T32: `set_enabled(true)` enqueues Start vs §5 "does not enqueue by itself".
12. §4.2 Stop[waiting] + Start: which stop reasons are not disable/uninstall?
13. Restart refused on a disabled unit: which error (§5 has only Busy; §9 says RPC "invalid")?
14. §4.2 Stop[running] + Start refuses for disable, but "enable while disabling" waits (reading: triggers refused, Start{Enable} waits).
15. Upgrade of a disabled unit in standby: refuse, or accept without starting?
16. JobMode::Fail: enable RPC while a disable RPC job is running → "busy"?
17. Interactive reserve + overrun: total max+2 or max+1?
18. §4.4 shared members count toward max_active vs A7 built-ins outside it (members are built-ins).
19. Own-process Builtin: first restart immediate or after 3 s?
20. Normal/Resident backoff reset after stable time? Within 5/10 min the 60 s cap is never reached.
21. Resources-answer timeout "today's `lazy.asking`" — today has no such timeout.
22. "Recently used" window undefined.
23. First-run classification 5 s timer: no Action asks for classification; when does it start?
24. Backoff/hold measured from exit or from reap?
25. `next_deadline` exact vs lazily-deleted heap entries.
26. Budget formula has no API entry (physical memory input).
27. Builtin "5 starts" vs Normal "5 crashes" in 10 min.

Status 2026-11-02: all 27 answered by the B1 maintainer in supervisor.md §17.2 (code changes for Q15 upgrade of a disabled unit, Q24 backoff from reap; Q18 ruled by the coordinator: built-ins incl. shared members outside max_active). Spec tests compile: 234/241 pass. Coordinator rulings on the remaining design questions: (a) trigger while Standby{hold_until}: Interactive answered at once with a reason; Start job waits for hold_until only if event triggers remain; (b) shutdown emits all Checkpoints before the first Terminate; (c) built-ins first: today's semantics under `Tunables::today()`, the design row under target tunables (switch). Four implementation fixes sent to the maintainer (T11 Resident retry, Stop{Removed}×trigger Dropped, Restart×trigger Held, runner re-placement via Replay).

## Open items

- **Needs the user:** SDK v1.5.56 not on npm. Release run 41502365359: macOS jobs never get a runner (cancelled after 15 min, twice; started after the paid budget went to 0 at ~19:00 — macOS runners appear to be affected even for the public SDK repo), so `publish` is skipped. Options: user re-enables macOS runners/budget for the SDK; or allow a release workflow change making macOS non-blocking for publish. Not blocking the supervisor batches; the player keeps SDK 1.5.55 until then.

- B2c follow-up: host `appearance.changed` gains a "what changed" field (theme/language; pure wire addition), SDK `appearance.ts` uses it when present → closes the woken-by-language extra callback and the rare lost theme change (review-sdkwake r2). Then bump the player's SDK.

- **Transitional unit fields `process`/`parked` (B2a) — user 2026-11-02:** do not preserve "one id with both a running process and a parked owner" long-term; it is the D13 class the new core forbids by construction (one process per unit). Hard deadline: removed in B2b (jobs drive units, the waits are replaced); B2b done check includes `git grep -n "\.parked\b\|\.process\b" src/host/src/supervisor` showing no transitional fields and the §17.8 entry marked closed. If the B2a review shows the overlap can be dropped already in B2a without a visible difference, do it in B2a. Same rule for every "transitional" item: each gets a named removal batch and a grep check in that batch's brief, so none survives silently.

- SDK/API inventory (researcher, 2026-11-02, vs SDK 1.5.55 and main 800e2bb1): no API shape change needed; manifest has no scheduling fields (resident/entry points/wake events are observed). Coordinator ruling on the one open decision: the 5 min request timeout (B6, `work.rs`) applies only to host→plugin requests awaiting a reply (e.g. `source.resolveRequest`); plugin-declared long work (`commands.busy`, settings busy) has no cap — Android foreground-service precedent; Chrome's 5 min limit is per request. Record in supervisor.md §17 at B3. Docs to update per batch: B2c/B2d `host-and-node.md` source excerpts (TRIGGERS, activations.begin, 250 ms poll); B4 crash restart/backoff, 512 MB heap, quit ≤1 s, `idle` meaning, when `failed` arrives; B5 pressure/budget standby as `idle`; B6 timings, resident never stands by, drop VESTRA_LAZY/"off by default", CHANGELOG "who must change: nobody". Optional: TestHost standby→wake helper (maintainer's call). No manifest activation/resident/memory/priority fields.
- SDK patch 1.5.56 in progress (worktree `C:\vestra\wt-sdkwake`, branch `wake-onchange`): appearance/fileTypes onChange after wake; session-storage "when cleared" wording. Release: SDK repo CI/npm — check how SDK releases run now that remote CI is off for the player (SDK repo may differ).

- B7 done criteria (user confirmed 2026-11-02: the new core fully replaces the old logic, no compatibility layer): remove `Tunables::legacy()` and every legacy-only switch and code path; remove the B2a `debug_assert` cross-checks; remove the non-on-demand path behind `VESTRA_LAZY` (unified scheduling ruling). Only the plugin wire protocol and SDK API stay unchanged. Maintainer to add this to supervisor.md §13 B7 in the next batch.

- No remote CI (user, 2026-11-02: private repo, paid Actions budget set to 0 until release). Every batch merges on local CI content. Releases after B4/B6 cannot use the tag → CI workflow: plan a local Windows release build; macOS artefacts cannot be built here — skip the macOS part and write it to the user at that point (needs the user), continue the rest.

- B2 notes from the final B1 review (review-b1d, non-blocking):
  1. Reinstall whose spec carries a checkpoint goes through `replay` (`mod.rs:1477`), which drops `last_pid` from `by_pid` while `stopping` still tracks it → later reap/cleanup ignored, every start fails Stuck until restart, owed `RemoveData` never sent. Unreachable today (uninstall drops the recording), but fix in the core at B2d (no replay while `stopping`, or keep the mapping) + test like probe j4/j5; B2d contract: reinstall carries no checkpoint.
  2. §17.7 item 19 incomplete: uninstalling a *running* (not parked) plugin then reinstalling without removeData runs two processes on one dir today (D13 class); the core waits — expected B2 difference, document and add a B0 expected-to-change test.
  3. B2d contract: `remove_unit` only while `unit.removed` is still true (a late old `RemoveData` after a reinstall must not remove the new unit).
  4. B2d: delete the recording at uninstall request time (the reinstalled unit's landing no longer emits `DropCheckpoint`).
  5. B2d: resolve the directory to delete per owner/request (two uninstalls across a reinstall emit one `RemoveData`).
  6. Shutdown does not kill a stuck tracked old process (as today) — B2b/B4.
- Build note: cargo in a fresh shell needs `C:\vestra\env.ps1` (mpv.lib), cargo runs from `src/host`, `npm ci` in `src/` for the real-node tests.

- Pending (user ruling 2026-11-02): rename `Tunables::today()` → `Tunables::legacy()` after the current B1 round, before re-review.

- Main has user-facing mojibake in session_storage.rs:67 ("1鈥?024 bytes") and a comment at :261 (found by the B0 reviewer) — fix in a separate small batch.
- Agents sharing the session scratchpad clobbered `test.log`: briefs now name a per-agent scratch subfolder.

- B0 finding: uninstall data removal happens at `Event::Reaped` (handle.rs) without waiting for `BackendCleaned`; supervisor.md T16/T23 assume both halves. Decide in B2b whether to keep today's order (data dir is plugin-owned; backend state is separate) — record the decision in supervisor.md.
- Host has no test constructor (needs libmpv + Slint), so several busy/queue behaviours can only be pinned once the pure core owns them (B1/B2).

- Known bugs to pin in B0: Book entry leaked when spawn fails after `activations.begin` (`load_package`); `Host.cleanup` only grows; disable then immediate enable can run two processes on one data directory (D13).

## Log

- 2026-11-02 ~21:55: B2b follow-up 2b0257fa re-checked → merge (non-blocking reaper, panic isolation, supervisor_tick back at reload_tick's place; wake-start owner order within a tick judged unobservable). Nit for B2c: §13 B2b row "same busy" → "except the D13 fix". B2b PR next.

- 2026-11-02 ~21:20: B2b review → merge (all checks, rulings 1–3 match, no blocking issue). Non-blocking notes sent back for a short fix round before merge: single reaper thread blocking `child.wait()` after kill (robustness vs old per-thread), reaper panic losing items, §17.9/§16 wording, `process_stopping` debug_assert → reported value, `package_wait` default. Reviewer re-checks the diff after.

- 2026-11-02 ~21:00: B2b committed (17114d53, 22 files): Unit::stopping, waits as core jobs via host_start, package_rpc_busy, data dir resolved at uninstall and removed at reap, multiplexed reaper (5 ms, kept for timing). A18 leak 2 and D13 fixed (B0 tests flipped); B2-2 not reproducible end-to-end on main (new instance loads after the 300 ms grace), pinned by a core test; item 20 left for B4. 1186 workspace tests; real-node suites pass (lifecycle 11/11 ×3); registry diff 9 scenarios: only pre-existing noise + the D13 busy reply. Rulings: (1) every start waits for the previous process (D13 class incl. crash-then-enable/reload, disable-then-install, uninstall data) accepted; (2) second install wait from its own start accepted; (3) enable no digest wait accepted (built-ins can't be disabled). Independent review started.

- 2026-11-02 20:15: SDK v1.5.56 release run 41502365359: all checks green except the two macOS jobs, which never got a runner (no steps, cancelled after 15 min) → publish skipped. Re-ran the failed jobs once. If macOS runners stay unavailable, ask the user (publishing without the macOS gate would need a workflow change). B2b maintainer working (20 files changed so far).

- 2026-11-02 ~21:05: B2a follow-up a8530443 (displaced owner returned, adapter reports; kind change = conflict) checked by the coordinator (core has no diag/print). **#144 (B2a) squash-merged → main 767fad55** on local evidence. SDK v1.5.56 release workflow queued (npm still 1.5.55); player SDK bump after npm shows it. B2b worktree created; maintainer briefed.

- 2026-11-02 ~20:45: SDK #71 CI green (Linux/macOS/Windows), squash-merged (3b2e07d), origin/main package.json = 1.5.56, tag v1.5.56 pushed → release workflow publishes to npm. Player bumps the SDK after npm shows 1.5.56 (with B2b or a small batch). B2a re-review → fix first on one point: `diag!` in the pure core (`identity.rs:227`) → move the report to the adapter; same-pid kind change also asserted; §17.8 wording. Maintainer resumed; reviewer said no full re-review needed after that — coordinator checks the diff.

- 2026-11-02 ~20:30: SDK re-review → merge. SDK PR vestra-player-sdk#71 opened (public repo, free CI); after green: squash-merge, check `git show origin/main:package.json` = 1.5.56, tag per the repo's release convention. Residual (not a regression, rare): woken by a theme change, then a language change before ready → held event carries the new language, theme change lost; same root cause as the documented limit. Follow-up: host adds "what changed" to `appearance.changed` (pure addition, maintainer's call) — schedule with B2c (wake events move into the core), then SDK uses it.

- 2026-11-02 ~20:20: B2a follow-up committed (single `hosted` owner, dev-load id check; impl-wide dead_code allow left for B2b: narrowing B2b done check). SDK fix committed (d2bc92f, `#eventLanguage`; `#notified` not initialised from the start read, or the wake bug returns). Both sent back to their reviewers.

- 2026-11-02 ~20:05: SDK 1.5.56 review → fix first: appearance theme `onChange` fires spuriously on language-only `appearance.changed` (host `language_set` broadcasts the unchanged theme; `#notified` started undefined). fileTypes, docs, version OK; `pack:check` failure is environmental (same on 1.5.55: GNU tar takes `C:` as a host). SDK maintainer resumed with the fix; possible later pure addition: host marks what changed in `appearance.changed`.

- 2026-11-02 ~19:55: B2a review → merge (all checks + real-node suites pass; semantics identical; A18 fix covers every failure path). Reviewer: the running+parked overlap is unreachable today (D13 is about unreaped stopped processes, B2b scope). Per user ruling, follow-up in B2a: one identity per unit instead of `process`/`parked` (debug_assert + release diag), dev-load double manifest read fix (activation booked under candidate.id vs process under manifest.id), narrow the impl-wide dead_code allow if cheap. Then a short re-review of the follow-up.

- 2026-11-02 ~19:40: SDK 1.5.56 committed on `wake-onchange` (8bf5f16): appearance/fileTypes `onChange` compare with the last notified value; tests (fail without the fix); session-storage clearing docs verified against host; local checks pass except `pack:check` (Windows tar; SDK repo is public, its CI is free — run there via PR). Process lapse: the SDK maintainer used `git stash push/pop` to test a revert (restored, no residue); briefs already forbid it — reviewers now told to use a `git archive` scratch copy. Independent SDK review started.

- 2026-11-02 ~19:30: B2a committed (dc802559): alloc_pid, by_id/by_pid replace plugin_ids/parked_ids (old kept for debug_assert), activation.rs moved into the core per unit (A18 leak 1 fixed, B0 test flipped), adapter skeleton `host/supervisor/`, module-wide allow removed; §17.8. 1170 workspace tests; real-node suites pass (lifecycle ×3, lazy_activation, shared_runner, package_install); registry dump diff (5 scenarios, 16 built-ins) clean except a `vestra.video` GPU-mode race also seen between two main runs. Independent review started. In parallel: SDK 1.5.56 patch (wake onChange) in `C:\vestra\wt-sdkwake`.

- 2026-11-02 ~18:50: user ruling: merge on full local checks without waiting for remote CI (remote may bill the user's paid quota, being turned off). Cancelled the running CI; **#143 (B0 + B1) squash-merged → main 800e2bb1.** B2a worktree `C:\vestra\wt-sup-b2a` created; maintainer briefed (identity to unit table, activations into Unit, adapter skeleton, drop module-wide allow; zero-diff evidence; CI doc + static steps in done checks).

- 2026-11-02 ~18:40: rustdoc fix d4fe0675 (comment-only diff, checked by the coordinator; local CI `cargo doc` -D warnings, fmt, clippy clean); pushed; CI re-running on #143.

- 2026-11-02 ~18:30: PR #143 CI: macOS green; Windows tests green (1129) but `cargo doc` with `-D warnings` failed on unclosed HTML tags `<name>` in supervisor doc comments (10 places). Maintainer fixing; lesson for briefs: add the CI `cargo doc` step (RUSTDOCFLAGS=-D warnings) to the done checks.

- 2026-11-02 ~18:50: round record copied into `supervisor-b1` (bd371fc4), pushed; PR #143 (B0 + B1) opened; CI running.

- 2026-11-02 ~18:45: re-review of bb8530e1 → **merge** (5 items fixed, no regressions, nothing met the blocking bar; 6 non-blocking B2 notes in Open items). PR for B0+B1 next.

- 2026-11-02 ~18:25: fourth fix round committed (bb8530e1): runner failure lands cleaned members, merged starts count from their request, legacy data removal at reap (switch `remove_data_waits_cleanup` on from B4), reinstall of a removed unit waits for the old process (today has no bug there); §17.7 items 16–20. Workspace 1164 tests, supervisor 342, lifecycle_baseline 10/10 ×3. Re-review started with an explicit blocking bar (regression, reachable legacy deviation, two processes on one data dir / data loss / stuck forever); other findings become B2 notes.

- 2026-11-02 ~18:00: re-review of 3292a203 → fix first. Earlier 4 stuck cases fixed, interleavings OK, spec setup change accepted (intent kept, R6 holds), B0 flake fixed. New: runner failure leaves a cleaned member stuck forever (regression), merged reload/enable jobs count 20 s from the stop, uninstall data removal waits for cleanup (today: at reap), reinstall of an id whose removed unit waits for a stuck process; docs §4.5/T18. Ruling: legacy removes data at reap as today (settles the B2b open item for legacy); reinstall never deletes the new install's data and never runs two processes on one data dir. Maintainer briefed (fourth round). Probe: scratchpad `review-b1c/probe`.

- 2026-11-02 ~17:45: third fix round committed (3292a203): stuck stop keeps tracking the old process, later starts wait 20 s from their request (legacy and target), Starting notice from spawn, exit checkpoint only after classification (spec `s_shutdown_checkpoints_before_any_terminate` setup got two classification answers, assertions unchanged — reviewer to judge), B0 quit test warm-up + medians; §17.7 items 11–15. 1123 host tests, supervisor 336, lifecycle_baseline 10/10 ×3. Re-review started.

- 2026-11-02 ~17:20: re-review of 7350f878 → fix first. All 9 earlier findings + API items confirmed fixed; D1–D3 and #10 judged sound (#10 matches today's `wake_failed` under legacy). New: after a stuck standby stop the unit lands in Standby and a later wake/enable/reload spawns beside the possibly-live old process; the 20 s counts from the stop, today from the request. Minor: "Starting…" from job creation not spawn, exit checkpoint before classification, Deliver-before-classify note for B2c, `enable_wait_busy` scope note, B0 quit-timing flake (1/3). Probe: scratchpad `review-b1b/probe`. Maintainer briefed (third round); same review scope after.

- 2026-11-02 ~17:00: rename committed (9228d69a; `legacy()`, dead `reclassify` removed; 1108 host tests pass). `supervisor-b0` merged into `supervisor-b1` (7350f878, clean) for one combined PR. Independent re-review started on 7350f878 (verifies the 9 findings + API items, rulings D1–D3, §17.7 #10, B0 tests ×3).

- 2026-11-02 ~16:20: spec author done (25d4bf2c): 14 tests moved to a B3 profile with legacy twins, stop-time Revoke, Development no-Checkpoint assertion, resident idle per D3; 329 supervisor tests, fmt/clippy pass. Maintainer briefed for the rename `today()`→`legacy()` + removal of the dead `reclassify` off path; then independent re-review.

- 2026-11-02 ~16:00: maintainer D2/D3 round committed (628819a9): switches per ruling, `resident_idle_standby` (today()–B5 on, target off), reclassify always on; extra §17.7 #10: stuck stop during a wake → `Failed{Stuck}` + Revoke in every preset (avoids a second instance on one data dir). Reclassify 5 s timeout moved to the B2c adapter. 330 supervisor tests pass; one clippy error in spec author's file. Spec author resumed to finish. Next: rename `today()`→`legacy()` (+ drop the unused `reclassify` off path), then re-review.

- 2026-11-02 ~15:50: user ruling: rename `Tunables::today()` → `Tunables::legacy()` (the name "today" goes stale after B6 flips defaults). Do it right after the current spec/maintainer round, before the re-review (maintainer renames in impl + tests + supervisor.md in one pass). User: keep the loop running until the goal is reached.

- 2026-11-02 ~16:10: B1 fix round 1 committed (27df8ebe): 9 deviations fixed with today()-switches, probe cases as sim tests, B2 API (exit text, enable/disable with busy pre-check, wake by reference, dead API removed); fmt+clippy pass; 14 spec tests conflict with today() reproduction (D1). Rulings: D1 A9/reload-carry spec tests run under the B3 preset with today() twins, reap-Revoke test → stop-time Revoke; D2 `enable_wait_busy`, `first_run_parks_when_idle`, `boot_wait_lists_stopped` flip in B3, `reclassify` stays on in all presets (re-record when behaviour changes; Chrome re-evaluates per run); D3 Resident idle-standby reproduced under today() via a switch, off in B6 (§3.3 holds in target). Spec author (tests/) and maintainer (impl + §17.7) running in parallel; then re-review by an independent reviewer.

- 2026-11-02 ~15:10: B1 review (53508ca0) → fix first. Checks pass (fmt, clippy, 295 supervisor tests, 1074 host tests), purity OK, but under `Tunables::today()` the core deviates from today in 8 probed ways (non-first-run standby after classify, Development checkpoint, disabled-Failed listed failed, second enable not busy, B3/B4 notifies/answers live, standby Revoke at reap, old pid still wakes, classify once) plus small ones and 5 API items for B2. Probe crate: scratchpad `review-b1/probe`. Coordinator merged origin/main into `supervisor-b1` (add/add on supervisor.md, kept B1's version) → 333fa088. Ruling: Revoke at once on disable/uninstall/reload from Standby (today's `forget_parked`; systemd socket stop precedent); recorded by the maintainer in §17. Maintainer briefed; same reviewer re-reviews after.

- 2026-11-02: design approved; supervisor.md, this record and supervisor-loop.md committed on `supervisor-design`.
- 2026-11-02: PR #142 merged (design, loop procedure, this record as of the first commit); later blackboard edits stay local on `supervisor-design` and go out with the next batch PR.
- 2026-11-02: PR #142 (design, loop procedure, this record) opened; CI running. Loop started. B0 and B1 maintainers and the spec test author briefed in parallel (disjoint files); disk 22.7 GB free.
