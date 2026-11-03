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
| B2b | **merged #145 (fd198a48)** | `supervisor-b2b` C:\vestra\wt-sup-b2b (from 767fad55) | API maintainer | fixes A18 leak 2, D13; running-uninstall-reinstall test; narrow job-API dead_code allow |
| B2c | **merged** (registry evidence: 18 scenarios, only pre-existing noise) | `supervisor-b2c` C:\vestra\wt-sup-b2c (from fd198a48) | API maintainer | lazy.rs deleted; appearance.changed "what changed" (wire addition); doc excerpts; §13 B7 removal list |
| B2d | **merged — B2 complete** | `supervisor-b2d` C:\vestra\wt-sup-b2d (from e29c0982) | API maintainer | lanes replace StartQueue/FirstQueue, every start a job, list_state, remove first_runs.rs + start_timeout_by_host, B2 notes 1–5 |
| SDK bump | **merged #147 (5fb5a7b5)** | `sdk-1.5.57` C:\vestra\wt-sdk157 | API maintainer (2nd instance) | player → @vestra-player/sdk ^1.5.57 |
| B3 | **merged** (GUI check folded into the B4 release GUI test, VESTRA_LAZY=1) | `supervisor-b3` C:\vestra\wt-sup-b3 (main 0e8ba741 + spec tests) | API maintainer | live preset → B3; LIMIT 64 removed; rulings 1–4; 200-wake storm before/after; GUI test by coordinator after review |
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

- **From B3 (maintainer questions, coordinator rulings 2026-11-03):**
  - B3 limits apply only with `VESTRA_LAZY=1` until B6 (as §15 says); confirmed — B6 makes on-demand the only mode (unified-scheduling ruling).
  - Spawn on the UI thread costs ~30 ms each (storm max tick 121 ms, then ≤ 52 ms) — violates the UI frame-rate rule under load. The off-thread spawn worker + per-tick budget (§6.1, A19) is **added to B4's scope** (before the B4 release), with a storm re-measure: target no tick > one frame at display rate from supervisor work.
  - `ui_begin`/`ui_finish` (T9 "ready only after UI") not wired → B4 scope.
  - Storm tests need `MAX_PACKAGES` raised in a scratch build only (or 127 plugins).
  - SDK-site docs for B3 (bounded backlog, explicit busy answers, triggers held to ready, busy keeps a plugin running): points at the end of supervisor.md §17.12 → next SDK docs batch.

