# 2026-10 round: Auto deinterlace without copy-back on progressive files

Started 2026-10-27. **This document is the only progress record for this round.** Status: the video author found no way to do it with the existing API; a §4.1 request (DEINT-AUTO-1) waits for the maintainer. No plugin or host change yet; `vestra.video` stays at 1.1.5.

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
- The auto deinterlacer (M`filters/f_auto_filters.c`, `deint_process`) decides **per frame** from `MP_IMGFIELD_INTERLACED` and picks by the frame's format: `d3d11vpp` for D3D11 frames (zero-copy `d3d11va`), `vavpp`, `vdpaupp`, `bwdif_cuda`, `bwdif_vulkan`; otherwise software `bwdif` with `mode=send_field` (double rate) behind an autoconvert. VideoToolbox frames have no hardware branch here, so on macOS zero-copy it would try to convert to a software format (to be checked by the maintainer).
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

**Maintainer reply**: (pending)
