# 2026-10 round: Auto deinterlace without copy-back on progressive files

Started 2026-10-27. **This document is the only progress record for this round.** Status: DEINT-AUTO-1 accepted in rewritten form (maintainer reply below); host 0.5.4 implemented on branch `video-deinterlace`, SDK 1.5.24 prepared as a patch, both waiting for the coordinator's release. The plugin change waits for the release; `vestra.video` stays at 1.1.5.

## User ruling (2026-10-27)

The deinterlace default **Auto** must cost nothing on progressive files (almost all modern video): deinterlace only when the source really is interlaced. Interlaced sources must still be deinterlaced as today.

## Problem today

`vestra.video` 1.1.5 (`src/plugins/video/model.ts` `videoNodes`) puts `{ kind: "deinterlace", when: "interlaced", output }` into its chain whenever Deinterlace is not `Never`, which is the default (`Auto`). The host maps it to FFmpeg `bwdif=mode=send_frame:deint=interlaced` (`src/host/src/effects.rs`). Any non-empty owned video chain makes `Processing::frames_in_memory` switch `hwdec` to its copy-back variant (`src/host/src/processing.rs`, `auto-safe` → `auto-copy-safe`), so with default settings **every** file loses zero-copy hardware decoding, although `bwdif` passes progressive frames through untouched. The 1.1.5 CHANGELOG recorded this as an open question.

## What the video author checked (SDK 1.5.23)

1. **Can the plugin know whether the source is interlaced?** No. `PlayerState`, `source.state`, `PlaybackStats.video` (`codec`, `width`, `height`, `fps`, `decoder`, `pixelFormat`, `colorspace`, `displayWidth/Height`, `displayFps`, `bitrate`, `hwdec`, `decoding`, `transfer`) and the media/source types have no field order or interlaced flag. A search of the installed SDK (`dist/*.d.ts`, `llms-full.txt`) for `interlac` finds only the `deinterlace` node itself. The host's `stats()` (`src/host/src/backend.rs` ~811) reads `video-params/*` but nothing about fields.
2. **Can the plugin reach mpv's own `deinterlace=auto`?** No. The only video setters are `VideoView.set(nodes)` (FFmpeg filter nodes) and `RenderOptions.set` (`toneMapping`, `targetPeak`, `peakDetect`, `shaders`, `interpolation`; macOS GPU only). There is no generic mpv property or option setter, by design.

So the plugin cannot do it alone; it needs one of the two host capabilities below.

## mpv facts (libmpv we ship, mpv e471496c56, `C:\vestra\deps\components.json`)

Permalink prefix **M** = `https://github.com/mpv-player/mpv/blob/e471496c56f0f49c133bd88b50aed62b845f1909/`.

- `--deinterlace=<yes|no|auto>` (M`DOCS/man/options.rst#L1818-L1837`): `auto` inserts a deinterlacer only when FFmpeg flags the decoded frame interlaced. Runtime enabling with a hardware decoder "may require setting `--hwdec-extra-frames`" because the surface pool is sized at decoder init.
- The auto deinterlacer (M`filters/f_auto_filters.c`, `deint_process`) decides **per frame** from `MP_IMGFIELD_INTERLACED` and picks by the frame's format: `d3d11vpp` for D3D11 frames (zero-copy `d3d11va`), `vavpp`, `vdpaupp`, `bwdif_cuda`, `bwdif_vulkan`; otherwise software `bwdif` with `mode=send_field` (double rate) behind an autoconvert. VideoToolbox frames have no hardware branch here, so on macOS zero-copy it would try to convert to a software format (to be checked by the maintainer). Once inserted, the deinterlacer stays in the chain (`filter_needed = … || p->sub.filter`, M`filters/f_auto_filters.c#L63-L64`) until the frame format changes or the video chain is rebuilt (next file); progressive frames after it pass through it unchanged (`bwdif deint=1`, `d3d11vpp interlaced-only=yes`). `deinterlace-active` reports the current frame's flag (`mp_deint_active`, M`filters/f_auto_filters.c#L203-L208`), not whether the filter is still inserted. (Maintainer correction in review, 2026-10-27.)
- It runs as the `userdeint` **pre-filter**, before the user `vf` chain (M`filters/f_output_chain.c`, `create_video_things`), which matches the plugin's fixed order (deinterlace first).
- Property `deinterlace-active` (M`DOCS/man/input.rst#L2604-L2608`): whether mpv's own deinterlacer is active ("only a guess with `auto`").
- Property `video-frame-info/interlaced` and `/tff` (M`DOCS/man/input.rst#L3068-L3072`): per frame, from the decoder, available without any filter.

