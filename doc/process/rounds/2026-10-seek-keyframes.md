# 2026-10 round: keyframe seeking for step seeks

Started 2026-10-26. **This document is the only progress record for this round.** Status: host side and SDK API merged (#46, SDK 1.5.20, player 0.5.1); the playback plugin side is being redone (see "Record note").

## User ruling (2026-10-26, binding)

- The user chooses between keyframe and exact step seeks, as in other popular players. The behaviour defaults to keyframe seeks, like mpv's arrow keys.
- The option is **on by default** ("jump to keyframes only"). The default steps stay 5 / 30 / 60 / 300 s.
- No "seek twice" behaviour. The user rejected showing a keyframe and then correcting to the exact frame, because no other player does that. Each press is one seek of one kind.

## Background (coordinator measurements, Windows 11, installed build of main 19fd7b6)

- Today every step seek is exact: `core_actions.rs` plans an absolute target, and `source_backend.rs:1464` runs `seek <t> absolute+exact`.
- 4K60 clips with a keyframe every 2 s (H.264, HEVC 10-bit): single presses play almost smoothly, with brief dips to 41–52 frames/s.
- 4K60 clips with a keyframe every 10 s: each press freezes the picture for about 0.6–1.5 s. Holding the arrow key gives 2–4 frames/s until the key is released.
- Logs: `C:\vestra\logs\seek-*.err`. Clips: `C:\vestra\clips\*-gop10.mp4`.

## Requirements the coordinator noted for the request

1. **Never stuck.** In keyframe mode a forward step lands on a keyframe at or after the target, and a backward step on one at or before it. Every press moves in its direction, even when the step is shorter than the keyframe interval. This matches mpv's relative keyframe seeks.
2. The OSD shows the **real landed position**, not the computed target.
3. The option applies only to **step seeks** (the arrow-key steps 1–4). Seek-bar clicks and drags, Jump to Time, chapters, bookmarks and A-B loop stay as they are; frame stepping is unaffected.
4. The option lives in the playback plugin's "Seek Steps…" window as a checkbox, on by default, persisted with the steps.
5. Repeated presses still add up (`backend.rs:883`) without getting stuck on the same keyframe.

## Progress

- [x] §4.1 request (playback author; summarised below, see Record note)
- [x] Maintainer reply (§4.3) and implementation (host + SDK patch): merged in #46 (host 0.5.1, SDK 1.5.20)
- [x] Plugin implementation (playback author, redone 2026-10-26 in worktree `C:\vestra\wt-kf`, branch `keyframe-seek-plugin` from main f831c9b; uncommitted, waiting for the coordinator)
  - `vestra.playback` 0.2.1, `minPlayerVersion` 0.5.1 (section 4 of the reply). Uses the real SDK 1.5.20 API with no casts.
  - Seek Steps…: a "Jump to keyframes only" checkbox below the four rows, on by default (`settings.json` `seekKeyframes`, default `true`; a value that is not a boolean falls back to `true`). OK saves it with the steps; Cancel and an invalid row save nothing. Translated in all 6 locales (zh-Hans "仅跳转到关键帧").
  - `player.setStepSeek({ seconds: steps[0], keyframes })` on every activation (also for step 5) and on every OK.
  - Tiers 2–4 read the setting at each press. On: `plugin.host.run("seek-forward" | "seek-backward", { seconds })`, with no plugin OSD (the host shows the landed position). Off: the old `seekBy` path, unchanged.
  - Fixed on the way: after Cancel, opening Seek Steps… again kept the cancelled edits on screen, because a hidden window stays `opened`. The window now shows the saved values again (tested).
  - Tests (SDK TestHost keyframe model): the default is on and the pair is sent at start; OK, Cancel and invalid rows; a restart restores the setting; a non-boolean stored value falls back to on; tiers 2–4 go to `host.run` with their step when on (`coreSeeks` landed 50/0/100/10, the host's OSD) and to `seekBy` when off; no plugin seek OSD in keyframe mode; turning the setting on in the window changes the next press. The previous "relative seeks accumulate" test now runs explicitly in exact mode. Assertions poll for every value they read (#43, #47).
  - Local checks: `plugin check/build playback` pass; `plugin test playback` passed 5 of 5 runs (90/90 each); `format:check` and `boundaries` pass; `ui:shots playback --lang en,de` gave 22 images with 0 problems, and the Seek Steps window was checked in both languages.
- [ ] Coordinator GUI check on the 2 s and 10 s keyframe clips (single presses and holding the key; also hold Ctrl+→, since tiers 2–4 now await `host.run` until the seek lands)

## §4.1 request (playback author, 2026-10-26) — summary

The original text was lost with an uncommitted worktree (see Record note). In short: step 1 is planned by the host with a hard-coded `absolute+exact` seek (`source_backend.rs:1464`), and `setStepSeek` only carries `seconds`, so a plugin cannot choose keyframe seeks. Steps 2–4 go through the plugin's own exact seeks. A plugin also cannot learn the landed position. Accumulating presses from computed targets gets stuck (keyframes at 0/10/20 s with 5 s steps). The author proposed `setStepSeek({ seconds, keyframes })` and left the shape to the maintainer.


## Maintainer reply (§4.3)

Written 2026-10-26 by the API maintainer against main 19fd7b6 (host 0.5.0, last tag v0.5.0) and SDK 1.5.18 (`C:\vestra\sdk`, a4bd3ad). mpv references are to commit e471496c56 (`src/packaging/deps.json:22`), read from `player/playloop.c`, `player/command.c`, `demux/demux.c` and `demux/demux_lavf.c` at that commit.

**Ruling: accept, rewritten.** Tier 1 is accepted in the requested shape: `setStepSeek({ seconds, keyframes })`. Tiers 2–4 get no new seek primitive. Instead, `plugin.host.run("seek-forward" | "seek-backward", { seconds })` runs the host's own step-seek action with a different step. The host already does this for PiP skips. No `keyframes` option is added to `player.seek`, `seekBy` or `Scrub.seek`, and no landed position is returned. In keyframe mode the call resolves once the seek has landed, and `player.state.position` is current after the `await`. SDK 1.5.19, host 0.5.1.

### 1. Classification and alternatives

- **Classification:** a capability gap, not a defect. Every step seek runs `absolute+exact` (`source_backend.rs:1463–1465`), which matches what `setStepSeek` and `Host.run` document (`sdk/src/plugin/types.ts:3833–3837`, `3883`). The user has ruled that the option exists (see "User ruling"), so this reply decides only the API shape.
- **Existing ways, checked and rejected** (I re-checked the author's list against the code):
  - *Plugin-side keyframe probing plus `player.seek`*: the seek is still `exact` (`player.ts:508`), so it does not remove the stall, and tier 1 stays exact. Rejected.
  - *Repeated `player.step("keyframe")`*: each call moves one keyframe (`core_actions.rs:559–566`) and a concurrent call rejects `busy` (`backend.rs` `step_now`, `self.stepping`). Rejected.
  - *The plugin takes ← → over from the core*: that reimplements a basic playback action, which §4.3 assigns to the core, and the plugin still could not see where mpv landed. Rejected.
  - *`host.run("seek-forward")` as it stands*: this is the right entry point (one host action with shared accumulation and the host's OSD), but it carries no step (`main.rs:3566–3573` reads only `action`) and no mode. **This is what is extended**, by one optional field.
- **Why the core changes (§4.3 item 4):** the mode lives in mpv commands that only the host issues, and the landed keyframe is mpv state that only the host sees. Step seeks are a basic playback action that already belongs to the core (`core_actions.rs:406–420`).

### 2. API shape (SDK 1.5.19, `@player 0.5.1`)

**2.1 Tier 1: `Player.setStepSeek(options: { seconds: number; keyframes?: boolean })`**

- `keyframes` is optional, and a missing value means `false`. The pair is written as a whole: the newest call of any plugin wins, as `seconds` does today (`main.rs:413–414`, `2897–2903`), and the pair reverts when that plugin stops (`main.rs:662–665`). The host default stays exact.
- A `keyframes` value that is not a boolean is rejected `invalid`, both in the SDK (`player.ts:584`) and in the host (`main.rs:3577–3587`).
- Doc text: *"`keyframes: true`: the step seeks (`seek-backward` / `seek-forward`, including `host.run` with `seconds` and PiP skips) land on a keyframe instead of the exact time: forward on the first keyframe at or after position + step, backward on the last at or before position − step, and they always move. Sources without moving video seek exactly. Seek bar, `player.seek`, scrubs, chapters, loops and `step()` are unaffected."*

**2.2 Tiers 2–4: `Host.run(action, options?)`**

```ts
run(action: HostAction): Promise<void>;
/** @player 0.5.1. One step seek of `seconds` (0.1–3600 in tenths) instead of the setStepSeek step:
 *  the same checks, mode, accumulation and on-screen text as ← / →. */
run(action: "seek-backward" | "seek-forward", options: { seconds: number }): Promise<void>;
```

- The host reads `params.seconds`, validates it with `valid_step` (`core_actions.rs:466–471`), and calls the existing `run_core_action_with(action, Some(reply), Some(seconds))` (`main.rs:2955–2999`). PiP skips already use this path (`main.rs:1368`, `pip.rs:257–263`). `seconds` together with any other action is rejected `invalid`.
- Why this shape and not `seekBy(…, { keyframes })` or a new `seek` option:
  - (b) Shared accumulation comes for free: all four tiers go through the same `host.action` in the playback worker (`backend.rs:867–903`).
  - Requirement 2 is met by the host: the OSD is the host's own template, built from the landed position (section 3), so the plugin builds no OSD text for these seeks.
  - The mode has a single source, the `setStepSeek` pair, so tiers 1–4 cannot disagree.
  - Scope guard (section 5): no general seek gains a keyframe mode, so the seek bar, Jump to Time and the others cannot start using it by accident.
- The wire is unchanged apart from the optional fields: `host.run { action, seconds? }` and `player.setStepSeek { seconds, keyframes? }` (`sdk/src/wire.ts:488`, `497–498`). There is **no** new `Control` field. A plugin-submitted `source.submit` intent cannot ask for a keyframe seek, which keeps the scope guard closed at the protocol level as well (`session.rs:46–50` stays unchanged).
- Plugin usage:

```ts
await player.setStepSeek({ seconds: steps[0], keyframes });            // activation and OK
// tiers 2–4:
action: () => keyframes ? plugin.host.run(delta < 0 ? "seek-backward" : "seek-forward", { seconds: Math.abs(delta) })
                        : p.seekBy(delta)                               // unchecked: exactly today's path
```

  With the box unchecked, the plugin keeps its own `seekBy` (`controller.ts:134–141`), so requirement (d) holds literally. In keyframe mode it must not call `notify` itself, because the host shows the text.

**2.3 How "never stuck" (requirement 1) is guaranteed**

The mpv command is `seek <±amount> relative+keyframes`, never `absolute+keyframes`:

- `relative` + keyframes: `mp_seek` sets `SEEK_FORWARD` only for a positive relative amount (`playloop.c:337–339`). `MPSEEK_KEYFRAME` disables hr-seek (`playloop.c:351`), so the flag survives (it is cleared only on the hr path, `playloop.c:383`). The lavf demuxer then seeks without `AVSEEK_FLAG_BACKWARD` when going forward, which finds a keyframe ≥ the target, and with it when going backward, which finds one ≤ the target (`demux_lavf.c:1927–1928`). The demuxer-cache path uses the same rule (`demux.c:3905–3913`: forward stops at the first keyframe ≥ pts).
- `absolute` + keyframes: for `MPSEEK_ABSOLUTE`, `demux_flags` stays 0 (`playloop.c:329–331`), so the seek **always rounds backward**. A forward step shorter than the GOP would land back on the keyframe it started from. That is exactly the stuck case, so `absolute+keyframes` is not used. (This matches the measurement in `core_actions.rs:559–561`.)
- The host computes `target = clamp(L ± k·step, 0, duration)` from the **real landed position L** and sends `amount = target − L`. Forward gives keyframe ≥ L + step > L, and backward gives keyframe ≤ L − step < L. Each issued seek therefore moves strictly in its direction, except at the boundaries:
  - backward with L = 0: the amount is 0, so no seek is sent and the OSD shows 00:00;
  - forward when there is no keyframe after the target: see the guard below.
- **Last-GOP guard (to verify in the GUI check):** with `SEEK_FORWARD` and no keyframe at or after the target, `av_seek_frame` fails, and mpv retries only in the backward direction (`demux_lavf.c:1977–1982`), so mpv's landing is unspecified. Host rule: if a forward keyframe seek settles at or before L and `eof-reached` is false, the host issues the same target once as today's `absolute+exact`. This is the only exact fallback. It covers a step past the last keyframe and is not "keyframe, then correct" (the user ruling), because the keyframe seek did not produce a forward frame.

**2.4 Repeated presses (requirement 5): serialized from the landed position**

Today (`backend.rs:883–903`) the next press within the window uses the previous **computed** target as its base. In keyframe mode that sticks, as the author's example shows (5 + 5 = 10 lands on 10 twice). Leaving the merging to mpv does not help either:
- `queue_seek` adds up unexecuted relative amounts (`playloop.c:484–488`);
- `MPSEEK_FLAG_DELAY` waits for a frame of the previous seek only for 0.3 s (`playloop.c:524–525`);
- after that, a relative seek is based on `last_seek_pts`, which is the **requested** target and not the keyframe ("Use the target time as current position … until a new video frame has been decoded", `playloop.c:431–433`, read by `get_current_time` `playloop.c:563–568`). That gives the same stuck case.

So the host serializes keyframe step seeks in the playback worker (`backend.rs`, a new `KeySeek` slot next to `seek` at `backend.rs:841` and `stepping` at `:843`):

1. **Press while idle:** L = mpv `time-pos`. Send one `relative+keyframes` seek and remember `{ generation, base: L, owed: 0, completions: [this press] }`.
2. **Press while in flight** (same `generation`): `owed += ±step` (tiers 1–4 and directions add up and net out). Push the completion. **Do not** touch mpv.
3. **Settled:** the same rule as `poll()` (`backend.rs:1007–1022`): `!seeking` and the position moved, or `eof-reached`, or 1.5 s passed. Then L' = `time-pos`.
   - If `owed ≠ 0`, send the next seek from L' as in 2.3 and carry the completions over.
   - Otherwise complete them all at once with one OSD text.
4. **Ended early:** a source change (`generation`, as `backend.rs:884–886`) or an exact seek submitted in the meantime (seek bar, Jump to Time, …) ends the slot. The completions resolve with no OSD text and do not reject. An mpv command error rejects `backend`.

Result for keyframes 0/10/20, L = 0, step 5, two quick → presses: the first lands on 10, and the second is owed and then sent from 10 (target 15), landing on 20. That gives 10 and then 20, not 10 and 10. A held key moves one keyframe seek at a time, each from the previous landing. Since a keyframe seek decodes one GOP head, the queue stays short. Exact mode keeps today's code path unchanged (`self.seek`, computed base, `Control::Seek`).

The keyframe command goes to mpv directly, like `player.step("keyframe")` already does (`backend.rs` `step_now`, `StepPlan::Command`), and not through `Control::Seek`. This is safe because a step seek is enabled only in `phase_ready` (`core_actions.rs:101–103`), so the from-end handling (`source_backend.rs:1486–1496`) is never needed. It also keeps `Control` and its exact path untouched.

**2.5 Which sources**

Keyframe mode applies when `facts.video` (moving video, `source_backend.rs:715–720`) and `can_seek`. Audio-only files and cover art quietly use today's exact path. That is a property of the preference, documented in 2.1, and not a silent success of a request. Producer sources cannot seek at all (`source_backend.rs:1180–1189`, and the action is disabled by `can_step_seek`). This answers the author's open point: **fall back quietly to exact** where keyframes do not apply, rather than reject `unsupported`, because the flag is a standing preference set at activation, usually before any source is open.

### 3. OSD and state

- **Tier 1 and tiers 2–4 alike**, since they are one host action: in keyframe mode the action is deferred, the same way frame steps defer (`Completion::Action`, shown by `action_done`, `main.rs:3001–3015`). The text is the existing template `"Forward {} s: {} / {}"` / `"Back {} s: {} / {}"` (`core_actions.rs:412–418`): the label is the step of the **last** press, the time is the **landed** position L', and then the duration. Coalesced presses complete together with one text. For the author's acceptance case 3 (from 12, step 5): "Forward 5 s: 00:20 / …".
- `core_actions::plan` gets a keyframe variant, for example `Plan::KeySeek { forward, step }`, which carries no target and no text. The text is built at settle time by a new `core_actions::key_seek_notice(forward, step, landed, duration)`. In exact mode `plan` returns exactly today's `(Plan::Seek(target), text)`.
- **What the plugin gets for tiers 2–4:** the `host.run` promise resolves **after** the landing in keyframe mode (in exact mode it resolves on submit, as today). The SDK already refreshes `player.state` after `host.run` (`sdk/src/plugin/services.ts:51–57`), so `player.state.position` is the landed position after the `await`. The return type stays `Promise<void>`: the plugin does not need the value, because the host shows the text, and returning a position from one core action would be a one-off.
- New source, superseded seek: resolves, no text (see 2.4, step 4).

### 4. Missing capability (old host or old SDK)

- **Old host:** `player.setStepSeek` reads only `seconds` (`main.rs:3578–3581`), and `host.run` reads only `action` (`main.rs:3567`). An old host would therefore run tiers 2–4 with the **tier-1** step and exact mode, which is worse than ignoring the field. The author's gate is **accepted and required**: `vestra.playback` sets `"minPlayerVersion": "0.5.1"`. The manifest check runs before anything else (`manifest.rs:119–126`, enforced at `manifest.rs:270`) and refuses with `versionMismatch`. The SDK marks both fields `@player 0.5.1` (as `presentedFps` is, `types.ts:3632`). A built-in ships with its host, so in practice the gate only catches a dev-loaded or side-loaded copy on an older player, but per §3 it must not report a false success there either.
- **Old SDK:** 1.5.18 destructures `{ seconds }` (`player.ts:584`) and would drop the flag. The type `setStepSeek(options: { seconds: number })` (`types.ts:3837`) also makes `{ seconds, keyframes }` a compile error (excess property), and `run(action)` with 2 arguments is a compile error as well. The plugin's `src/package.json` moves to `^1.5.19`, so a stale SDK fails the build instead of shipping silently.
- **New host, but keyframes do not apply:** quiet exact fallback (2.5). Arguments that are present but wrong are rejected `invalid` in both places: non-boolean `keyframes`, `seconds` out of range, and `seconds` with an action other than the two seek actions.
- I do **not** add a feature-detection call (`player.features` or the like). `minPlayerVersion` is the existing mechanism, and one plugin does not justify another.

### 5. Scope guard

- Unchanged, and stays `absolute+exact`: `Control::Seek` (`source_backend.rs:1463–1465`), and therefore the seek bar and drags (`Scrub.seek`, `player.ts:358`), `player.seek` (`player.ts:508`), Jump to Time, chapters, bookmarks (marks) and the A-B loop's jump back (its own exact path in `source_backend.rs`, not a step seek). No `Control` or `SourceIntent` field is added (`session.rs:46–50`), so no plugin can reach the keyframe mode by any route other than the two step actions.
- Frame steps, Previous/Next Keyframe (`player.step`, `core_actions.rs:528–570`), the `busy` rule and `frame_notice` are unchanged.
- Exact-mode step seeks keep today's code (`backend.rs:883–903`, the computed base and the 300 ms window).
- PiP skip (`pip.rs:257–263`) uses the same core action and therefore follows the flag. I consider that correct, since a PiP skip is a step seek. The coordinator should flag it if the user disagrees.

### 6. Implementation plan and acceptance

**Host (`src/host/src`), after the host-fix batch (section 7):**
- `main.rs`:
  - `step_seek: BTreeMap<u64, (u64, f64, bool)>` (`:413–414`) and a `step_keyframes()` next to `step_seconds()` (`:2897–2903`);
  - `player.setStepSeek` reads and validates `keyframes` (`:3577–3587`);
  - `host.run` reads `seconds` (`:3566–3573`);
  - `run_core_action_with` adds `"keyframes": self.step_keyframes()` to the `host.action` params (`:2996`);
  - the revert on stop is already pair-based (`:662–665`);
  - `step_seek_changed` (`:2905–2919`) still compares only seconds, because the menu titles do not depend on the mode.
- `core_actions.rs`: `plan()` gets a `keyframes: bool` argument and the `Plan::KeySeek` variant. Add a pure `KeySeek` state machine (press / settled → next relative amount or done), `key_seek_notice`, and the `"relative+keyframes"` command builder, so that all of the logic is unit-testable without mpv.
- `backend.rs`:
  - in `run_now` (`:867–931`), the keyframe branch (2.4);
  - the `KeySeek` slot next to `seek` / `stepping` (`:841–843`);
  - `poll` (`:1007`) also drives `KeySeek` and may return several completions;
  - the last-GOP guard (2.3).
- `source_backend.rs`, `session.rs`, `pip.rs`: **no change**.

**SDK `C:\vestra\sdk` → 1.5.19:**
- `src/plugin/types.ts`: `setStepSeek` (`:3832–3837`), the `Host.run` overload (`:3905–3921`), and the `seek-backward` / `seek-forward` row of the core-action table (`:3883`).
- `src/plugin/player.ts:584–593`: validate and forward `keyframes`.
- `src/plugin/services.ts:51–57`: validate `seconds` and forward it.
- `src/wire.ts:488`, `497–498`: optional fields.
- `src/plugin/testing.ts` (TestHost):
  - `stepSeekKeyframes = false` beside `stepSeek` (`:1731`);
  - `keyframes: number[] | null` test data;
  - `#runCore` seek (`:2015–2035`) lands on the modeled keyframe (forward ≥, backward ≤, based on the landed position, with an exact intent when no keyframes are modeled or keyframes are off) and pushes the landed-position OSD;
  - `host.run` (`:3328–3348`) accepts `seconds`;
  - `player.setStepSeek` (`:3349–3357`) validates `keyframes`;
  - `coreRuns` records `{ action, seconds? }` only if that stays compatible. Otherwise add a separate `coreSeeks` log, because `coreRuns: string[]` is public.
- `test/core-actions.test.ts`; `docs/concepts/playback.md` (a short "Step seeks: exact or keyframe" section); `docs/reference/commands.md` (`:115`, `:465`); `CHANGELOG.md`; `llms.txt` and `llms-full.txt` regenerated.

**Host-side SDK agreement:** `src/tests/sdk/host-agreement.test.ts` (`:526`, `:553`) maps the same two wire methods, so add a parameter case for `keyframes` and `seconds`.

**Tests (maintainer):**
1. `core_actions.rs` unit tests, with mpv rounding modeled (forward ≥, backward ≤) on keyframes 0/10/20/30:
   - from 12, → 5 gives 20; ← 5 gives 10; from 10, ← 5 gives 0;
   - from 0, → → (5 + 5, coalesced) gives 10 and then 20;
   - tier 1 then tier 2 (5 then 30) from 0 gives 10, then the owed 30 from 10 (target 40), so it lands on 40 or ends;
   - mixed directions net out;
   - backward at 0 sends no seek;
   - the last-GOP guard issues one exact seek;
   - with the flag off, `plan` gives the same output as today for every existing case (`core_actions.rs:775–800`).
2. Validation: `setStepSeek` with non-boolean `keyframes` is invalid; `host.run` with `seconds` and a non-seek action is invalid; `seconds` outside 0.1–3600 or not in tenths is invalid.
3. Scope: `source_backend.rs` `Control::Seek` still builds `absolute+exact`, and no `Control` variant changed (serde round-trip test).
4. SDK: `npm test` in `C:\vestra\sdk`, including the TestHost keyframe model. Then the §6.1 static checks (`cargo fmt --check`, `clippy -D warnings`, `cargo test` with `CARGO_TARGET_DIR=/tmp/vestra-review-target`, `tsc`, the docs check).

**Author:** the plugin moves to SDK `^1.5.19`, sets `minPlayerVersion 0.5.1`, sends `{ seconds, keyframes }` on every activation and on OK, uses `host.run(…, { seconds })` for tiers 2–4 when the box is checked, and adds TestHost tests for acceptance items 5 and 2 (mixed tiers).

**Coordinator GUI check (Windows 11, installed build):**
- Clips:
  - 10 s GOP: `C:\vestra\clips\h264-2160p60-gop10.mp4` and `C:\vestra\clips\hevc-2160p60-10bit-gop10.mp4`;
  - 2 s GOP: `C:\vestra\clips\h264-2160p60.mp4` and `C:\vestra\clips\hevc-2160p60-10bit.mp4`.
- Steps:
  1. Box checked: single → and ←, then Ctrl / Shift / Ctrl+Alt + ← →. Expect no freeze (the 0.6–1.5 s freeze from the measurements is gone), the OSD shows keyframe times (multiples of 10 s or 2 s), and every press moves.
  2. From 00:12 on a gop10 clip, → with 5 s shows "Forward 5 s: 00:20 / …".
  3. Hold → and Ctrl+→ for 5 s: the picture keeps updating, the position only grows, and frames/s do not drop to the 2–4 measured today.
  4. Mixed: → then Ctrl+→ quickly: the moves add up (`seek-*.err` logs).
  5. The last GOP of a gop10 clip: → ends or moves forward and never jumps back (the 2.3 guard).
  6. The seek bar, Jump to Time, a chapter or bookmark, and A-B loop are still exact; D/F frame steps and Previous/Next Keyframe are unchanged.
  7. Box unchecked: today's behaviour, and the OSD shows computed targets again.

### 7. Ordering note

The host change is implemented **on main after the pending host-fix batch merges** (`diag!`, catalog quarantine, mount batching, settings-check thread, built-in disable refusal; uncommitted in worktree `agent-a1c627f0c9f5fdd34`, 19fd7b6 plus edits). Overlap with this plan:
- **`src/host/src/main.rs`: textual conflicts.** The batch changes `eprintln!` → `diag!` at `main.rs:2914`, inside `step_seek_changed`, which sits right next to the new `step_keyframes()` and the `step_seconds()` edit. It also changes `:3072` (the core-action binding loop, next to `run_core_action_with`) and `:1369` (the PiP `Effect::Action` arm). It also adds `Host` struct fields (`:343–356`) and initializers (`:5045`, near `step_seek: BTreeMap::new()` at `:5099`). Overall the batch touches about 230 lines of `main.rs`.
- **`src/host/src/backend.rs`: same file, no overlapping hunks.** The batch changes `:448–647` (`diag!`), and this plan changes `:841–1022`. It still has to be built on top of the batch.
- **`src/host/src/pip.rs`, `source_backend.rs`:** changed by the batch (`diag!`), not by this plan, so no conflict.
- **`src/tests/sdk/`:** the batch changes `runner.test.ts`, and this plan changes `host-agreement.test.ts`, so there is no conflict.
- The SDK 1.5.19 work (`C:\vestra\sdk`, clean at a4bd3ad) and the plugin's preparation do not depend on the batch and can start now. The host 0.5.1 release must contain both.

### Status

- [x] Maintainer reply (§4.3): accept, rewritten (2026-10-26)
- [ ] The author accepts the shape or rebuts once (§4.3 item 3)
- [ ] SDK 1.5.19 (maintainer, can start now)
- [ ] Host 0.5.1 implementation (maintainer, on main after the host-fix batch)


## Author response (playback author, 2026-10-26) — summary

Accepted the ruling without rebuttal, with three confirmations:
- `seek-*` must not become `exclusive` (`core_actions.rs:36–46`), so that presses during a keyframe seek add up instead of being dropped.
- A batch of merged presses reports an error at most once.
- The TestHost records `keyframes` and `seconds` so that plugin tests can assert them.

## Coordinator ruling: picture-in-picture (user, 2026-10-26)

The user asked for the simplest architecture. Picture-in-picture forward/back already goes through the same core seek action as the arrow keys (`run_core_action_with`, `main.rs`), so it follows the same "jump to keyframes only" setting. There is no separate logic and no separate option.

## Record note (2026-10-26)

The playback author's uncommitted plugin-side work and this document's appended sections were deleted when the coordinator removed a worktree that had not been merged (a coordinator error). The maintainer's reply above is restored from its draft. The request and the author response are summarised from the coordinator's notes. The host side and the SDK API were already merged in #46 and were not affected. The plugin side is redone against SDK 1.5.20.

## Coordinator GUI check (Windows 11, 2026-10-26, host 0.5.1 + playback 0.2.1)

On the 3-minute 4K60 H.264 clip with a keyframe every 10 s (`h264-2160p60-gop10-long.mp4`):
- **Landing:** a press at ~6 s goes to 00:20 (target 11 s, next keyframe 20 s). The next press goes to 00:30. The OSD shows the landed time ("Forward 5 s: 00:20 / 03:00").
- **Frames produced per second during single presses every 2 s:**
  - exact (the previous build): 0–5 for a second at almost every press;
  - keyframe: 59–61 throughout.
- **Press to landed:** about 100 ms.
- **Holding the key:**
  - exact: 2–7 frames/s;
  - keyframe: 34, then 20 frames/s, until the hold reached the end of the clip.
- The host version is now **0.5.1**. Without it, playback 0.2.1 (`minPlayerVersion` 0.5.1) is refused at start. The first GUI attempt failed for that reason.
