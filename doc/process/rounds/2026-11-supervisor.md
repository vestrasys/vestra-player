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
| B2a | not started | | | |
| B2b | not started | | | |
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