## DEINT-AUTO-1: Auto deinterlace that does not force copy-back (§4.1 request, video author, 2026-10-27)

**Requesting plugin / code location**: `vestra.video`, `src/plugins/video/model.ts` `videoNodes` (the deinterlace node), `main.ts` (`setDeinterlace`, the chain install).

**User action and expected result**: the user opens any file with default settings (Deinterlace ▸ Auto). Progressive file: no deinterlacer and no owned chain, so decoding stays zero-copy (`hwdec-current` = `d3d11va` on Windows GPU rendering, `videotoolbox` on macOS, not `*-copy`). Interlaced file (or interlaced parts of a file): deinterlaced as today. `Always` and `Never`, and the By Frame / By Field output, behave as today.

**Why the existing API cannot do it**: see "What the video author checked" above. Minimal reproduction: default settings, open any progressive H.264 file on Windows with GPU rendering; `stats().video.hwdec` is `d3d11va-copy`; choose Deinterlace ▸ Never and it becomes `d3d11va`. The plugin has nothing to decide on, because no API tells it whether the source is interlaced.

**Desired usage**: either of these; the author prefers **(B)**.

(A) *Expose the flag* (addition): `stats().video.interlaced: boolean | null` (and ideally a `PlayerState` change event when it first becomes known for a source), from mpv `video-frame-info/interlaced`. The plugin installs the node only while it is `true`:

```ts
const interlaced = (await player.stats()).video?.interlaced === true;
nodes = look.deinterlace.mode === "on" || (look.deinterlace.mode === "auto" && interlaced) ? [deinterlaceNode, ...rest] : rest;
```

Drawbacks: the flag is known only after the first decoded frames, so on every interlaced file the plugin installs a node after playback started, which switches `hwdec` mid-stream (decoder reopen and the `HwdecWatch` retry path, a visible hitch at the start), and the first frames show combed. It is per frame in mpv, so mixed files (broadcast captures that switch between progressive and interlaced) need polling or an event per change. The plugin would also have to re-evaluate on every source change and treat `null` (not known yet).

(B) *A deinterlace node the player maps to mpv's own deinterlacer* (addition, new value of an existing field): `{ kind: "deinterlace", when: "auto", output }`, documented as "only frames flagged interlaced, decided per frame by the player; uses the decoder's hardware deinterlacer where there is one; does not by itself need frames in memory". The host maps it to mpv `deinterlace=auto` instead of a `bwdif` filter, and does **not** count it for `frames_in_memory`: a chain whose only node is `when: "auto"` keeps zero-copy decoding. When other nodes force copy-back anyway, mpv's auto deinterlacer falls back to software `bwdif` on the copied frames, still before the plugin's filters. The plugin then changes only one line (`when: deinterlace.mode === "on" ? "all" : "auto"`, with `minPlayerVersion` raised to the player that has it):

```ts
nodes.push({ kind: "deinterlace", when: mode === "on" ? "all" : "auto", output: deinterlace.output });
```