- **B3 design questions (spec author, 2026-11-03) — coordinator rulings, to record in supervisor.md §17 with B3:**
  1. Wake start timeout under B3 keeps registrations, like a spawn failure (§17.7 item 10, §15 B3 "keeps its menu entries"): the unit returns to Standby with entry points, no Revoke; the core's crash path must not send a start timeout to Failed+Revoke for a wake under B3. Precedent: Chrome MV3 — an extension whose service worker fails to start stays installed and the next event tries again. Test `k_start_timeout_of_a_wake_keeps_registrations_and_answers`.
  2. Work accounting distinguishes kinds: host→plugin requests awaiting a reply are capped by `request_timeout` (B6 value 5 min); plugin-declared long work (`commands.busy`, settings busy, etc.) is uncapped. `begin_work` gets a kind/flag for that (pure core API change, maintainer's call on shape). Since `standby_refused`/host busy move into `work.rs` in B3, declared busy enters `begin_work` with the uncapped kind. Test `w_declared_busy_has_no_request_cap`.
  3. LIMIT 64 in `activation.rs` is removed in B3 (done criterion); if `activation_begin` disappears, keep only `x_more_than_64_builtins_activate_at_once`.
  4. Failed-start menu items: under B3 (no backoff) entries stay enabled and a click retries the start; from B4, while `hold_until` holds (backoff, T20/R5), items are shown disabled with the reason and a click shows the OSD. §15 B3 wording to be split accordingly.
- B3 spec tests committed on `supervisor-b3-spec` (29 tests; 26 pass on main, the 3 above fail until B3). Merge that branch into the B3 implementation branch.

- **Done 2026-11-03 (18 scenarios + 2 main baselines, no B2c-caused difference):** B2c registry probes pending: the user is using the machine (game + own Vestra), system commit 86–97 % → probes not allowed (>85 %) and one player at a time. When commit < 85 % and no user Vestra runs: in `scratchpad\sup-b2c\` run `probe.ps1` for the remaining B2c scenarios (shared, 4 manage, classification probes) plus a second main run, then `diffall.sh`; confirm the wake-scenario `vestra.preferences` timing difference and re-run `lazy_activation` (one failure from a Bluetooth headset changing `vestra.audio` devices mid-run). Merge of B2c waits for this.
- SDK-site doc `host-and-node.md` (SDK repo) still cites `TRIGGERS`, `activations.begin(.., 15_000)`, the 250 ms poll → update in the next SDK docs batch, together with SDK use of `appearance.changed.changed`.

- **Resolved (user ruling 2026-11-02, "一律强制更新"):** cause is GitHub macOS arm64 runner capacity, not billing. SDK 1.5.57 = 1.5.56 content + release workflow where macOS does not gate publish (worktree `C:\vestra\wt-sdkrel`); v1.5.56 tag left alone (no tag moves). Old note: SDK v1.5.56 not on npm. Release run 41502365359: macOS jobs never get a runner (cancelled after 15 min, twice; started after the paid budget went to 0 at ~19:00 — macOS runners appear to be affected even for the public SDK repo), so `publish` is skipped. Options: user re-enables macOS runners/budget for the SDK; or allow a release workflow change making macOS non-blocking for publish. Not blocking the supervisor batches; the player keeps SDK 1.5.55 until then.

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

- 2026-11-03 ~07:35: B3 review → merge (all checks; own storm re-run: 200 woken, ≤ 4 concurrent, max tick 113 ms then ≤ 75 ms; probe tests for queue interleavings; i18n complete; b3.rs untouched). Non-blocking → B4/docs: GUI run (done-criterion) folded into the B4 release GUI test — B3 only acts with VESTRA_LAZY=1, default users unaffected; tick budget + spawn worker + O(due) timers → B4; SDK docs: an `activate()` awaiting its own command/UI callback waits until ready (T9) → document; `start_now` answers ok before a queued reload/enable spawn error → B4; held triggers dropped unanswered at shutdown (acceptable, B4 shutdown).

- 2026-11-03 ~07:10: B3 committed: limits live with VESTRA_LAZY=1, LIMIT 64 gone, WorkKind Request/Declared, failures keep registrations, explicit OSD answers (7 languages), refused resolveRequest fails at once (`owner.refuseResolution`). 1209 host tests; real-node suites pass. Storm (200 wakes, release ×2): main 63 woken + 137 could-not-start, 63 concurrent, max tick ~1.94 s → B3 200 woken, 4 concurrent, max tick ~120 ms then ≤ 52 ms, all events delivered. Registry 26 scenarios: lazy-off identical; differences explained by B3 rules. Review started.

- 2026-11-03 ~05:45: **#148 (B2d) squash-merged → main 0e8ba741. B2 complete.** B3 worktree created with the spec tests merged in; maintainer briefed (live preset to B3, rulings 1–4, request accounting kinds, storm measurement, registry diff, flips of B0 tests B3 changes on purpose).

- 2026-11-03 ~05:35: B2d follow-up re-review → merge (switch semantics checked on install/restore/first-run/timeout paths; hotkeys snapshot difference accepted: no mechanism in B2d, main-vs-main also differs, not user-visible). B2d PR + merge next; then B3 (merge `supervisor-b3-spec` in, rulings 1–4 in Open items).

- 2026-11-03 ~05:15: B2d follow-up committed (`start_job_ends_at_spawn` legacy switch, off from B3/B4; repro ported; runner-restart order; late RemoveData test; flaky test fixed test-side; docs). 1208 tests; real-node suites pass; manage scenarios = noise; boot-off session 1 shows a hotkeys ready-snapshot timing difference (final content equal) — sent to the reviewer to judge.

- 2026-11-03 ~04:35: B2d review → fix first: Restart-kind starts (reload, install target, restore, disable-then-enable) hold triggers and keep package_busy for the whole activation (B2c delivered at once / ended the wait at spawn) — legacy-visible, could hang an activate() awaiting its own command. Plus cheap items (runner-restart ordering, §17.11 text, late RemoveData test, lifecycle test flake 1/30). Remaining registry differences confirmed main-vs-main noise. Maintainer resumed on e873e489.

- 2026-11-03 ~04:05: B2d committed (b5b838fd) + main merged (e873e489). Lanes replace StartQueue/FirstQueue; every start/stop through the core (one Spawn and one Terminate executor, one deadline); packages.list from list_state; remove_unit per contract; B2 notes 1–5 closed with tests; ~20 transitional items removed; new legacy switches `deliver_while_starting`, `verify_before_first_run_place` (off in B3). 1206 workspace tests; real-node suites pass; registry diff 18 + 4 boot-order scenarios: only main-vs-main noise (two real differences found and fixed during probing). Ruling: `operation_pending` stays (package transaction stage, §2.3 KEEP). Process lapse: maintainer ran `git rm --cached` then `git restore --staged` (index restored). Independent review started.

- 2026-11-03 ~02:05: removed merged, clean worktrees (sup-b0/b1/b2a/b2b/b2c/spec, sdk157; SDK wake/rel), branches checked pushed and PRs merged; disk 31 GB. Spec test author started on B3 (new test files only, from the design tables) in parallel with B2d.

- 2026-11-03 ~01:45: SDK bump reviewed (lock integrity = npm, vendored files byte-identical to the published 1.5.57), **#147 merged → main 5fb5a7b5.** B2d (branched from e29c0982) will need main merged in before its PR (package files only, no overlap). B2d maintainer working.

- 2026-11-03 ~01:10: **#146 (B2c) squash-merged → main e29c0982.** Briefed in parallel (disjoint files): B2d maintainer (host) and SDK bump to 1.5.57 (second maintainer instance, package files). Disk 33 GB free.

- 2026-11-03 ~01:00: B2c last fix (first-run place freed before the `classifying` check; coordinator checked the diff, 1202 tests) + registry evidence: 18 scenarios, second main baseline; every difference also occurs main-vs-main or vanishes on re-run (hotkeys registration timing, wake park timing, idle park counts, storm listener order); classification fixtures and the four wake-event storms identical. B2c PR + merge next; then SDK 1.5.57 bump and B2d in parallel.

- 2026-11-03 ~00:40: user: not using the computer, free use, keep the loop until all goals are done; away > 14 h (until ~15:00). Machine load is now ours: probes and real-node suites can run (still one player at a time, 85 % commit / 8 GB disk guards, isolated profiles only).

- 2026-11-03 ~00:25: B2c follow-up re-review → merge; all four real-node suites passed (commit 81–85 %, no user Vestra). Maintainer: tiny first-run-slot release fix in `class_known`, then the remaining registry scenarios + second main baseline + wake re-run. B2c merges after the probe diff is clean.

- 2026-11-03 ~00:05: **SDK 1.5.57 is on npm.** B2c follow-up committed (eec2f3cc: B0 hold tests ported, one deadline for classification, `start_timeout_by_host` transitional switch until B2d, appearance.changed test); 1202 workspace tests. Commit load 83 %, no user Vestra → reviewer re-checks and runs the remaining real-node suites; registry probes after that (one player at a time).

- 2026-11-02 ~23:48: SDK #72 all green (macOS too this time), squash-merged (65bb17a), origin/main package.json = 1.5.57, tag v1.5.57 pushed; release workflow running. Then: player bump to 1.5.57 (small batch after B2c).

- 2026-11-02 ~23:45: B2c review → fix first (cheap): two B0 hold_event tests not ported; classification timeout double timer (late answer can overwrite a resident record); wake StartTimeout double timer (ready between deadlines → Failed{stuck} while running). lazy_activation 7/7 + lifecycle 11/11 passed at 81 % commit (the earlier failure was the Bluetooth device); shared_runner/package_install skipped at 86 %. Maintainer resumed. Registry probes still pending (machine use).

- 2026-11-02 ~23:25: SDK 1.5.57 (8fe3eb2) reviewed → merge; PR vestra-player-sdk#72 opened. Plan: merge when Linux/Windows checks pass (macOS may hang; owner ruling: don't wait for it), verify `origin/main` package.json = 1.5.57, tag v1.5.57, watch release, then npm view. B2c review running.

- 2026-11-02 ~23:05: B2c committed (37fae5fe): lazy.rs deleted, decisions in the core, adapter files, `Lazy`→`OnDemand`, `appearance.changed.changed`. 1197 workspace tests (lazy.rs tests ported); lifecycle ×3, shared_runner, package_install pass; lazy_activation 6/7 (Bluetooth device change mid-run). Registry probes only partly run: the machine is in use by the user (commit 86–97 %); wake scenario shows a `vestra.preferences` timing difference under load, unconfirmed. Process lapse: maintainer used `git mv` (staged rename, committed as is). Review started (code + cargo checks; real-node suites only if commit < 85 %).

- 2026-11-02 ~22:00: **#145 (B2b) squash-merged → main fd198a48** on local evidence. B2c worktree created; maintainer briefed (incl. appearance.changed "what changed" wire addition, host-and-node doc excerpts, §13 B2b nit and B7 removal list). npm still 1.5.55 (SDK release waits on the user: macOS runners).

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