Open points for the maintainer under (B):
- **Output**: mpv's software fallback is always `send_field` (double rate) and the hardware filters have their own default rate; `output: "frame"` may not be honourable with `when: "auto"`. Acceptable answers, in the author's order of preference: honour it where mpv allows (e.g. `d3d11vpp` options), otherwise document that `when: "auto"` outputs at the rate the player chooses; or accept `when: "auto"` only with `output: "field"` and refuse `frame` with `invalid` (the plugin would then map Auto + By Frame to the existing `bwdif` node and accept copy-back for that combination).
- **Single owner**: `deinterlace` is one global mpv option. With several plugins, two `when: "auto"` nodes map to the same option; the host should treat it like `frameBlend` (at most one, or "any owner wants it → `auto`"), and clear it when the owner's chain is removed or the plugin is deactivated.
- **`hwdec-extra-frames`**: set at startup (before the first file) so runtime insertion of the hardware deinterlacer does not run out of surfaces.
- **Snapshots and exports**: a node that is an mpv option, not an FFmpeg filter, may not reach the export pipeline; state whether exports deinterlace with `when: "auto"` (e.g. map it to `bwdif=deint=interlaced` there).
- **macOS**: VideoToolbox has no branch in `f_auto_filters.c`; check what mpv does with `deinterlace=auto` on zero-copy VideoToolbox frames that are interlaced (autoconvert to software or fail), and whether the host must use copy-back there once a frame is interlaced (`deinterlace-active` tells it).

**Who owns state and resources; failure, cancellation, missing capability**: the plugin owns its setting and its chain, as today. Under (B) the host owns the mpv `deinterlace` option and resets it to `no` when no owner wants it (chain removed, plugin deactivated, plugin crashed). Failure: the node fails with the usual `FilterStatus` `failed`. Missing capability on an older player: `when: "auto"` is refused with `invalid`; the built-in plugin ships with the player, so `minPlayerVersion` covers it. Under (A), an older player has no `interlaced` field and the plugin would keep today's node.

**Whether it breaks a contract**: no. (A) adds a field; (B) adds a value to `when` and does not change what `interlaced` / `all` do. Only `vestra.video` uses the deinterlace node today. The default picture of interlaced files may change slightly under (B) (hardware deinterlacer instead of `bwdif`, possibly double rate); that is within the user's ruling ("still deinterlaced") but the coordinator should look at it.

**Minimal acceptance**:
- Host unit tests: a video chain with only `{ kind: "deinterlace", when: "auto" }` sets mpv `deinterlace=auto`, installs no `vf` entry and does not switch `hwdec` to copy-back; adding a colour node switches to copy-back and keeps `deinterlace=auto`; removing the node or deactivating the owner sets `deinterlace=no` and restores `hwdec`; `when: "interlaced"` / `"all"` unchanged.
- SDK: `VideoNode` type and docs, TestHost accepts and records the value (validation as the player).
- Plugin (after release): TestHost tests that Auto installs `when: "auto"`, Always `when: "all"`, Never nothing; version 1.1.6.
- GUI (coordinator, Windows GPU rendering, default settings): progressive file → `hwdec-current` = `d3d11va` (no `-copy`), `deinterlace-active` false; interlaced test clip (below) → no combing on motion, `deinterlace-active` true. Same on macOS with `videotoolbox`.

Interlaced test clip (ffmpeg, 25i, top field first, flagged interlaced in the H.264 stream):

```powershell
ffmpeg -f lavfi -i "testsrc2=size=1920x1080:rate=50,interlace=scan=tff" -t 20 -c:v libx264 -flags +ildct+ilme -x264-params tff=1 -pix_fmt yuv420p interlaced-tff.mp4
```

`ffprobe -v error -select_streams v -show_entries stream=field_order interlaced-tff.mp4` must print `field_order=tt`.

**Maintainer reply** (API maintainer, 2026-10-27; §4.2 / §4.3):

Permalink prefix **M** as above (mpv e471496c56).

*Classification*: a capability gap. Not a defect: `frames_in_memory` keeps its contract (every owned node is an FFmpeg software filter, and `bwdif` is one), and `when: "interlaced"` does what it says. Not a docs problem. Not breaking: a new value of an existing field, refused by older players.

*Existing ways considered first*, with their cost:

1. **Today's node** (`when: "interlaced"`): correct picture, but copy-back on every file. Fails the user ruling.
2. **The plugin decides by itself** (read the field order from the file in Node, or spawn `ffprobe`): plugins do not run OS tools (§1.5) and the player ships no `ffprobe` to them; parsing field flags of MPEG-2 / H.264 / HEVC in every container inside one plugin reinvents the decoder; it cannot follow files that switch between progressive and interlaced; and on an interlaced file the node still arrives after playback started, with a mid-stream `hwdec` switch. Rejected.
3. **Reduce the requirement** (default Never, Auto only on request): contradicts the user ruling.
4. **(A) `stats().video.interlaced`**: an addition, but strictly weaker than (B) for more API surface (a field plus a change event): the plugin acts only after frames were decoded, so every interlaced file starts combed and switches `hwdec` mid-stream (decoder reopen, `HwdecWatch` path), and mixed files need polling. Not accepted.
5. **`RenderOptions`**: render stage, macOS GPU only; deinterlacing belongs before scaling, at the decode stage. Not applicable.

The capability needs mpv's `deinterlace` option and the decoder's own hardware deinterlacer, which only the host has (§4.3 item 4, first bullet), and any processing plugin can use it. So the core changes.

*Ruling*: **accept (B), rewritten (narrower)**. Host 0.5.4, SDK 1.5.24 ("Who must change: nobody").

- New value `when: "auto"` of the `deinterlace` node, **only with `output: "field"`**: `{ kind: "deinterlace", when: "auto", output: "field" }`. Contract: "the player's own deinterlacer, decided per frame from the decoder's interlaced flag; where frames stay on the GPU it uses the decoder's hardware deinterlacer; one output frame per field (double rate) on interlaced frames, progressive frames untouched; by itself it does not need frames in memory, so with no other node decoding stays zero-copy. Runs before every plugin's chain and before the player's own orientation, wherever it is written (as `frameBlend` always runs last)."
- `when: "auto"` with `output: "frame"` → `invalid`. A chain (one resource) with a `when: "auto"` node and any other `deinterlace` node → `invalid`. Older players refuse the value with `invalid` (`when` is a closed enum there).
- The host maps it to mpv `deinterlace=auto` (no `vf` entry), does not count it in `frames_in_memory`, and owns the option (below).

*The author's open points*:

1. **Output (frame vs field)**: every branch of mpv's auto deinterlacer outputs one frame per field, and none can be told otherwise through the `deinterlace` option: the software fallback is hard-coded `bwdif mode=send_field` (M`filters/f_auto_filters.c#L155-L160`), `bwdif_cuda` / `bwdif_vulkan` likewise (`#L111-L122`), `d3d11vpp` is created with `deint=yes:interlaced-only=yes` and its default processor mode outputs fields (`out_fields` is false only for `blend` / IVTC, M`video/filter/vf_d3d11vpp.c#L786-L796`); there is no VO deinterlacer in this mpv. Accepting `frame` and delivering double rate would break what `output` promises, so `frame` is refused rather than silently reinterpreted (the author's second option). **Consequence for the plugin's default** (Auto + By Frame, `model.ts` `defaultLook`): the user ruling can be met only at field rate, because no zero-copy path deinterlaces at frame rate. Recommended mapping: Auto → `when: "auto", output: "field"` whatever the Output setting, with Output applying to Always (say so in the menu or the README); keeping By Frame honoured under Auto means the old `bwdif` node and copy-back on every file again. Interlaced files then play at double rate by default (25i → 50 fps, as mpv and most players do); **the coordinator should look at this** in the GUI test.
2. **Single owner of mpv's global `deinterlace`**: not like `frameBlend`. Two `frameBlend` nodes would convert the rate twice, so at most one is enforced; two `when: "auto"` nodes ask for the same idempotent thing, and refusing plugin B because plugin A holds one would make B's chain depend on another plugin (§1 plugin independence). So: `deinterlace=auto` while at least one owned video chain of any plugin holds a `when: "auto"` node, `no` otherwise; recomputed on every video chain change and on owner deactivation (`Processing::disable`, also used for a crash). The host is the only writer of the option (libmpv has no default key bindings and the host sets it nowhere else). Explicit nodes in other plugins' chains are not refused (that would couple plugins again) but documented: mpv's deinterlacer runs first, an `interlaced` node then sees unflagged frames and passes them through, an `all` node deinterlaces a second time, as two `bwdif` nodes do today.
3. **`hwdec-extra-frames`**: not set. The surface pool is sized at decoder init with mpv's `auto` sizing, at least 6 extra surfaces on top of the codec's (`HWDEC_EXTRA_FRAMES`, M`video/decode/vd_lavc.c#L73` and `#L956-L972`); `d3d11vpp` is created with no reference frames (`mp_refqueue_set_refs(p->queue, 0, 0)`, M`video/filter/vf_d3d11vpp.c#L792`); when its video processor is created on the first frame it holds as many past and future frames as the driver's rate-conversion caps ask for (`PastFrames` / `FutureFrames`, none for `bob` / `blend`; `#L356-L379`; numbers not measured here), and it writes into its own output pool. (Corrected in review, 2026-10-27: the first version gave "typically 1–2 and 0–1" without a source.) A fixed higher value would cost GPU memory on every file, against the ruling. The GUI acceptance on the interlaced clip watches the log for surface allocation errors or drops; if they appear, setting a fixed value is a host defect fix with no API change.
4. **Exports and processed snapshots**: they run the frozen chain in offline FFmpeg (`offline_descriptor.rs`), where mpv's option does not exist. There `when: "auto"` renders as `bwdif=mode=send_field:deint=interlaced`, placed first in the graph: the same per-frame decision and the same field rate as the live picture. Raw snapshots stay unprocessed. **Known limitation** (from review): offline FFmpeg applies the container rotation (autorotate) while decoding, before the graph, whereas live mpv deinterlaces before `@vestra-orientation`; so an export or processed snapshot of a rotated (90° / 270°) interlaced source deinterlaces rotated fields and can differ from the live picture. The explicit modes (`interlaced`, `all`) already behave like this offline (and live they run after the orientation too); rotated interlaced sources are rare, so it is documented, not changed.
5. **macOS VideoToolbox**: `f_auto_filters.c` has no VideoToolbox branch, so mpv takes the generic path: an autoconvert restricted to software formats, which downloads hardware frames (`HW-downloading from …`, M`filters/f_autoconvert.c#L202-L280`), then software `bwdif send_field` (M`filters/f_auto_filters.c#L131-L161`). No decoder switch and no failure. Progressive files stay zero-copy. Once the first interlaced frame inserted the deinterlacer, it stays until the frame format changes or the next file (see the mpv facts above), so from then on **every** frame of that file is downloaded, progressive ones included (bwdif passes those through): a mixed source loses zero-copy after its first interlaced frame. (Corrected in review, 2026-10-27: the first version said only interlaced frames are copied.) The cost on interlaced files is about today's copy-back plus `bwdif`. The host does not switch to copy-back on `deinterlace-active`. **Not measured on a Mac** (none here): added to the Mac test list below.

*Mac test list* (coordinator, macOS GPU rendering, default settings): progressive file → `hwdec-current` `videotoolbox`; interlaced clip → no combing, `hwdec-current` still `videotoolbox`, the log has `HW-downloading from videotoolbox` and no `no deinterlace filter available`; a mixed source (progressive then interlaced, `doc/process/mac-verification.md`) → zero-copy until the first interlaced frame, then downloaded frames (expected) with no stall or error at the switch.

*Windows today*: the player on `main` renders in software on Windows (`platform::GPU_RENDER` is false there), so decoding is copy-back (`d3d11va-copy`) with or without a chain, and the author's `d3d11va` / `d3d11va-copy` reproduction applies to a GPU-rendering build. On software rendering `when: "auto"` still removes the cost on progressive files (no `bwdif` in the graph; mpv's deinterlacer is inserted only on interlaced frames) and deinterlaces interlaced ones with `bwdif send_field`; the `d3d11vpp` branch applies once Windows renders on the GPU with zero-copy `d3d11va`.

### Implementation (API maintainer, 2026-10-27)

Host 0.5.4 (`src/host/`; no plugin, runner or SDK-test file touched):

- `effects.rs`: `DeinterlaceWhen::Auto`; `auto` + `frame` and a second `deinterlace` node in a resource with `auto` are `invalid` (`Node::validate_chain`); `Node::first` / `Node::by_player`: the player's graph (`render_player`, `runtime_commands`) leaves such nodes out, the offline graph (`render_lavfi`: exports, processed snapshots) renders them first as `bwdif=mode=send_field:deint=interlaced`; `player_deinterlace` and `needs_frames_in_memory` over the owned resources.
- `processing.rs`: `set_deinterlace` owns mpv's global `deinterlace` (`auto` while any owner's chain has an `auto` node, `no` otherwise), set before the graph and restored if the graph change fails; recomputed in `disable` (deactivation, crash); `frames_in_memory` follows `needs_frames_in_memory` instead of "any chain" in `replace`, `disable` and `replay_ready`; `set_graph` / `apply` use `render_player`.
- `tests/fixtures/processing-conformance.json`: a new case for `when: "auto"` (shared with the SDK's TestHost through `src/tests/sdk/host-agreement.test.ts`, which therefore needs SDK 1.5.24 installed).
- Tests: `effects::auto_deinterlace_is_the_players_and_bwdif_first_offline` (player vs offline graph, live update indices, rebuild on mode change, refusals, several owners); `processing::auto_deinterlace_keeps_zero_copy_decoding` (only `auto` → `deinterlace=auto`, `hwdec` stays `auto-safe`, no `bwdif` in `vf`; plus a gblur → `auto-copy-safe`; back → `auto-safe`; explicit `interlaced` → `bwdif`, `deinterlace=no`, copy-back; `[]` → `no`; refusals change nothing); `processing::auto_deinterlace_is_shared_and_revoked_with_its_owners` (two owners, disabling one keeps `auto`, the last → `no`; disabling an owner with `auto` + gblur restores `deinterlace=no`, `auto-safe` and the graph); ignored real-decoding test `processing::auto_deinterlace_on_real_decoding` (software rendering, ffmpeg-made 25i TFF H.264 clip and a 25p clip).
- Docs: `doc/architecture/README.md` §4.1.

SDK 1.5.24, prepared as a patch against SDK `main` (e45c691, 1.5.23) for the coordinator: `VideoNode` `when: "auto"` with its contract, `nodeProblem` (the TestHost's validation) with the same two refusals, a test in `test/round4a.test.ts`, *Processing* concept page, CHANGELOG ("Who must change: nobody"), version 1.5.24, `llms.txt` / `llms-full.txt` regenerated.

Checks run (Windows, 2026-10-27):

- Host: `cargo fmt --check` 0; `cargo clippy --locked --all-targets -- -D warnings` 0; `cargo test --locked` 470 passed, 0 failed, 8 ignored; `cargo test --locked -- --ignored auto_deinterlace_on_` 1 passed (this machine's `d3d11va-copy`: on the interlaced clip `video-frame-info/interlaced` true, `deinterlace-active` true, `estimated-vf-fps` about 50, no `bwdif` in `vf`, no "no deinterlace filter available" / "creating deinterlacer failed" / filter-failed log line; on the progressive clip `deinterlace-active` false, about 25 fps); `npm run unsafe-policy` ok; `typos` on the changed files clean (the whole tree reports one pre-existing hit from `main`, in `doc/process/rounds/2026-10-shared-runner.md` line 353).
- Player `src/`: `npm run check`, `format:check`, `boundaries` 0. `npm run test:sdk` with SDK 1.5.24 installed temporarily (`npm install --no-save` of the packed patch, then `npm ci` back to the lock): 34/34 pass; with the locked 1.5.23 the new conformance case fails (expected: 1.5.23's TestHost refuses `auto`), so the SDK upgrade goes in the same commit. Plugin tests and `portability` not run (no plugin or platform code changed).
- SDK patch in a temporary clone: `npm ci`, `check`, `format:check`, `lint`, `docs:check` (0 failing in Markdown and enforced categories; the 61 "other" `@example` warnings are unchanged from 1.5.23), `npm run docs` (TypeDoc, exit 0, no warnings), `docs:llms -- --check`, `test` (192 tests, 191 pass, 0 fail, 1 skipped), `npm pack --dry-run` all exit 0; the patch applies to `C:\vestra\sdk` `main` (`git apply --check`).
- Not run: GUI (coordinator), zero-copy `d3d11va` + `d3d11vpp` (no Windows GPU rendering on `main`), macOS (Mac test list above).

### Review fixes (API maintainer, 2026-10-27, after the independent review of b438f1d)

1. `Processing::disable` now switches mpv's `deinterlace` before the graph change, as `replace` does (`replace` through `deinterlace_then`, `disable` inline): when paused, the graph change redraws the picture and must already see the new value. Disabling does not put the deinterlacer back on a failed graph change (ownership is revoked regardless). The ignored real-decoding test `auto_deinterlace_on_real_decoding` gained the paused case (pause on the interlaced clip with `auto`, disable the owner, the redrawn frame differs from the deinterlaced one). It passed on this machine with the old order as well: mpv applies the `vf` change's redraw on its next playloop iteration, after the option was already set, so the race the review describes was not reproducible here; the order is fixed anyway.
2. Wording corrected (mpv facts, reply items 3 and 5, `doc/architecture/README.md` §4.1): mpv keeps its deinterlacer once inserted until a format change or the next file, so on macOS a mixed source is downloaded from its first interlaced frame on; `d3d11vpp` starts with no references (`#L792`) and takes the driver's `PastFrames` / `FutureFrames` when its processor is created (`#L356-L379`). One point differs from the review: `deinterlace-active` is `mp_deint_active`, which reads the current frame's flag (`f_auto_filters.c#L203-L208`), so it does not stay true after the filter was inserted. Mixed-source and paused cases added to `doc/process/mac-verification.md` (the mixed recipe checked with `ffprobe`: 50 frames flagged 0, then 50 flagged 1).
3. Rotated interlaced sources in exports and processed snapshots: documented as a known limitation (reply item 4, architecture doc).
4. New unit tests: `a_failed_graph_change_restores_the_deinterlacer` (`deinterlace_then`: the graph change runs with the new value, a failure restores the previous one, both directions) and `replaying_an_auto_only_chain_keeps_zero_copy_decoding` (`replay_ready` with an `auto`-only chain keeps `auto-safe`, with a gblur added it switches to `auto-copy-safe`).

Checks: `cargo fmt --check` 0; `cargo clippy --locked --all-targets -- -D warnings` 0; `cargo test --locked` 472 passed, 0 failed, 8 ignored; `cargo test --locked -- --ignored auto_deinterlace_on_` 1 passed; `typos` on the changed files clean (the tree still has the one pre-existing hit from `main` in `doc/process/rounds/2026-10-shared-runner.md` line 353). The SDK patch is unchanged.

### For the video author: acceptance list (after host 0.5.4 / SDK 1.5.24 are released)

1. `manifest.json` `"minPlayerVersion": "0.5.4"`; SDK dependency `^1.5.24`.
2. `model.ts` `videoNodes`: Auto → `{ kind: "deinterlace", when: "auto", output: "field" }`; Always → `when: "all"` with the Output setting as today; Never → no node. Auto + By Frame cannot be `when: "auto"` (`invalid`): either Auto ignores Output (recommended; the only mapping that meets the user ruling with the default By Frame), or Auto + By Frame keeps today's `when: "interlaced", output: "frame"` with copy-back on every file. Say in the menu or the README which one applies.
3. Never put a second `deinterlace` node in the same chain as the `auto` one (`invalid`).
4. Keep the node first in the chain for readability; the player runs it first anyway.
5. TestHost tests: Auto installs `when: "auto"` (and, with no other look setting changed, nothing else), Always `when: "all"`, Never nothing; a chain with `auto` and colour nodes is applied.
6. Version 1.1.6, CHANGELOG: interlaced files under Auto now play at field rate (25i → 50 fps); progressive files no longer lose zero-copy decoding.
7. GUI acceptance as in the request (coordinator), plus the Mac test list above.

*Author's acceptance*: (accepted by the video author (2026-10-27, video 1.1.6).
