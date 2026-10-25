# Features round 1: core interface negotiation record

Basis: [Community workflow](../community-workflow.md) §4.2, §4.3; source of the requests: [Feature requirements](../../product/feature-requirements.md) Appendix A (below, "the requirements document"); for the round, see [2026-10-features-r1.md](2026-10-features-r1.md).

This document records three steps: step 1, the maintainer's reply (this section); step 2, rebuttals from the author representatives (at most one per item, evidence required); step 3, the maintainer's ruling and schedule. Step 1 is only an **initial assessment**, not an approval, and nothing has been implemented.

Reading conventions:

- **The four conditions of §4.3-4** are abbreviated below as: [host] needs host-internal state or a capability only the host has; [shared] the capability itself must be shared across plugins; [core duty] it already belongs to the core; [general] a basic capability that several plugins need, or that many plugins will use later. Items that meet none of the four are proposed for rejection.
- **Core size impact** is estimated by order of magnitude: "tiny" = passing through one mpv property or one validation range (a few dozen lines at most); "small" = one new method plus a state field (about 100–300 lines, including SDK, TestHost, docs); "medium" = a new state machine, a new mount slot or new persistence (about 300–1000 lines); "large" = changes to the rendering or window architecture (over a thousand lines, with regression risk).
- Code examples use only the existing API ([API reference](../../plugins/reference.md)), unless marked "rewritten form".
- The current state of the host is taken from `src/host/src` (this branch); facts used are cited in the "Reason" of each item.

---

## Step 1: Maintainer reply

### Player

### R-P1 `player.stepFrame` frame stepping

- **Requested by**: playback B07; core basic actions `frame-back` / `frame-forward` (H1)
- **Initial assessment**: proposed rewrite
- **Alternative** (no API change): use `seek` for a relative jump of 1/fps:
  ```ts
  const { video } = await plugin.player.stats();
  const fps = video?.fps ?? 25;
  const pos = plugin.player.state.position ?? 0;
  await plugin.player.setPaused(true);
  await plugin.player.seek(pos + 1 / fps);   // exact seek
  ```
  Cost: VFR sources and files with irregular timestamps skip or repeat frames; each exact seek decodes again from the keyframe, so the forward direction is an order of magnitude slower than mpv `frame-step`, with visible stutter while F is held. Reliable "frame stepping" at the user level is not possible this way, so this item cannot be rejected.
- **Suggested rewrite**:
  1. Merge with P2 into one method: `player.step(unit: "frame" | "keyframe", direction: "backward" | "forward", opts?: { ifCurrent? }): Promise<void>`.
  2. Drop the two state fields `can.frameStep` / `can.frameBackStep`. Plugins derive availability from existing information: `stats().video !== null` means there is video; stepping backward also needs `state.can.seek`. When the condition is not met, the core fails with `unsupported` and does not return a false success. This way PlayerState does not gain a field for one feature, and each state push does not need to read extra mpv properties for it.
  3. "Drop repeated key presses while a step is in flight" stays with the caller (plugin or core action): the method itself performs only one step.
- **Reason**: [host] `frame-step` / `frame-back-step` are mpv commands; [core duty] the H1 basic actions use them.
- **Core size impact**: tiny to small. One RPC mapped to two mpv commands, plus the `unsupported` check; no new state.

### R-P2 `player.stepKeyframe` keyframe jump

- **Requested by**: playback B08
- **Initial assessment**: proposed rewrite (merged into R-P1)
- **Alternative**: for local files, the plugin can use ffprobe to read only the keyframes near the current position, then seek exactly:
  ```ts
  // read video keyframe times within [pos-30, pos+30] seconds, no shell, with timeout
  execFile(ffprobe, ["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey",
    "-read_intervals", `${pos - 30}%${pos + 30}`, "-show_entries", "frame=pts_time",
    "-of", "csv=p=0", path], { timeout: 3000, signal });
  // take the first one strictly greater than pos → plugin.player.seek(t)
  ```
  Cost: not possible for network sources and emulator streams; each key press starts an ffprobe process (about 50–200 ms), which cannot keep up with key repeat; with very long GOPs the ±30 s window can miss, and the window must be widened and retried.
- **Suggested rewrite**: merge into R-P1's `player.step("keyframe", direction)`; the core maps it to mpv `seek ±0 relative+keyframes` plus direction. One method, one set of error semantics, no separate API.
- **Reason**: [host] mpv knows the demuxer's keyframe index; a plugin can only start another process and scan again; the alternative does not work for network sources. But it is only another unit of P1 and does not justify a separate method.
- **Core size impact**: tiny (one branch of P1).

### R-P3 `player.setStepSeek` level 1 step size

- **Requested by**: playback B06; basic action handover (H1)
- **Initial assessment**: proposed accept (done together with H1)
- **Alternatives (considered)**:
  - Let playback not use the core's ←/→, and register its own level 1 command and take over the binding: this conflicts with the H1 goal "←/→ still work after playback is uninstalled", and two commands competing for one key would have to be resolved by load order.
  - Lower the requirement: level 1 fixed at 5 seconds, B06 exposes only levels 2–4. The cost is that level 1 is the one users adjust most.
- **Reason**: [core duty] after H1 hands ←/→ to the core, the step size is a parameter of a core action; only the core can change it. The setting is still saved by playback; the core does not persist it.
- **Constraints**: "restore 5 seconds after the calling plugin is deactivated" follows the filter undo rule (each effect is recorded under its caller); with several calling plugins, the last write wins.
- **Core size impact**: tiny. One numeric state and an undo hook.

### R-P4 `plugin.host.run(action)` run a core action

- **Requested by**: playback bottom bar buttons (basic action handover)
- **Initial assessment**: proposed accept (with H1)
- **Alternative (considered)**: the bottom bar buttons call `setPaused`, `seek`, `setOutput` and so on directly and show their own OSD. Cost: buttons and shortcuts become two implementations, and the accumulation rule (repeated presses → accumulate from the latest target), clamping and hint text slowly diverge, which is exactly what interaction requirements §1 "shared state and behavior" is meant to avoid.
- **Constraints**:
  - `action` can only be an action id registered by the core, not an arbitrary command id. A plugin cannot use it to run another plugin's command (that would be inter-plugin communication).
  - When the action is disabled, it fails with `unavailable` and is not queued.
  - No new capability name; `commands` is reused.
- **Reason**: [core duty] the core is the party that is always present; running core actions does not depend on any plugin.
- **Core size impact**: tiny (H1 already has the action table; this is only an RPC that dispatches by id).

### R-P5 `player.preload` gapless playback

- **Requested by**: playlist B26
- **Initial assessment**: proposed reject (not this round; can be raised again with measurement evidence)
- **Alternatives**:
  1. Keep the current `media.onEnd → player.open(next, { ifCurrent: ended })`. playlist can shorten the advance path as much as possible: compute the next item in advance, run `fs.stat` on local files in advance, and do only one `open` in the `onEnd` callback.
  2. Lower the requirement: B26 becomes "fast advance" with no gapless promise; for album-type content, users can merge the tracks into a single file (outside the scope of this plugin).
- **Cost**: at the switch there is a short silence and one black frame. There is currently **no measurement**: it could be 100 ms or 500 ms.
- **Evidence needed for a rebuttal**: measure the switch gap between two adjacent flac files on the current host (recorded waveform), and give a standard for what users can perceive. If the gap really exceeds about 150 ms, a core solution will be discussed.
- **Reason**: only one plugin uses it, and of the four conditions it only touches [host]. But its cost is especially high: it would bring the mpv internal playlist (`loadfile … append`, `prefetch-playlist`, `gapless-audio`) into the current "one source at a time" session state machine (`session.rs`, `source_backend.rs`), and would need an fd lease for the next local file before the current source ends (`source_lease.rs`); it also changes the ordering contract "old source ended → new sourceId", which affects every plugin that uses `ifCurrent`. This is what §4.2 item 3 means by "changing the lifecycle is also a compatibility break". Evidence first.
- **Core size impact**: medium to large. A new state machine branch, new resource lease timing, a cross-plugin contract change.

### R-P6 `PlayerState.videoSize` display size

- **Requested by**: window H03, H04, H07
- **Initial assessment**: proposed rewrite (merged into R-P7, not in PlayerState)
- **Alternative**: the window plugin queries stats once **when the command runs**; no state push is needed:
  ```ts
  run: async () => {
    const { video } = await plugin.player.stats();
    if (!video?.width || !video.height) return plugin.osd.show("No video");
    await plugin.window.setVideoArea({ width: video.width * k, height: video.height * k }); // W2
  }
  ```
  The existing `video.width/height` is the decoded size (`video-params/w/h`) without DAR, so 720×576 (16:9) comes out wrong.
- **Suggested rewrite**: add `displayWidth` and `displayHeight` to `PlaybackStats.video` (mpv `video-out-params/dw`, `dh`, the size after DAR and the host's orientation handling), which are the fields of the same name already listed in P7. The command's `enabled` uses `player.state.phase`, and "No video" is shown at run time (a slightly lowered requirement: the menu does not grey out the audio-only case in advance; or the window plugin queries `stats()` once at ready after `sourceId` changes and caches it for `enabled`).
- **Reason**: the display size is [host] information, but PlayerState is rebuilt on every push (`source_backend.rs::snapshot` reads mpv properties every time), so adding data that is only used when a command runs is not worth it. An on-demand query is enough.
- **Core size impact**: tiny (two fields of P7).

### R-P7 `PlaybackStats` extended fields

- **Requested by**: info I05; window H07 acceptance refers to `avSyncMs`; merges P6, P8
- **Initial assessment**: proposed rewrite (accepted, fields narrowed)
- **Alternative (considered)**: ffprobe can only read files, not the session that is being decoded: whether hardware decoding is active, the current frame rate, A/V sync and buffered duration are all unavailable. Network sources even less so. There is no in-plugin way for this part.
- **Suggested rewrite**:
  - The fields only pass through raw mpv values, all may be `null`, with no further processing: `video.decoder` (`video-codec`), `pixelFormat`, `colorspace` (`video-params/colormatrix`), `displayWidth/displayHeight` (P6), `displayFps`, `bitrate`, `transfer` (P8, the `video-params/gamma` string as is, not mapped to an enum); `audio.decoder`, `format`, `channelLayout`, `bitrate`; `avSyncMs`, `cacheSeconds`.
  - `hwdec` is not added for now: the host fixes `hwdec=no` (`backend.rs`), so this field would always be `null`; it will be added with the hardware decoding path (after C-GPU).
  - Still an on-demand query (`stats()`), not pushed.
- **Reason**: [host] all of it comes from mpv properties; `backend.rs::stats` is already a pass-through function, and extending it is a few more property reads.
- **Core size impact**: tiny.

### R-P8 `PlaybackStats.video.transfer`

- **Requested by**: video E16
- **Initial assessment**: proposed rewrite (merged into R-P7)
- **Alternative**: for local files, the plugin can read `smpte2084` / `arib-std-b67` with `ffprobe -show_entries stream=color_transfer`; not possible for network sources.
- **Suggested rewrite**: as P7's `video.transfer: string | null`, giving mpv `video-params/gamma` as is (for example `"pq"`, `"hlg"`, `"bt.1886"`); the plugin decides whether it is HDR. It only matters for E16, and E16 depends on C-GPU, so it is scheduled for round 4 and released together with V10.
- **Reason**: [host]; but it does not justify a separate enum type.
- **Core size impact**: tiny.

### Tracks and subtitles

### R-T1 `player.addAudioTrack` load an external audio track

- **Requested by**: audio A11
- **Initial assessment**: proposed accept
- **Alternative (considered)**: the plugin uses ffmpeg to mux the video and the external audio into a temporary mkv and then calls `open`: this remuxes the whole file (tens of seconds to minutes for large files), replaces the current source (`sourceId` changes, the position must be restored), and resets the state of all other plugins. "Add an audio track to the current video" is not possible at the user level.
- **Constraints**: by default conditioned on the current `player.state`; the external track goes away with the source; `AudioTrack.external = true`, `exportable = false` (existing rule); local paths also go through the host's path validation.
- **Reason**: [host] only the host can call mpv `audio-add`.
- **Core size impact**: small. One command, plus the `audioTracks` state refresh (already exists).

### R-T2 `player.selectTrack("audio", id)` select audio track

- **Requested by**: audio G03, G04
- **Initial assessment**: proposed rewrite (narrower form)
- **Alternative (considered)**: none. Only the host can write `aid`; plugins can see `audioTracks` but cannot change it.
- **Suggested rewrite**: drop the `kind` parameter reserved for "future extension" (E15 video streams was removed, so audio is now the only use), and write it as `player.selectAudioTrack(id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>`. If selecting video streams is really needed later, add a separate method then; do not reserve parameters for hypothetical needs.
- **Reason**: [host] mpv `aid`. In addition, the later "stream selection" button of playback will also use it, which meets [general].
- **Core size impact**: tiny.

### R-T3 `subtitles.selectSecondary` secondary subtitles

- **Requested by**: subtitles A08
- **Initial assessment**: proposed rewrite
- **Alternatives**:
  - The plugin merges two tracks into one ASS (primary at the bottom, secondary placed at the top with `\an8`) and loads it as a whole with `load`: this only works for tracks whose text the plugin can get; embedded tracks must be extracted first (see the alternative in R-T5), the plugin must compose styling and sync itself, and when the primary subtitle track changes it must be composed and uploaded again. The cost is high and the behavior differs from normal primary subtitles.
  - Lower the requirement: secondary subtitles support external files only.
  This item really needs `secondary-sid`.
- **Suggested rewrite**:
  - No new method; extend the existing `select`: `subtitles.select(track | null, opts?: { role?: "primary" | "secondary"; ifCurrent? })`, default `"primary"`, same as now.
  - `SubtitlesState` only gains `secondary: SubtitleTrack | null`; no separate `secondaryConfirmation`, `trackConfirmation` is reused (the same confirmation field reports the most recent commit for both selections).
  - "Primary and secondary cannot be the same track" and "bitmap tracks cannot be secondary" are validated by the core and fail with `invalid`.
- **Reason**: [host] mpv `secondary-sid`, `secondary-sub-pos`.
- **Core size impact**: small. `subtitle_state.rs` gets one more selection slot and clearing on source change.

### R-T4 `subtitles.load` support for ASS/VTT, title and selection mode

- **Requested by**: subtitles A08, A09, D17, D18; subtitle-online D16
- **Initial assessment**: proposed rewrite (accepted, narrower scope)
- **Alternatives**:
  - VTT: the plugin converts to SRT and uploads (losing `::cue` styling, which is ignored anyway); this can stay out of the core.
  - ASS: converting to SRT loses positioning and effects, which defeats the purpose of D17; and the host currently validates only with the SRT parser (`subtitle_upload.rs`), so ASS bytes cannot be uploaded.
- **Suggested rewrite**:
  - `format?: "srt" | "ass"` (`ssa` is treated as `ass`; the host detects `[Script Info]` from the content). **No `"vtt"`**: VTT is converted to SRT by the plugin, on the same in-plugin conversion path as SAMI and MicroDVD, and the core maintains only two validators.
  - `title?: string`: accepted, a few lines of code; it fixes the problem of the host's temporary file name being shown to the user.
  - `select?: "primary" | "secondary" | "none"`: accepted; `"secondary"` depends on R-T3.
  - ASS validation follows the bounded rules of the requirements document (UTF-8, ≤ 2 MiB, two required sections, Dialogue ≤ 20 000).
- **Reason**: [host] the temporary file extension and validation are in the host; [general] both the subtitles and subtitle-online plugins need it.
- **Core size impact**: small. One bounded ASS validator plus two options.

### R-T5 `subtitles.cues` read the per-line timing table

- **Requested by**: subtitles B11
- **Initial assessment**: proposed reject
- **Alternatives**:
  - External tracks loaded by this plugin: the plugin already has the bytes and parses them directly (SRT/ASS parsing is a few dozen lines, and npm has the mature `subtitle` package).
  - Embedded text tracks (local files): the plugin uses ffmpeg to extract the stream as SRT and then uses the same parser:
    ```ts
    // track.embeddedId is the embedded track number given by the host; convert it to the ffmpeg stream index by ffprobe's subtitle stream order
    execFile(ffmpeg, ["-v", "error", "-nostdin", "-i", source, "-map", `0:s:${n}`, "-f", "srt", "pipe:1"],
             { timeout: 15000, maxBuffer: 8 << 20, signal });
    ```
    Results are cached by `track.id` and invalidated on track change and source change (the requirements document already designs it this way).
  - Embedded subtitles of network sources, and external tracks loaded by other plugins: the command is disabled with the reason "This subtitle does not support jumping by line".
- **Cost**: extracting subtitles from a large MKV scans the whole file and can take a few seconds (the requirements document already has a "Reading subtitles…" hint); the plugin must check the mapping between `embeddedId` and the ffmpeg stream index; network sources are not supported.
- **Evidence needed for a rebuttal**: measured ffmpeg extraction time on typical sources (for example a 4 GB MKV); or a sample where `embeddedId` does not match the ffprobe order. The host does already have `native_subtitle.rs::extract`; if the evidence holds, exposing it is not expensive and can be discussed again.
- **Reason**: only the B11 feature uses it, so it does not meet [shared] or [general]; [host] holds only for network sources, which is an edge case.
- **Core size impact (if accepted)**: small to medium (new query, 20 000 line limit, text cleanup).

### R-T6 `SubtitleTrack.codec`, `SubtitleTrack.text`

- **Requested by**: subtitles A08, B11, D18
- **Initial assessment**: proposed accept
- **Alternative (considered)**: for local files, ffprobe can read `codec_name` and map it to tracks; not possible for network sources, and the mapping depends on the same fragile number correspondence as in R-T5.
- **Constraints**: `text` is decided by the core from the codec (`subrip`, `ass`, `webvtt`, `mov_text` and so on are text; `hdmv_pgs_subtitle`, `dvd_subtitle` are bitmap; unknown is `false`); the decision table goes into the reference docs.
- **Reason**: [host] the host already reads the codec from mpv `track-list` but does not expose it; R-T3's "bitmap tracks cannot be secondary" validation needs it as well.
- **Core size impact**: tiny.

### R-T7 New `SubtitleStyle` style fields

- **Requested by**: subtitles D09, D11, D17, A08
- **Initial assessment**: proposed rewrite (accepted, fields merged)
- **Alternative (considered)**: the plugin writes styles into the uploaded ASS (`Style:` lines): this only affects external tracks loaded by this plugin, embedded tracks are not controlled, and it is inconsistent with the scope of the existing `setStyle`. Not feasible.
- **Suggested rewrite**:
  - `bold?: boolean` (`sub-bold`), `shadow?: number` (0–4, `sub-shadow-offset` scaled to 1080p): accepted.
  - Merge `box` and `boxOpacity` into one field: `box?: number | null` (`null` or omitted = off; 0–100 = opacity). One fewer combination of interdependent fields.
  - `assOverride?: "keep" | "force"`: accepted; this is H12.
  - `secondaryPosition?: "top" | "aboveMain"`: done together with R-T3; when libmpv does not support `secondary-sub-pos`, this is explained with `styleReason` and falls back to top.
  - All optional; omitting keeps the current effect; old calls are unchanged.
- **Reason**: [host] only the host can write mpv style properties.
- **Core size impact**: small. A few property mappings and range validations in `subtitle_style.rs`.

### R-T8 Widen `SubtitleStyle.offsetMs` to ±600 000

- **Requested by**: subtitles D05
- **Initial assessment**: proposed accept
- **Alternative (considered)**: the plugin shifts the whole timeline of an external track and uploads it again: not possible for embedded tracks, and each sync adjustment requires a new upload.
- **Reason**: [host] only the host can write `sub-delay`; only the validation range changes, and mpv itself has no such limit.
- **Core size impact**: tiny (one constant and tests).

### Picture and rendering

Common rules for processing nodes: every new `VideoNode` must enter both the live mpv graph and the offline export graph (shared through `effects.rs::render_lavfi`), is subject to the overall limit of 64 nodes, and needs range validation, runtime parameter marking and unit tests. Each existing node is about 30 lines (`effects.rs`); with SDK types, reference docs and offline export verification, each node is about 80–120 lines in practice. So the items below are reviewed one by one on "can this be one node type fewer". Processing nodes themselves are [host] (plugins cannot write filter strings; this is the precondition for security and offline consistency), so in principle they are accepted, only the number is narrowed.

### R-V1 `hue` node

- **Requested by**: video E02
- **Initial assessment**: proposed accept
- **Alternative (considered)**: simulate hue with the `eq` node: `eq` has no hue parameter, so this is not possible.
- **Constraints**: FFmpeg `hue=h=<deg>`, `degrees` −180–180, marked as a runtime parameter (dragging the slider does not rebuild the graph); before marking it runtime, verify on the live graph that the runtime command for `hue` really takes effect.
- **Reason**: [host]; hue is a basic picture adjustment.
- **Core size impact**: tiny.

### R-V2 `sharpen`, `cas` nodes

- **Requested by**: video E04
- **Initial assessment**: proposed rewrite (add only one node)
- **Alternative**: lower the requirement: "Sharpen" and "Edge sharpen" are implemented as two parameter sets of the same filter.
- **Suggested rewrite**: add only `{ kind: "sharpen"; amount: number; size: "small" | "large" }` → FFmpeg `unsharp` (`size` sets `luma_msize_x/y` to 3 or 7; `amount` maps to `luma_amount`, 0–2). "Sharpen" uses `small`, "Edge sharpen" uses `large` (unsharp with a large window mainly enhances outlines). No `cas`.
- **Cost**: the look differs from a dedicated edge sharpening algorithm; if the author can give comparison screenshots showing that `unsharp` with a large window clearly falls short at the user level, `cas` will be considered.
- **Reason**: [host]; but two nodes for one feature is unnecessary growth.
- **Core size impact**: tiny.

### R-V3 `denoise3d`, `temporalDenoise` nodes

- **Requested by**: video E05
- **Initial assessment**: proposed rewrite (add only one node)
- **Alternative**: lower the requirement: the three denoise modes (3D / transient / progressive) become in-plugin presets, all mapped to the four parameters of `hqdn3d` (spatial luma, spatial chroma, temporal luma, temporal chroma). "Transient" = large temporal parameters, small spatial parameters; "progressive" = moderate balanced values.
- **Suggested rewrite**: add only `{ kind: "denoise"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` → `hqdn3d`, range 0–20. No `temporalDenoise`.
- **Reason**: [host]; the temporal components of `hqdn3d` already cover temporal denoising.
- **Core size impact**: tiny.

### R-V4 `deblock`, `deband` nodes

- **Requested by**: video E06
- **Initial assessment**: proposed accept
- **Alternative (considered)**: the two are different algorithms (blocking vs banding) and cannot replace each other; plugins cannot add filters themselves either.
- **Constraints**: `deblock`'s `strength` has only two levels, `weak | strong` (FFmpeg `deblock=filter=weak|strong`); `deband`'s `threshold` and `range` get explicit ranges.
- **Reason**: [host].
- **Core size impact**: tiny (two nodes).

### R-V5 `flip`, `transpose` nodes

- **Requested by**: video E08, E09
- **Initial assessment**: proposed rewrite (merged into one node)
- **Alternative (considered)**: mpv `video-rotate` has no effect with the software renderer (the host already handles container orientation with filters in `source_geometry.rs`); plugins have no alternative.
- **Suggested rewrite**: one node `{ kind: "transform"; rotate: 0 | 90 | 180 | 270; flipH: boolean; flipV: boolean }`, which the core expands into a combination of `transpose` / `hflip` / `vflip`. One node maps to the single concept "picture orientation", and the plugin's chain is shorter.
- **Constraints**: the order in which this stacks with the host's own orientation handling (container rotation metadata) must be documented: host orientation first, and the plugin's `transform` acts on the "already upright" picture.
- **Reason**: [host].
- **Core size impact**: tiny to small (tests for the combined expansion and size swapping).

### R-V6 `deinterlace` node

- **Requested by**: video E10
- **Initial assessment**: proposed rewrite (narrower parameters)
- **Alternative**: the global mpv `deinterlace` property is not undone with the plugin lifecycle, so it does not fit as a plugin effect; there is no in-plugin alternative.
- **Suggested rewrite**: `{ kind: "deinterlace"; when: "interlaced" | "all"; output: "frame" | "field" }`, with the algorithm fixed to `bwdif`. Drop the `method` option (`yadif` is the predecessor of `bwdif` with lower quality; offering both is unnecessary).
- **Reason**: [host].
- **Core size impact**: tiny.

### R-V7 `crop`, `pad` nodes

- **Requested by**: video E12
- **Initial assessment**: proposed rewrite (merged into one node)
- **Alternative**: the plugin computes pixel sizes itself from `stats().video.width/height` and sends a generic `crop`: the real input size after the plugin's `transform` and after the host's orientation handling is not always known, and it must be recomputed on source change, so it is error-prone. Not advisable.
- **Suggested rewrite**: one node `{ kind: "reframe"; mode: "crop" | "pad"; aspect: number; x: number; y: number }` (`aspect` 0.25–4; `x`, `y` −1–1 as offsets). The core generates `crop` / `pad` with FFmpeg expressions (`iw`, `ih`), so it does not need to know the input size, and the core does not need to compute it at run time.
- **Reason**: [host].
- **Core size impact**: small (expression generation and boundary tests).

### R-V8 `frameBlend` node

- **Requested by**: video E18 (frame doubling)
- **Initial assessment**: proposed accept (with a performance acceptance gate)
- **Alternative (considered)**: no in-plugin way; mpv `interpolation` depends on C-GPU.
- **Constraints**: FFmpeg `framerate=fps=N` (blended frame interpolation), `fps` 24–120, not a runtime parameter; it changes the output frame rate, so the frame scheduling of the software rendering path (the rhythm at which the host fetches frames) must be verified not to drop frames at 48/60 fps, and the offline export frame rate must be verified to change accordingly. If acceptance fails, it is withdrawn.
- **Reason**: [host].
- **Core size impact**: tiny (the node), but with regression risk for render timing.

### R-V9 `plugin.video.view()` display layer

- **Requested by**: video E11, E13
- **Initial assessment**: proposed rewrite
- **Alternatives**:
  - Simulate the aspect ratio with R-V7's `reframe(pad)`: this goes into snapshots and exports, which violates "display layer effects do not affect snapshots", and it does not change the display box in the window.
  - Lower the requirement: only crop/stretch (E12), no display layer. The cost is losing the whole group of aspect ratio override, fill window, and zoom/pan features.
  These are mpv vo-level properties; there really is no in-plugin way.
- **Suggested rewrite**:
  - Fields narrowed to `aspect: number | null`, `fit: "keep" | "stretch" | "fill"`, `zoom`, `panX`, `panY`. Drop `scaleX` / `scaleY` ("frame width / frame height ±" is a rarely used edge feature; when needed, `aspect` gives the same distortion). E13 drops the four frame width / frame height commands accordingly.
  - Ownership becomes **single owner**, the same as `provideSequence`: the first plugin to call `view()` owns it, a second one fails with `conflict`; when the owner is deactivated, the core restores the defaults. No "merge across plugins, last write wins", which saves per-field ownership records.
  - Status reuses the four states of `FilterStatus`.
  - Precondition C-SIZE (H2). This interface is not opened before C-SIZE, to avoid releasing an API with wrong results (the requirements document already says that the result is wrong outside a 16:9 window).
- **Reason**: [host] mpv vo properties.
- **Core size impact**: small (property mapping and undo), scheduled after C-SIZE.

### R-V10 `plugin.video.render()` GPU rendering options

- **Requested by**: video E16, E17, E18 (motion compensation)
- **Initial assessment**: proposed rewrite (deferred to the same release as C-GPU)
- **Alternative**: the software path has no way: the bundled FFmpeg has no `zscale` / `libplacebo`, 4K tone mapping on the CPU cannot be real time; GLSL shaders and mpv interpolation only work in the GPU renderer.
- **Suggested rewrite**:
  - Do not release early an interface whose "`status` is always `failed / unsupported`". Before C-GPU, the plugin's related commands hard-code the disabled reason (the requirements document already designs it this way), which needs no help from the core.
  - After C-GPU, whether it merges with V9 as two groups of fields on the same handle or stays separate will be decided then, based on V9's actual form; ownership is single owner, the same as V9.
  - `shaders` limit 16, absolute paths required and the files must exist; compile failures are reported as `failed` with mpv's original text.
- **Reason**: [host]; but the timing depends on H3.
- **Core size impact**: small (property mapping); the precondition H3 is a large change.

### R-V11 JPEG support in `Snapshot.export`

- **Requested by**: capture F07
- **Initial assessment**: proposed reject
- **Alternative**: export PNG and convert it with the macOS built-in `sips`; the requirements document is already designed this way:
  ```ts
  const snap = await plugin.media.capture({ processed: true });
  const job = await snap.export({ format: "png", destination: tmpPng });
  await job.done;
  execFile("/usr/bin/sips", ["-s", "format", "jpeg", "-s", "formatOptions", String(q), tmpPng, "--out", dest],
           { timeout: 10000 });
  ```
- **Cost**: one more encode and a temporary file, about 100 ms more per snapshot; invisible to the user.
- **Reason**: the requirements document itself marks it "optional, not doing it does not affect the feature"; none of the four conditions is met.
- **Core size impact (if accepted)**: small.

### Audio

### R-A1 `channels` audio node

- **Requested by**: audio G05
- **Initial assessment**: proposed accept
- **Alternative (considered)**: plugins cannot add filters; the existing nodes (volume, equalizer, high shelf, dynamic normalization) cannot do channel routing.
- **Constraints**: `mode: "stereo" | "left" | "right" | "mono" | "swap"`, the plugin does not send it for `stereo`; with more than 2 channels the core first downmixes to stereo (`aresample` / `pan` downmix); the downmix is internal to this node and exposes no parameters; whether it is adjustable at run time must be confirmed by testing, and if it cannot be confirmed it is marked non-runtime.
- **Reason**: [host].
- **Core size impact**: tiny to small.

### R-A2 `player.audioDevices` device list

- **Requested by**: audio G16
- **Initial assessment**: proposed rewrite (one design together with A3 and A4, see merged topic 3)
- **Alternative**: the plugin can list devices with `system_profiler SPAudioDataType -json`, but what it gets are not device ids mpv can use, so they cannot be used for switching; listing them alone is pointless.
- **Suggested rewrite**: see R-A3.
- **Reason**: [host] mpv `audio-device-list`.
- **Core size impact**: tiny.

### R-A3 `player.setAudioDevice` select output device

- **Requested by**: audio G16
- **Initial assessment**: proposed rewrite
- **Alternative**: switch the macOS system default output (for example with the `SwitchAudioSource` command line tool): this changes a system-wide setting, and the sound of the user's other applications changes too. Not acceptable.
- **Suggested rewrite** (the form after merging A2–A4):
  ```ts
  player.audioDevices(): Promise<{ id: string; name: string }[]>   // playback.read
  player.setAudioDevice(id: string | null): Promise<void>          // playback.control; null = system default
  PlayerState.audioOutput: { requested: string | null; active: string | null }
  ```
  - Drop `devicesRevision`: plugging or unplugging a device changes `active` (fallback or switching back), which naturally pushes a state update; only a newly plugged-in device that is **not selected** causes no push, and the plugin finds it when the user opens the window that contains the "Output device" submenu, or by polling `audioDevices()` every 5 seconds (one RPC reading one mpv property, negligible cost). Cost: a new device appears in the menu with up to 5 seconds of delay. If the author considers this unacceptable, `devicesRevision` can be the subject of a rebuttal.
  - The core remembers the device choice for **the current run**; saving it across launches is the responsibility of the audio plugin (it calls once on activation); the core does not persist it.
  - For the generated stream (cpal) path, see H9.
- **Reason**: [host] mpv `audio-device`; fallback and switching back also happen in the host.
- **Core size impact**: small.

### R-A4 `PlayerState.audioOutput`

- **Requested by**: audio G16
- **Initial assessment**: proposed rewrite (merged into R-A3, fields narrowed to `requested`, `active`)
- **Alternative**: plugins cannot learn the host-internal fact of a fallback (device unplugged).
- **Reason**: [host].
- **Core size impact**: tiny.

### R-A5 Widen audio `delayMs` to ±10 000

- **Requested by**: audio G09, A11
- **Initial assessment**: proposed accept
- **Alternative (considered)**: none; delay can only be performed by the processing chain.
- **Constraints**: the existing budget in `processing.rs`, "positive and negative contributions each at most 2000 ms, so that the result can still be represented after a plugin is removed", must be widened to 10 000 at the same time, keeping rounding to 10 ms; add budget boundary tests.
- **Reason**: [host]; an offset of more than 2 seconds is common for external audio tracks (A11), and A11 is already proposed for acceptance.
- **Core size impact**: tiny.

### Window and application

### R-W1 `window.state` / `window.onChange`

- **Requested by**: window H03–H05, H07
- **Initial assessment**: proposed rewrite (fields narrowed)
- **Alternative**: plugins cannot read the window size or fullscreen state (host-internal). No alternative.
- **Suggested rewrite**: `WindowState { fullscreen; videoArea: { width; height }; pictureInPicture: { supported; active } }`. Drop `onTop`: always-on-top can only be changed by the plugin that calls `setAlwaysOnTop`, which knows it itself; when the core undoes it, the plugin has already been deactivated. `pictureInPicture` only appears with W5 in round 4; the field is not added before that.
- **Reason**: [host]. `fullscreen` and `videoArea` will also be read by other plugins later (for example interfaces that depend on fullscreen detection), which meets [general].
- **Core size impact**: small (`window_state.rs` already has a snapshot; it is only exposed and pushed).

### R-W2 `window.setVideoArea`

- **Requested by**: window H03, H04
- **Initial assessment**: proposed accept
- **Alternative (considered)**: resize the window with `osascript` + System Events: this needs Accessibility permission; the plugin does not know the bottom bar, sidebar, divider or minimum size rules, so the computed video area is bound to be off. Not feasible.
- **Constraints**: the parameter is the video area (logical points); the core adds the bars according to the existing layout rules (`window_state.rs`) and clamps to the visible screen area; in fullscreen it fails with `unavailable`; it returns the actual value and `clamped`.
- **Reason**: [host] only the host knows the window geometry.
- **Core size impact**: small.

### R-W3 `window.setAlwaysOnTop`

- **Requested by**: window H05
- **Initial assessment**: proposed accept
- **Alternative (considered)**: a plugin process cannot change the NSWindow level of another process; AppleScript has no always-on-top. No alternative.
- **Constraints**: applies to the main window, floating sidebars and tool windows opened later; the core restores it when the plugin is deactivated.
- **Reason**: [host].
- **Core size impact**: small (`objc2-app-kit` already has the `NSWindow` feature; new windows must inherit the level).

### R-W4 `window.setCursorAutoHide`

- **Requested by**: window B30
- **Initial assessment**: proposed accept
- **Alternative (considered)**: plugins receive no pointer events in the video area and cannot hide the host's pointer. No alternative.
- **Constraints**: the current rule is `cursor-hidden = fullscreen && idle && playing` (`main.rs`); this interface only relaxes the `fullscreen` term to "or mode == playing", and the idle timer and occlusion rules are unchanged; restored when the plugin is deactivated.
- **Reason**: [host].
- **Core size impact**: tiny.

### R-W5 `window.setPictureInPicture`

- **Requested by**: window H07
- **Initial assessment**: proposed accept (round 4, with H8)
- **Alternatives**:
  - The original "mini size" requirement: shrink the video area to the minimum with W2 (plus W3 always-on-top); no new API needed.
  - Lower the requirement to the form above, at the cost of not having macOS system picture-in-picture (across Spaces, position managed by the system).
  Picture-in-picture itself can only be started by the host through AVKit; there is no in-plugin way.
- **Reason**: [host] the window, layers and frames are all in the host.
- **Core size impact**: large (see H8). Therefore scheduled for round 4; in rounds 2–3 the window plugin can ship the "mini size" form first, at the author's discretion.

### R-W6 `plugin.app.info` / `onChange`

- **Requested by**: info I05, I09
- **Initial assessment**: proposed rewrite
- **Alternatives**:
  - Node version: the plugin uses `process.versions.node` (the same Node the host provides to plugins).
  - Plugin API version: the manifest's `apiVersion` is `vestra.experimental/0.1`.
  - mpv, libav, Slint, Vestra versions and build number: link-time information of the host, unavailable to plugins.
  - Loaded plugin list: see R-G1.
- **Suggested rewrite**:
  - `plugin.app: { version: string; build: string | null; components: { mpv: string | null; libav: Record<string, string>; slint: string } }`, a **synchronous read-only property**, sent with the handshake on activation; no method call is needed, and no `onChange` either (it does not change while running). No capability needed.
  - Drop `AppInfo.plugins` and `app.onChange`: the plugin list goes through R-G1's `packages.list` / `onChange` only; no second copy is kept.
- **Reason**: [host] version information; [general] "About", "Playback properties" and any later error reporting or diagnostic plugins will use it.
- **Core size impact**: tiny (build-time constants and a handshake field).

### Interface and input

### R-U1 `plugin.ui.image(path)` runtime images

- **Requested by**: playback I07; visualizer J11, H12
- **Initial assessment**: proposed accept
- **Alternatives (considered)**:
  - Slint compile-time `@image-url`: only fixed files inside the package; thumbnails and covers are generated at run time.
  - Draw pixels with Slint (encode the image into a JSON array): large data volume, slow, and the bridge also has a size limit.
  No alternative.
- **Constraints**:
  - `ImageRef` is opaque and can only be created by this function; PNG/JPEG, ≤ 16 MB and ≤ 4096×4096.
  - Paths are allowed under this plugin's `dataDirectory` **or this plugin's package directory** (the latter means the plugin does not have to copy package resources into the data directory); paths out of bounds, or symlinks pointing out of bounds, are rejected.
  - When the content of the file at the same path changes, the plugin must call `ui.image` again to get a new reference (the host caches the decoded result by reference and does not watch files).
  - `ui_values.rs` currently rejects images explicitly; the `PropValue` conversion and the two write paths (panel / window) must be extended.
- **Reason**: [general] two plugins already need it, and plugins showing covers, avatars or previews later will use it; [host] Slint images can only be constructed in the host process.
- **Core size impact**: small to medium (decoding, size validation, caching and release, SDK types).

### R-U2 Drag and drop: `drops.provide`, `Panel.onDrop`, `Panel.onDragOver`

- **Requested by**: playlist A15
- **Initial assessment**: proposed rewrite
- **Alternative**: lower the requirement: no drag and drop; use "Open File…" and "Add Folder…" instead. Cost: drag and drop is one of the most common ways macOS users open files, so its absence is obvious. There is no in-plugin way to receive system drag and drop (winit events only reach the host window).
- **Suggested rewrite**:
  - Video area: `plugin.drops.provide({ extensions: string[]; drop(e: { paths: string[]; modifiers }): unknown }): Disposable`, capability `ui.drop`. The core dispatches by extension; when one extension has several providers, "first registered wins" and a log entry is written (the same rule as binding conflicts); for extensions with no provider, the core does nothing.
  - Panel: `Panel.onDrop(listener: (e: { paths; at: { x; y }; modifiers }) => unknown)`, giving only the drop point, no continuous `onDragOver`. The insertion line degrades to "insert before the row at the drop point", and no insertion line is drawn during the drag. Cost: no live position feedback while dragging. If the author can show that the missing insertion line causes wrong drops (for example, the position cannot be predicted in a long list), this can be a rebuttal.
  - The field name is uniformly `extensions` (lowercase, without the dot), not the broader `kinds`.
- **Reason**: [host] system drag and drop only reaches the host window; [general] later the subtitle plugin (dropping `.srt`) and the audio plugin (dropping `.mka`) will use video area drag and drop, and dispatch by extension is a general mechanism.
- **Core size impact**: medium (winit drag and drop events, converting the drop point to panel coordinates, the dispatch table). Dropping `onDragOver` saves forwarding high-frequency events.

### R-U3 Backdrop mount slot `panels.mount("backdrop")`

- **Requested by**: visualizer J11, H12
- **Initial assessment**: proposed rewrite
- **Alternatives**:
  - Show the cover in the sidebar or bottom bar: that is not the video area, so the requirement itself is not met.
  - Lower the requirement: only OSD info block text (`osd.panel()` showing the track title), no cover or logo.
  The video area is drawn by the host; there is no in-plugin way.
- **Suggested rewrite**:
  - `panels.mount("backdrop", file)`; `panels.backdrop.state: { effective: boolean } | null`. Drop `mode: "idle" | "audio"`: the plugin uses the existing `player.state.phase` (`empty` = idle) and `stats().video` (`null` = no video) to decide itself which content to draw; the core is only responsible for one thing, "show the backdrop when there is no video frame".
  - Only one backdrop at a time (a second one gets `conflict`); it receives no input; layering follows the requirements document: below the host input layer, below on-screen hints and info blocks.
  - Reuse the composition mechanism of panel mounting (`composition.rs`); do not build a second one.
- **Reason**: [host] the video area and "is there a video frame" are both in the host; it is a general mount slot with no business logic.
- **Core size impact**: medium (new mount slot, visibility rules, interface rebuild on first mount).

### R-U4 (Conditional) panel `PopupWindow` extending beyond its bounds

- **Requested by**: playback I06, I07
- **Initial assessment**: proposed reject (condition not triggered)
- **Alternative**: put the hover tooltip in the panel's own area above the progress bar (the bottom bar is 62px high, and the tooltip can be drawn in the progress bar area above the button row); or temporarily make the bottom bar taller on hover. Implement first, and raise it again only if clipping really happens.
- **Reason**: the requirements document itself marks it as a conditional request; it has not been reproduced so far.
- **Core size impact (if accepted)**: medium (popup layer under Slint panel composition).

### R-U5 (Optional) `plugin.commands.popup(path)`

- **Requested by**: video E17 (the `S` key pops up the "Pixel shaders" submenu)
- **Initial assessment**: proposed reject
- **Alternative**: leave `S` unbound (the requirements document already says this is acceptable); or bind it to the "Use shaders" toggle itself. If popping up this plugin's commands inside a panel is needed later, `Panel.showMenu` already exists.
- **Reason**: a convenience key for a single plugin; none of the four conditions is met; popping up a **main menu submenu** would also give the plugin an entry point to other plugins' menu items.
- **Core size impact (if accepted)**: small.

### Timeline and marks

### R-M1 `PlayerState.chapters`

- **Requested by**: marks B21, B22
- **Initial assessment**: proposed rewrite (on-demand query instead, not in PlayerState)
- **Alternative**: local files: `ffprobe -show_chapters -of json`; network sources (mostly stream URLs produced by resolvers) cannot be read, and marks does not know the resolved URL either.
- **Suggested rewrite**: `player.chapters(): Promise<{ sourceId: number; chapters: { title: string | null; start: number }[] }>` (needs `playback.read`); the plugin queries once after each `sourceId` change, when `phase` becomes `ready`, and caches it. Drop `index` (the array index is the index). Not in PlayerState: `source_backend.rs::snapshot` rereads mpv properties on every push, and the static chapter list should not be serialized with every position update.
- **Reason**: [host] mpv `chapter-list`; for network sources only the host can read it.
- **Core size impact**: tiny.

### R-M2 `player.setLoop` / `PlayerState.loop`

- **Requested by**: marks B23; playback draws the range (B22)
- **Initial assessment**: proposed accept
- **Alternative (considered)**: the plugin watches the position and calls `seek(A)`: position events have a granularity of about 0.1 s, so the jump-back point jitters; the jump back races with user actions and produces `stale`; a short stretch of audio and video after B is still played. "Exact looping" is not possible at the user level.
- **Constraints**: if mpv cannot give `remaining` directly (the remaining count of `ab-loop-count`), the field is not provided, only `count`; cleared on source change (H13).
- **Reason**: [host] mpv `ab-loop-*`; [shared] the loop state must be given to another plugin (playback) for drawing.
- **Core size impact**: small.

### R-M3 Timeline mark layer `plugin.timeline`

- **Requested by**: marks B22, B23; drawn by playback
- **Initial assessment**: proposed accept
- **Alternative (considered)**: marks mounts its own panel to draw the marks: the progress bar is in playback's bottom bar, and marks cannot draw into it; plugins cannot communicate with each other. The requirements document already compared three options, and C is the only one without business logic.
- **Constraints**: limits per the requirements document (1000 per layer, titles of 100 characters, cleared on source change, one layer per plugin); `kind` is fixed to three values (the core does not interpret them; they are only used for coloring by the reader). TestHost needs a new `host.timeline`.
- **Reason**: [shared] a typical cross-plugin data layer, the same pattern as `osd.panel()` and the playback sequence flags.
- **Core size impact**: small (per-owner layered lists and pushes).

### Bindings

### R-K1 `plugin.bindings` list, rebind, reset, change notification

- **Requested by**: hotkeys J01, J02
- **Initial assessment**: proposed accept
- **Alternative (considered)**: plugins cannot see other plugins' commands and cannot write the override layer. No alternative.
- **Constraints**:
  - Capability `bindings.manage` (it can change other plugins' keys, so it must be declared explicitly).
  - `BindingInfo.labels` is generated by the core (the same function as the text on the right side of menus), so plugins do not reimplement key display rules.
  - Writes reuse the debounced save of `ui_state.rs`; validation for `set` is shared with the binding validation of `commands.add`.
- **Reason**: [host] the binding table and the override layer are both in the core, and the override layer was reserved for exactly this.
- **Core size impact**: small to medium (list assembly, shadowed computation, orphans, pushes).

### R-K2 `ignoreDefaults` ignore default shortcuts

- **Requested by**: hotkeys J03
- **Initial assessment**: proposed reject
- **Alternative**: the hotkeys plugin implements it itself with R-K1, and records in its own data directory "which overrides I wrote to ignore defaults":
  ```ts
  const { commands } = await plugin.bindings.list();
  const written: BindingRef[] = [];
  for (const c of commands)
    if (!c.overridden && c.defaults.length && !(c.plugin === "host" && ["quit", "exit-fullscreen"].includes(c.command))) {
      await plugin.bindings.set(c, []); written.push(c);
    }
  saveJson("ignore-defaults.json", written);           // when the switch is turned off, reset only these
  plugin.bindings.onChange(() => applyToNewCommands()); // write again after new plugins load
  ```
- **Cost**: after a new plugin loads and before hotkeys receives `onChange` and writes again, there is a very short window in which default keys take effect; when hotkeys itself is deactivated the switch stops working (but the override layer remains, the keys stay ignored, and hotkeys must reconcile the state with the menu check mark on startup).
- **Evidence needed for a rebuttal**: describe the concrete scenario in which a user would hit that race window and see a visible problem.
- **Reason**: a single switch of a single plugin; it is a rarely used setting, of the four conditions only [host] barely applies, and the alternative is nearly equivalent at the user level.
- **Core size impact (if accepted)**: small (one branch in resolution plus one persisted field).

### R-K3 Bindable input `Mouse:LeftClick`

- **Requested by**: hotkeys J02
- **Initial assessment**: proposed accept (merged with H11)
- **Alternative (considered)**: plugins receive no pointer events in the video area and cannot distinguish single from double clicks. No alternative.
- **Constraints**: delayed decision only when a binding exists; without a binding, focus stays immediate (H11); double click detection is unchanged.
- **Reason**: [host] video area pointer events; the mouse binding table is in the core.
- **Core size impact**: small.

### Settings and appearance

### R-S1 Settings page mounting `plugin.settings`

- **Requested by**: preferences J04, H10; hotkeys J01, J02; plugins J07; visualizer H12 (page form)
- **Initial assessment**: proposed rewrite (accept, narrowed form)
- **Alternative**: option C in the requirements document (each plugin puts its settings entry into the "Settings ▶" menu), zero core cost; the price is that there is no central options window. Option B violates plugin independence.
- **Suggested rewrite**:
  - `plugin.settings.page(file, { id, category, title, order? }): SettingsPage`; the inference rules for component and global are the same as for `panels.mount`, with **no extra** `component` / `global` options (panels already have the same rules; when a file exports several, follow the panel handling).
  - `SettingsPage` is just `View & Disposable`; drop `visible` beyond `ready` (whether the page is showing has no behavioral meaning for the plugin).
  - `plugin.settings.show(pageId?)` can only open this plugin's own pages.
  - The category table is fixed in the core; nine categories are enough; `order` sorts only within one category.
  - Container window: see H7.
- **Reason**: [shared] only the core can group Slint pages from several plugins into one window; [general] four plugins already need it, and almost every future plugin will have a settings page.
- **Core size impact**: medium (new mount slot, category tree, page switching; reuses `composition.rs`).

### R-S2 Appearance `plugin.appearance`

- **Requested by**: preferences H10
- **Initial assessment**: proposed rewrite
- **Alternative**: a plugin cannot change the `Theme` of other plugins' windows, so there is no alternative; or lower the requirement and do no color scheme switching (only one color scheme).
- **Suggested rewrite**:
  - `plugin.appearance: { readonly theme: string; onChange(listener); set(theme: string): Promise<void> }`; `set` needs `ui.theme`.
  - Drop the `themes` list and the `preview` metadata: the color scheme table lives in the core with two fixed ids (`vestra` and a second color scheme; the second one was later removed), written into the reference document; the preferences preview block is drawn with its own Slint (the preview is just three color blocks). Add a list later if extensible color schemes are really needed.
  - Drop the separate `corners`: corner radius follows the color scheme and is not adjustable on its own (lowered requirement).
  - The choice is persisted by the core (it affects the host's own interface, so it is core state).
- **Reason**: [shared] Theme is a global shared by all views; [host] the host interface must change with it.
- **Core size impact**: small (API); the real cost is in H6 (medium).

### Plugin management

### R-G1 `packages.list` / `onChange`

- **Requested by**: plugins J07; info I09
- **Initial assessment**: proposed rewrite
- **Alternative**: plugins cannot query each other's running state; `fs.readdir` on the data directory shows which ids have data, but not the running state or install records. No alternative.
- **Suggested rewrite**:
  - Drop `dataDirectory` from `PluginInfo` (the rule is fixed as `<data root>/<id>`, so a plugin can build it itself; when `VESTRA_PLUGIN_DATA` changes the root, the same rule derives it; the reference document states this) and `self` (the caller can compare `info.id === plugin.id`).
  - **`list` and `onChange` need no capability** (read-only public information), so info I09 does not have to declare `plugins.manage`, which can install and uninstall plugins, just to show the list; write operations (G2, G3, the existing install / rollback) still need `plugins.manage`.
  - Merge W6's `AppInfo.plugins` (see R-W6).
- **Reason**: [host] running state and the install store are in the core; [shared] two plugins read the same list.
- **Core size impact**: small (the host already has the `packages.installed` wire method and the runtime table).

### R-G2 `packages.setEnabled`

- **Requested by**: plugins J07
- **Initial assessment**: proposed accept (merged with H10)
- **Alternative (considered)**: a plugin cannot revoke another plugin's host resources. No alternative.
- **Constraints**: disabling itself is `invalid`; trial loads are `unsupported`; development loads apply only to the current run; the disabled set of installed plugins is saved across launches.
- **Reason**: [host]; [core duty] plugin lifecycle already belongs to the core (the host already has `disable(owner)` internally).
- **Core size impact**: small to medium (public interface + disabled set persistence + skipping at startup).

### R-G3 `packages.uninstall`

- **Requested by**: plugins J07
- **Initial assessment**: proposed accept (merged with H10)
- **Alternative (considered)**: the plugin deletes the install store directory directly: this bypasses the journaled commit in `package_store.rs` and would break store consistency. Not feasible.
- **Constraints**: installed plugins only; for `removeData` the core deletes `plugin-data/<id>/` (one plugin is not allowed to delete another plugin's data directory); bindings in the override layer are kept as orphans.
- **Reason**: [host] the core is the only writer of the install store; [core duty].
- **Core size impact**: small (the store adds a journal operation for delete records + gc already exists).

### Host behavior changes

### R-H1 Hand basic playback actions over to the core

- **Requested by**: playback (whole chapter); hotkeys J01
- **Initial assessment**: proposed accept (round 2, top-down process)
- **Alternative (considered)**: keep the basic actions in playback: after playback is uninstalled or replaced, Space, ←/→ and the rest all stop working, and a hotkey settings plugin has no way to bind basic operations. This conflicts with the "default plugins are replaceable" principle.
- **Constraints**:
  - The 14 actions match the table in the requirements document; menu paths, order, default bindings and hint texts stay the same; behavior is moved over from `controller.ts` as is, with a menu snapshot regression.
  - **`rate-10` (Speed ▶ Normal, Z) moves over too**, as `rate-normal`: it belongs to the same basic key group as X / C; leaving it in playback would put "slower, faster" in the core and "reset" in a plugin, so after uninstalling playback you could not reset.
  - Bindings in the override layer for `vestra.playback/<old id>` are mapped once to `host/<new id>` on upgrade.
  - Top-down per §5: the maintainer first builds the core actions and P3, P4, then the playback author migrates; the old commands keep working until the switch.
- **Reason**: [core duty].
- **Core size impact**: medium (14 actions, accumulate and clamp logic, OSD texts, override layer migration). This is the largest item in round 2.

### R-H2 C-SIZE: render size follows the video area

- **Requested by**: video E11, E13; window H07
- **Initial assessment**: proposed accept (round 4)
- **Alternative**: on the fixed 960×540 buffer support only "keep aspect ratio"; skip "don't keep", "fill" and upscaled quality. The price is that half of the results of the whole V9 feature group are wrong.
- **Constraints**: the render size follows the video area's physical pixels (or at least its ratio); window dragging needs throttling to avoid rebuilding the render context per pixel; the CPU cost of a 4K video area under software rendering must be measured.
- **Reason**: [host] rendering is unique to the host; it also improves sharpness of every picture (fullscreen is currently 960×540 upscaled), not just serving a single plugin.
- **Core size impact**: medium to large (render buffer rebuild, frame buffer sharing, isolation from the snapshot path).

### R-H3 C-GPU: libmpv switches to GPU rendering

- **Requested by**: video E16, E17, E18 (motion compensation); window H07 (optional zero-copy)
- **Initial assessment**: proposed accept (round 4; a separate design before work starts, and the scope may need user confirmation per §7)
- **Alternative**: keep software rendering and skip E16, E17 and motion compensation (the plugin hard-codes the reason they are disabled). Other parts of the product are not affected.
- **Constraints**: under Safe Rust (`unsafe` is forbidden in first-party code), connecting the libmpv OpenGL render interface and embedding it in Slint first requires confirming usable safe wrappers (rsmpv's render feature, Slint's low-level OpenGL interface); if this cannot be done, it is a case of "the chosen technical path cannot meet a key scenario" and is escalated per §7. The snapshot and offline export paths are not affected (still offline ffmpeg).
- **Reason**: [host] rendering; it is also the prerequisite for hardware decoding (`hwdec`) and higher-quality scaling, and is a long-term direction of core evolution (technology choices §6 already lists it as pending design).
- **Core size impact**: large, with platform and safety constraint risks.

### R-H4 Add numpad key names to the binding syntax

- **Requested by**: video E13
- **Initial assessment**: proposed accept
- **Alternative**: use only fallback keys (Alt+Plus, Alt+arrow keys and so on), no numpad support. Price: users with an external keyboard cannot use their usual numpad layout, and today the matching between `"5"` and numpad 5 is undefined.
- **Constraints**: the host already gets `PhysicalKey` at the winit layer (`KeyboardInput` in `main.rs`); map the `Numpad*` physical keys to separate input names; main keyboard digits do not match `Numpad*`, and `Numpad*` does not match main keyboard digits. Same change location as H5.
- **Reason**: [core duty] binding syntax belongs to the core; [general] any plugin can use numpad names.
- **Core size impact**: small.

### R-H5 macOS Option key recognition

- **Requested by**: all default bindings that include Alt (several plugins)
- **Initial assessment**: proposed accept (treated as a defect, §4.2 item 1)
- **Alternative**: plugins switch to default keys without Alt: many default keys of reference players could not be carried over, and plugins each finding replacement keys would conflict with each other.
- **Reason**: this is a **defect in the existing contract**: reference document §4.1 allows `Alt+…` bindings, but `from_slint_key` recognizes keys by `event.text`; on macOS Option+key types a different character or a dead key, so the binding never fires. Fix it as a defect; no author justification is needed. Fix: merge in the winit `PhysicalKey` during recognition (same location as H4); also verify the problem where `Ctrl+[` arrives as `\u{1b}` and is misread as `Ctrl+Escape`.
- **Core size impact**: small.

### R-H6 Theme becomes settable by the host

- **Requested by**: preferences H10
- **Initial assessment**: proposed rewrite (narrowed along with R-S2)
- **Suggested rewrite**: only change the color tokens to `in property`; `radius-*` stays constant (R-S2 already dropped separate corners, and the second color scheme also uses the Vestra corner radius; lowered requirement). The host pushes the values when each window, panel and menu instance is created and when the color scheme changes; the two color scheme tables live in the host.
- **Alternative**: no color scheme switching.
- **Reason**: [shared] Theme is shared by all views.
- **Core size impact**: medium (the push path to every view instance, including panel rebuilds and floating windows).

### R-H7 Options container window

- **Requested by**: preferences J04
- **Initial assessment**: proposed rewrite (along with R-S1)
- **Suggested rewrite**: the container has only "category tree + page area + close", with no layout capability beyond scrolling inside the page; the core remembers `settings.lastPage`. Page width is fixed at 440px, and authors design for that.
- **Alternative**: option C (menu entry).
- **Reason**: [shared].
- **Core size impact**: medium (the same cost as R-S1, not counted twice).

### R-H8 Picture-in-picture frame output

- **Requested by**: window H07
- **Initial assessment**: proposed accept (round 4, along with W5)
- **Alternative**: see R-W5 (mini size form).
- **Constraints**: `AVSampleBufferDisplayLayer` / `AVPictureInPictureController` need bindings such as `objc2-av-kit` and `objc2-core-media`; first confirm they are usable under Safe Rust (if not, escalate per §7); the render driver must not depend on the main window's draw callback; frame timestamps map to `CMTimebase`. Do it after C-SIZE, so the floating window can set its render size from the video aspect ratio.
- **Reason**: [host].
- **Core size impact**: large.

### R-H9 Generated stream audio follows the output device

- **Requested by**: audio G16
- **Initial assessment**: proposed rewrite (in phases)
- **Alternative**: lower the requirement: in phase one G16 applies only to media played by mpv; streams generated by plugins (NES) still use the system default device; the menu and the reference document state this limitation.
- **Suggested rewrite**: round 2 does only A2–A4 (mpv path); the cpal path (`producer_audio.rs`) opening by device id and switching back after disconnect is deferred to round 3. The reason is that the one-to-one mapping between cpal device ids and mpv CoreAudio device names must be confirmed first; if it cannot be confirmed, this item becomes two device lists.
- **Reason**: [host]; one setting must cover all output paths to be consistent, so it should be done eventually, but not necessarily in the same batch as A3.
- **Core size impact**: small to medium.

### R-H10 Disabled state persistence and external disabling

- **Requested by**: plugins J07
- **Initial assessment**: proposed accept (merged with R-G2, R-G3)
- **Constraints**: the disabled set is stored in the install store catalog (same transaction boundary as the install records), not in `ui-state.json`; development-loaded plugins are not written.
- **Reason**: [core duty] plugin lifecycle.
- **Core size impact**: small to medium (counted in G2, G3).

### R-H11 Single-click detection

- **Requested by**: hotkeys J02
- **Initial assessment**: proposed accept (merged with R-K3)
- **Constraints**: delay detection only when `Mouse:LeftClick` has a binding (system double-click interval, movement ≤ 4px), the same approach as the existing `has_wheel`; without a binding, focus immediately as now.
- **Reason**: [host].
- **Core size impact**: small.

### R-H12 Configurable ASS override policy

- **Requested by**: subtitles D17
- **Initial assessment**: proposed accept (merged with `assOverride` in R-T7)
- **Constraints**: changing the default from `force` to `keep` (`scale`) is a **default behavior change** (§4.2 item 3): the display of existing embedded ASS tracks will change. Note it in the change notes on release, and cover the "embedded ASS + plugin style" scenario in the GUI regression.
- **Reason**: [host] it is an mpv global property, currently hard-coded in the host startup arguments.
- **Core size impact**: tiny.

### R-H13 A-B loop resets across files

- **Requested by**: marks B23
- **Initial assessment**: proposed accept (merged with R-M2)
- **Reason**: [host] mpv `ab-loop-*` persists across files by default, and only the host can clear it when the source changes; this is part of the M2 contract, not a separate feature.
- **Core size impact**: tiny.

---

## Merged topics

Per §4.2 item 5, the following requests point to the same capability; they are designed together and finalized once, not implemented one by one:

1. **Stepping and basic actions**: R-P1 + R-P2 (combined into `player.step(unit, direction)`) + R-P3 + R-P4 + R-H1. Basic actions need to call frame stepping, and P3/P4 are parts of H1; follow the top-down process of §5, with the playback author migrating.
2. **Playback stats extension**: R-P6 + R-P7 + R-P8, unified as pass-through fields of `PlaybackStats`, queried on demand, not in PlayerState.
3. **Audio output device**: R-A2 + R-A3 + R-A4 + R-H9, one `audioOutput` design in two phases (mpv path / generated stream path).
4. **Secondary subtitles and subtitle loading**: R-T3 + R-T4 (`select` option) + R-T6 + R-T7 (`secondaryPosition`) + R-H12. The `role` of `select`, the `select` of `load`, graphic track validation and the secondary subtitle position are one set of state, designed once.
5. **Video processing nodes**: R-V1–R-V8 review the node list and chain order once; after narrowing there are nine: `hue`, `sharpen`, `denoise`, `deblock`, `deband`, `transform`, `deinterlace`, `reframe`, `frameBlend` (14 originally requested).
6. **Display layer and rendering**: R-V9 + R-V10 + R-H2 + R-H3 + R-P8; the handle form and ownership of the display layer and GPU render options are decided together after C-SIZE / C-GPU.
7. **Window control**: R-W1–R-W5 + R-H8, one `plugin.window` module and the capability `window.control`; the picture-in-picture part (W5, H8, `pictureInPicture` of W1) is split into round 4.
8. **Plugin and app information**: R-W6 + R-G1 + R-G2 + R-G3 + R-H10. There is only one plugin list (`packages.list`, read-only, needs no capability); `plugin.app` keeps only version information.
9. **Timeline and loops**: R-M2 + R-M3 + R-H13 (and the query of R-M1); when drawing, playback reads two sources, `timeline.marks` and `PlayerState.loop`.
10. **Bindings**: R-K1 + R-K3 + R-H4 + R-H5 + R-H11, all in one place, `bindings.rs`; H5 is fixed first as a defect.
11. **Settings and appearance**: R-S1 + R-S2 + R-H6 + R-H7; both the settings container and the theme push have to walk all view instances, so they share one "push to all views" mechanism.
12. **Video area mounting and images**: R-U1 + R-U3 (and the video area drop target of R-U2). The backdrop layer, drop target and runtime images all sit on the video area and panel composition mechanism.

### Candidates proposed for promotion to general core capabilities

Per §4.3 item 4, clause four (needed by several plugins, or a general basic capability), the following capabilities are currently implemented **separately** by several plugins with system commands in the requirements document, or are proposed in appendix A only for a single feature. They are proposed to be provided once as general capabilities; the "Implementation layer" column says where they cost the core least: what can be implemented in the plugin runner (`src/runtime/runner.ts`, in each plugin's Node process) does not go into the Rust host, so host size stays the same, while behavior is unified and the runner cleans up on disable.

| Candidate | Who implements it separately now | Proposed form | Implementation layer | Reason and cost |
| --- | --- | --- | --- | --- |
| Clipboard write (text, image) | playback I08 (`pbcopy`), info I05 / I09 (`pbcopy`), capture F03 (`osascript` writes the image) | `plugin.clipboard.writeText(text)`, `plugin.clipboard.writeImage(path)`; add reading once there is a need | Host (`objc2-app-kit` with the `NSPasteboard` feature) | Three plugins; `pbcopy` encoding depends on the `LANG` environment variable, and writing images with `osascript` depends on script concatenation; each plugin writes it differently and behavior easily diverges. The host already links AppKit, so the increment is very small |
| Runtime images | playback, visualizer | That is R-U1 | Host | Already listed, proposed accept |
| Settings page mounting | preferences, hotkeys, plugins, visualizer | That is R-S1 | Host | Already listed, proposed rewrite, then accept |
| Prevent sleep during playback | playback B29 (`caffeinate`) | Host behavior: while playing (`ready && !paused`, including generated streams) hold a power assertion; plugins only toggle it: `plugin.power.preventSleep(mode: "off" \| "playing")`, revoked on disable | Host (just start a `/usr/bin/caffeinate -d -w <host pid>` child process, no IOKit bindings needing `unsafe`) | Only one plugin for now, but "keep the display awake while playing" is a basic behavior of every player, and the basis for deciding "is playing" (including emulator streams) is in the host; when a plugin uses `caffeinate -w <plugin pid>`, a plugin crash and restart leaves or loses the assertion. Lower priority than the first three |
| Confirmation dialog | playlist C18 (replace list), marks B21 (clear bookmarks), hotkeys J01 / J03 (restore defaults, ⌘Q, ignore defaults), plugins J07 (disable, uninstall) | `plugin.dialogs.confirm({ message, detail?, confirm, cancel? }): Promise<boolean>`, native NSAlert; one optional checkbox (J07 "also delete data") | Host (the already-used `rfd` has `MessageDialog`) | Four plugins each drawing a Slint confirmation window would make button order, default key and Esc behavior inconsistent; same module as the existing `dialogs.*`, small increment |
| Show in Finder / open folder | playlist C23 (`open -R`), capture F09, video E17, plugins J07 (`open`) | `plugin.system.reveal(path)`, `plugin.system.openPath(path)` | Runner (Node `execFile("/usr/bin/open")`) | Four plugins; the implementation is one line, but path validation (message when it does not exist) and the no-shell rule can be unified. Not in the host |
| External tool calls (ffprobe, ffmpeg) | playlist, info I05, playback I07, visualizer J11 (and the alternatives of R-P2, R-T5) | `plugin.tools.run("ffprobe" \| "ffmpeg", args, { timeout, signal, maxOutput })`: locate uniformly via `VESTRA_FFPROBE` / `VESTRA_FFMPEG` / PATH, no shell, timeout, end the process when the plugin is disabled, a uniform `unavailable` when not found | Runner | Appendix B.4 already sets uniform rules, yet each plugin has to implement them again; in the runner the rules exist only once. Not in the host |
| Atomic read/write of plugin settings files | Almost all plugins (`settings.json`, `style.json`, `marks.json` and so on, 45 places in the requirements document) | `plugin.storage.json<T>(name, { defaults, validate? })` → `{ value; save(next) }`: write a temp file then rename, use defaults when the file is corrupt, with a version number | Runner | The rules of reference document §3.4 (temp file + rename, defaults when corrupt) are rewritten by every plugin. Pure Node implementation, not in the host |
| Enter-URL dialog (UI-DLG-02) | playback A02, playlist "Add URL…" | No API; provide a reusable input row component in `@vestra/widgets.slint` | Host UI library (a Slint component, not protocol) | Two plugins each write an interface to the same spec; putting it in the widget library unifies the look without adding protocol |

Not proposed for promotion: `sips` format conversion (only capture), `system_profiler` / `sw_vers` / `sysctl` (only info), the keychain (only subtitle-online), `fs.watch` (only subtitles). They are all special needs of a single plugin and stay in the plugin.

---

## Preliminary schedule

Only items proposed for acceptance or rewrite are listed (rewrites are scheduled in their rewritten form); the 6 items proposed for rejection (R-P5, R-T5, R-V11, R-U4, R-U5, R-K2) are not scheduled and wait for the evidence of step 2.

**Round 2: playback, tracks, processing nodes, basic action handover**

- Basic actions: R-H1, R-P3, R-P4, R-P1 + R-P2 (`player.step`)
- Playback information: R-P6 + R-P7 (`PlaybackStats` extension, without `transfer`)
- Tracks and subtitles: R-T1, R-T2, R-T3, R-T4, R-T6, R-T7, R-T8, R-H12
- Processing nodes: R-V1–R-V8 (the nine after narrowing), R-A1, R-A5
- Audio output phase one: R-A2 + R-A3 + R-A4 (mpv path)
- Timeline: R-M1, R-M2, R-M3, R-H13
- Defect fixes: R-H5 (can come early, done together with R-H4 in the same change)

**Round 3: interface, window, settings, bindings**

- Bindings: R-K1, R-K3, R-H11, R-H4
- Window (without picture-in-picture): R-W1 (without `pictureInPicture`), R-W2, R-W3, R-W4
- App and plugin management: R-W6, R-G1, R-G2, R-G3, R-H10
- Settings and appearance: R-S1, R-H7, R-S2, R-H6
- Video area and images: R-U1, R-U2, R-U3
- Audio output phase two: R-H9 (generated streams follow the device)
- Adopted items among the general capability candidates (clipboard, confirmation dialog, runner-side `tools` / `storage` / `system`)

**Round 4: C-SIZE, C-GPU, picture-in-picture**

- R-H2 (C-SIZE), followed by R-V9 (display layer)
- R-H3 (C-GPU), followed by R-V10 (render options) and R-P8 (`transfer`)
- R-W5 + R-H8 (picture-in-picture) and the `pictureInPicture` field of R-W1

Totals: 67 items (54 interface items from appendix A + 13 host behavior items); 6 proposed for rejection, 30 for rewrite, 31 for acceptance.

---

## Step 2: author responses

Respondents: author representatives of each plugin (playback, playlist, subtitles, subtitle-online, video, audio, capture, info, window, history, marks, hotkeys, preferences, plugins, visualizer). Per §4.3 item 3: each item may be rebutted at most once, and a rebuttal must come with evidence. Where an alternative is feasible at the user level, it is accepted.

Test environment: MacBook Air (Mac14,2, 8 GB memory, internal SSD), macOS 26.2, mpv 0.41.0, FFmpeg 9.0.2. Test scripts and outputs are in the `neg/` directory of the session scratchpad (`gap.py`, `gap2.py`, `tl.lua`, `small.mkv`, `big.mkv` and so on), not in the repository. This machine has no loopback recording device, so R-P5 measures event timing, not the recorded waveform (see that item's notes).

### Summary of conclusions

| Item | Initial assessment | Author response | Key points |
| --- | --- | --- | --- |
| R-P5 | Proposed reject | Accept | The measured switch gap is below the 150 ms threshold set by the maintainer; B26 is downgraded to "fast advance". One host improvement suggestion attached (not a new API) |
| R-T5 | Proposed reject | Accept | Extraction inside the plugin takes the same time as the existing host implementation (both scan the whole file); the index mapping holds in the samples. One field addition attached, under R-T6 |
| R-V11 | Proposed reject | Accept | `sips` measured: 1080p to JPEG 0.05 s, to BMP 0.03 s |
| R-U4 | Proposed reject | Accept | "Draw inside the panel" as the maintainer suggested is not possible, but the host composition does not clip, so the hint can be drawn outside the panel. Request to write "no clipping" into the reference document |
| R-U5 | Proposed reject | Accept | `S` stays unbound |
| R-K2 | Proposed reject | Accept | The race only appears when a plugin is newly installed, and the window is milliseconds; a core boolean would not solve the leftover problem after uninstalling hotkeys either |
| 30 rewrite items | Proposed rewrite | All accepted | Author-side costs for some items are noted below |
| R-T6, R-U1 | Proposed accept | Additions | `SubtitleTrack.streamIndex`; release rules for `ImageRef` |

**No rebuttals in this step.** All 36 items (6 rejections + 30 rewrites) are accepted.

### Items proposed for rejection

#### R-P5 `player.preload` gapless playback: accepted

The evidence the maintainer asked for is a measurement of the switch gap between two consecutive flac files on the current host. Method and results follow.

**Method**: reproduce the host's advance path with mpv 0.41.0: `keep-open=yes`, load options identical to what host `source_backend.rs::load_with_options` uses for local files (`demuxer=lavf,demuxer-lavf-probe-info=yes,rebase-start-time=yes`). When `eof-reached` turns true, immediately `loadfile <next track> replace`, and time until the next track's `playback-restart` event. Test sources are two 6-second, 440 Hz sine wave flac files. Volume set to 0 and muted to avoid producing sound.

| Scenario | Runs | eof → next track starts (ms) |
| --- | --- | --- |
| `ao=null`, both 44.1 kHz flac | 15 | median 3.7, range 2.4–8.4 |
| `ao=coreaudio`, both 44.1 kHz flac | 15 | median 9.0, range 4.0–11.3 |
| `ao=coreaudio`, 44.1 kHz → mp3 44.1 kHz | 10 | median 9.8, range 4.6–14.5 |
| `ao=coreaudio`, 44.1 kHz → 48 kHz flac (audio output must be rebuilt) | 10 | median 58.4, range 56.0–60.4 |

**The host adds further delay** (derived from reading the code, not measured in the GUI):

- Natural end is detected only in the state snapshot: `source_backend.rs` reads `eof-reached` around line 442, and that code runs only in the 100 ms cycle at line 368 of `backend.rs`. So `ended` reaches playlist's `media.onEnd` 0–100 ms late.
- After that come the plugin RPC round trip, `File::open` and the fd lease, then `loadfile`. These steps are estimated at a few milliseconds.

Total: about 5–115 ms for a same-format switch (about 60 ms on average), about 60–165 ms when the sample rate changes. Only the worst combination, "sample rate change that also hits the end of a cycle", exceeds 150 ms. By the maintainer's threshold, the evidence does not support introducing preload in this round, so it is accepted.

**Author-side cost and follow-up**:

1. B26 is downgraded to "fast advance". The menu name becomes "Playlist ▶ Fast Advance to Next", and the acceptance criterion changes from "recorded waveform gap < 20 ms" to "same-format switch < 150 ms". The playlist author updates the requirements document. Note: even a 5–10 ms break in a continuous waveform produces an audible click, so "true gapless" for live albums and classical music is given up this time. If raised again later, it will come with recorded waveform evidence (needs the coordinator to measure on a machine with a loopback device).
2. playlist follows alternative 1: compute the next item in advance, `fs.stat` it in advance, and call `open` only once in the `onEnd` callback.
3. **Suggestion (not an API request; can be discussed separately per §4.2 item 1)**: when the source phase changes (`ready → ended`), push `source.state` immediately instead of waiting for the 100 ms cycle. This removes the bulk of the 0–100 ms above. The change is in the `backend.rs` event loop: call `snapshot` and broadcast right after a phase change. All plugins that use `onEnd` (playlist, history, marks) benefit, and the contract does not change. If the maintainer thinks it is not worth doing, this item's conclusion is unaffected.

#### R-T5 `subtitles.cues`: accepted

**Timing measurements**: a 1-hour MKV muxed with two subtitle tracks (subrip, ass, 1800 lines each) and one font attachment; extracted with the command the maintainer gave (`-map 0:s:N -f srt pipe:1`), 3 runs each:

| File | Size | Extract subrip | Extract ass |
| --- | --- | --- | --- |
| `small.mkv` (1.5 Mbps, 3600 s) | 0.68 GB | 0.15–0.44 s (first run 0.44) | 0.14 s |
| `big.mkv` (7.5 Mbps, 4500 s) | 4.25 GB | 2.61–2.66 s | 2.61–2.76 s |

A 4 GB file takes about 2.6 s, mostly disk reading (a machine with 8 GB memory cannot cache the whole file; about 1.6 GB/s). The host's existing `native_subtitle.rs::extract` also starts an ffmpeg that scans the whole file (`-threads 1`, even a little slower), so putting it in the core would **not be faster**. Performance is not a reason to put it in the core.

**Index mapping**: `SubtitleTrack.embeddedId` is mpv's per-type track number (line 275 of `subtitle_backend.rs` takes `id`), not the container stream number. In the sample above (video, audio, subrip, ass, attachment, plus one `--sub-file` external track), mpv's `sub id=1/2` corresponds to `ff-index=2/3`, that is `0:s:0/1`; the external track comes after (`id=3`, `external=true`). So "`embeddedId − 1` = the Nth subtitle stream" holds in the sample; no counterexample was found.

**Author-side cost**:

- The timeout cannot be a fixed 15 s. A 4 GB file takes 2.6 s on SSD; on a USB hard disk (about 100–150 MB/s) it is estimated at 30–40 s. subtitles sets the timeout from the file size (about 1 s/100 MB, minimum 15 s), and keeps the "Reading subtitles…" hint from the requirements document.
- Jumping by line is not supported for embedded subtitles from network sources; the command is disabled with the reason stated (accepted).
- Embedded ASS tracks can also be extracted as SRT (measured, it works), so B11 works for ASS too. The host's existing extract only supports subrip (line 666 of `subtitle_backend.rs`); putting it in the core would need a further extension.

Addition: "`embeddedId − 1` = the Nth subtitle stream" relies on an ordering mpv does not document. When the host extracts itself it uses `ff-index` (lines 290 and 675 of `subtitle_backend.rs`), and a comment in `media_tracks.rs` states that ff-index is reliable only under lavf. So that plugins do not depend on this ordering, we request that R-T6 also expose `streamIndex`; see "Additions to items proposed for acceptance" below. This is not a rebuttal of R-T5.

#### R-V11 `Snapshot.export` JPEG: accepted

Measured: a 1920×1080 PNG (336 KB) converts with `sips -s format jpeg -s formatOptions 85` in 0.05 s (giving 163 KB), and to BMP in 0.03 s. Faster than the maintainer's 100 ms estimate; users will not notice. capture implements the alternative: the temporary PNG is placed in `dataDirectory/tmp/` and deleted after conversion.

#### R-U4 Panel `PopupWindow` beyond its bounds: accepted (with one documentation request)

"Draw inside the panel" as the maintainer suggested is **geometrically impossible**:

- The bottom bar `PlaybackBar` has fixed `min-height` and `max-height` of 62px (`bar.slint` lines 214–215), and the progress line is at `y = 6px` (line 226). There are only 6px above the line, while I06 requires the hint's bottom edge 4px above the progress line, drawn over the video area.
- The I07 popup is a 160×90 thumbnail plus time text, at least about 112px tall, taller than the whole bottom bar.
- "Temporarily make the bottom bar taller on hover" would squeeze the video area: the bottom bar slot is a `vertical-stretch: 0` row in a vertical layout (`composition.rs` line 171). The whole picture would jump on every hover, which is not acceptable.

But `PopupWindow` is not needed: the host composition **does not clip**. Neither the outer `Rectangle` nor the `FocusScope` of the bottom bar generated at `composition.rs` lines 171–175 has `clip`, and in fullscreen the bottom bar is already drawn at `y: -self.height` outside a zero-height outer element, over the video area. That means elements with negative `y` inside the component are drawn directly over the video area. playback builds I06 and I07 this way: the hint box sits inside `seek-row` with a negative `y`, clamped to the bottom bar width.

**Request (documentation, not API)**: state in reference document §6.3 that "content of the bottom bar panel can be drawn into the video area above the slot; the host does not clip; side bars are above it". This keeps future composition changes (for example the R-U3 backdrop layer, or the R-S1 settings window reusing `composition.rs`) from quietly adding `clip` and breaking I06 and I07. If the maintainer does not want to make it part of the contract, the playback author accepts the risk and adds a hover screenshot check to playback's GUI regression.

#### R-U5 `commands.popup`: accepted

`S` stays unbound; the "Use shaders" toggle keeps only Shift+P. The video requirements document removes the `S` key description accordingly.

#### R-K2 `ignoreDefaults`: accepted

Checking the costs the maintainer gave, one by one:

- **Race window**: the override layer is saved across launches by the core, so commands that exist at startup are always in the "overridden" state; there is no window. Only when a plugin is **newly installed or newly enabled** while the switch is on do its default keys briefly take effect before hotkeys receives `onChange` and writes the override. This is one RPC round trip, milliseconds; we cannot find a scenario where the user presses that new key in that time.
- **hotkeys disabled or uninstalled**: with the alternative, the empty overrides written stay in the override layer, the default keys stay disabled, and there is no interface to restore them (only re-enabling hotkeys). But the original approach in the requirements document (the core persists a boolean) would also leave `ignoreDefaultBindings: true` behind, the same problem. So this cannot be a reason to rebut.
- **Author-side handling**: hotkeys records the entries it wrote in `ignore-defaults.json`. Entries the user changed while the switch was on are removed from the record, and when the switch is turned off only the remaining entries are `reset`. The "ignored" display in the J01 list is derived by hotkeys from the record and does not depend on the core distinguishing them.

### Items proposed for rewrite

All accepted. Below are only the author-side work, costs or questions; anything not mentioned follows the suggested rewrite.

**Player**

- **R-P1 / R-P2 `player.step(unit, direction)`**: accepted. With `can.frameStep` gone, playback queries `stats()` once after `sourceId` changes and the source enters `ready`, and caches "has video" for the command's `enabled`. Generated streams (`state.kind === "stream"`) are disabled by playback itself, so the user does not get `unsupported` only after clicking. playback implements dropping in-flight repeated key presses while F is held.
- **R-P6 `videoSize` merged into stats**: accepted. window uses the same "query `stats()` once after ready and cache it" to compute `enabled`; for audio-only the menu can still be greyed out in advance, and users see no difference.
- **R-P7 `PlaybackStats` extension**: accepted, including not adding `hwdec` for now. The "Hardware decoding" row in info I05 shows "Not used" for now, until C-GPU.
- **R-P8 `transfer`**: accepted, round 4.

**Tracks and subtitles**

- **R-T2 `selectAudioTrack`**: accepted.
- **R-T3 Secondary subtitles (`role` of `select`)**: accepted. Cost: the two roles share one `trackConfirmation`. subtitles serializes primary and secondary subtitle selection (waits for the previous confirmation before submitting the next), so the confirmation state always maps to the right menu item.
- **R-T4 `load` supports ASS, title and selection mode**: accepted, including not adding `"vtt"`. subtitles and subtitle-online each do VTT-to-SRT conversion (about 40 lines). Since the two plugins cannot share runtime code, the same implementation is put into both packages.
- **R-T7 Style fields**: accepted, `box` merged into one number field.

**Video and rendering**

- **R-V2 Add only `sharpen` (unsharp, two sizes)**: accepted. No `unsharp` vs. `cas` comparison screenshots this round, so no rebuttal on `cas`.
- **R-V3 Add only `denoise` (hqdn3d)**: accepted; the three denoise presets define their parameters in the video plugin.
- **R-V5 `transform`**: accepted.
- **R-V6 `deinterlace` fixed to bwdif**: accepted. The algorithm radio choice is removed from the E10 menu.
- **R-V7 `reframe`**: accepted.
- **R-V9 `video.view()`**: accepted: drop `scaleX` / `scaleY`, E13 removes four commands; changed to a single owner; not exposed before C-SIZE.
- **R-V10 `video.render()`**: accepted. Before C-GPU, the commands for E16, E17 and motion compensation hard-code the reason they are disabled.

**Audio**

- **R-A2 / R-A3 / R-A4 Output device**: accepted, including dropping `devicesRevision`. audio polls `audioDevices()` every 5 seconds while the workbench window containing the "Output Device" submenu is open, and stops when the window closes. The main menu submenu refreshes when `sourceId` or `audioOutput` changes. A newly plugged-in device appears at most 5 seconds late, which is acceptable.
- **R-H9 Generated streams follow the device (in phases)**: accepted. In phase one, a line is added under the G16 menu: "Emulator sound still uses the system default device".

**Window and app**

- **R-W1 `window.state`**: accepted, `onTop` dropped.
- **R-W6 `plugin.app`**: accepted (synchronous read-only, no `onChange`).

**Interface and input**

- **R-U2 Drag and drop (no `onDragOver`)**: accepted. Cost: no insertion line when dragging in a long list. playlist downgrades to "insert before the row it lands on; append to the end when it lands on empty space". The core gives panel coordinates, and playlist also needs the list scroll position to convert that to a row. The way is to send the scroll position back in Slint with `changed viewport-y => { Bridge.scrolled(...) }` (side bars already use `changed` handlers); no core help needed.
- **R-U3 Backdrop layer (`mode` dropped)**: accepted. visualizer uses `phase` and `stats().video` to decide by itself what to draw. In audio files with cover art, mpv treats the cover as a video track (`stats().video` is not empty). Then the backdrop layer is not shown, and the user sees the cover rendered by mpv. This matches J11's expectation; noted here so it is not later reported as a defect.

**Timeline**

- **R-M1 `player.chapters()`**: accepted. marks queries once after ready, caches it, and writes it into its own timeline layer for playback to draw.

**Settings and appearance**

- **R-S1 `settings.page`**: accepted; 440px width matches the requirements document. The three-column table of hotkeys J01 is relaid for 440px (command 220, shortcut 140, status 60).
- **R-S2 `appearance`**: accepted: drop the `themes` list and `corners`, two fixed ids (`vestra` and a second color scheme, the latter later removed). preferences draws its own preview block.
- **R-H6 Theme push**: accepted, corner radius stays constant.
- **R-H7 Options container window**: accepted.

**Plugin management**

- **R-G1 `packages.list`**: accepted. Drop `dataDirectory` and `self`; `list` and `onChange` need no capability, so info I09 does not have to declare `plugins.manage`.

### Additions to items proposed for acceptance

- **R-T6: add a field `SubtitleTrack.streamIndex: number | null`** (the container stream number of an embedded track, that is mpv `ff-index`; `null` for external tracks or when unknown). Reasons:
  1. The host already gets it in the same `track-list` read (line 290 of `subtitle_backend.rs`); like `codec` it only needs to be exposed, and the core increment is one field.
  2. The host uses it itself when extracting subtitles (line 675), and notes that ff-index is reliable only under lavf. With the same value, plugins can `-map 0:<streamIndex>` without depending on the undocumented rule "mpv track numbers are in the same order as subtitle streams". The R-T5 alternative becomes reliable as a result.
  3. subtitles is not the only user (B11 line jump, D18 export of embedded subtitles): capture choosing "subtitles frozen with the snapshot" for comparison on export, and info I05 showing stream information, both need to map tracks to container streams.

  If the maintainer thinks the "reliable only under lavf" premise of `ff-index` makes this field's semantics unstable, the rule can be: always `null` for non-lavf or network sources. That matches exactly the "network sources not supported" boundary of R-T5.
- **R-U1 `ui.image`**: please state when an `ImageRef` is released. I07 hover thumbnails go up to 200 per source, and references change often as the pointer moves. The hoped-for rule: a reference belongs to the scope that created it; calling `ui.image` again on the same path invalidates the old reference (the host discards the old decoded result); or the host keeps a per-plugin LRU with a documented limit (for example 64 MB decoded memory per plugin). Without clear rules, long hovering could make host memory keep growing.
- **R-H1 Basic action handover**: the playback author confirms migrating per §5. What needs migrating is the bottom bar buttons (switching to `host.run`) and the accumulate and clamp logic in `controller.ts`; old ids in the override layer are mapped by the core.

### Position on the "general core capability candidates"

| Candidate | Plugins that would use it and how | Position |
| --- | --- | --- |
| Clipboard write | playback I08 (copy file name, folder, path; currently `copy.ts` + `system.ts` calling `pbcopy`); info I05 / I09 (copy property text); capture F03 (copy the screenshot; currently needs an `osascript`-built script) | **Support**. Hope `writeImage(path)` accepts PNG and JPEG; on failure return uniform error codes so the OSD texts of the plugins are consistent |
| Runtime images | That is R-U1 | See the addition above |
| Settings page mounting | That is R-S1 | Already accepted |
| Prevent sleep during playback | playback B29 (currently `sleep.ts`, 94 lines: `caffeinate -d -i` with video, only `-i` for audio-only, `-w <plugin pid>`) | **Support, low priority**. Two points: (1) the core must distinguish "has video frames" from "audio only": the former prevents display sleep, the latter only system sleep. So `mode` is more than `"playing"`, or the core decides by itself from video frames. (2) The maintainer's claim that "a plugin crash leaves the assertion" does not hold, because `-w` follows the plugin process and the assertion ends when the plugin crashes; the only problem is the assertion being lost in the time before the crash restart. The real reason to put it in the core is that the "is playing" decision (including generated streams) is in the host |
| Confirmation dialog | playlist C18 (currently `confirm.ts` + self-drawn `confirm.slint`); marks B21; hotkeys J01 / J03; plugins J07 (disable, uninstall, with the "also delete data" checkbox) | **Support**. Hope for an optional `destructive: true` (NSAlert `hasDestructiveAction`, for "Uninstall" and "Clear Bookmarks"); otherwise the maintainer's form. playlist will delete its self-drawn confirmation window |
| Show / open folder | playlist C23 (currently `finder.ts`, 51 lines); capture F09; video E17; plugins J07 | **Support**; a runner implementation is enough |
| External tool calls | playlist (`probe.ts`, `media.ts`, ffprobe); info I05; playback I07 (ffmpeg thumbnails); visualizer J11; **subtitles B11** (after R-T5 was rejected, it switched to in-plugin ffmpeg extraction) | **Strongly support**; this is the item used by the most plugins. Needs: `timeout` given by the caller (R-T5 measurements show large files take tens of seconds), `maxOutput` at least 16 MB (subtitle SRT can be around 8 MB), end the process when the scope is released, fail uniformly with `unavailable` when the executable is not found (the "needs ffmpeg" reason in each plugin's menu should be consistent) |
| Atomic settings file read/write | Almost all plugins; playback `settings.ts` (61 lines) and playlist `store.ts` (146 lines) alone have two implementations of "temp file + rename + defaults when corrupt" | **Support**. Hope `save` has built-in debounce (for example merging into one write every 300 ms, since playlist writes continuously while drag-sorting), and that on `validate` failure the corrupt file is renamed and kept as `.bad` so users can investigate |
| Enter-URL dialog component | playback A02 (currently `open-url.slint` + `open-url.ts`, 123 lines); playlist "Add URL…" | **Support**; a `@vestra/widgets.slint` component is enough, no protocol needed |

The four items "not proposed for promotion" (`sips`, `system_profiler` and similar, the keychain, `fs.watch`): the authors of capture, info, subtitle-online and subtitles all agree to keep them in the plugins.

### Totals

36 items answered (6 proposed for rejection, 30 for rewrite): **36 accepted, 0 rebutted**. 3 additions to items proposed for acceptance (R-T6 adds `streamIndex`, R-U1 release rules, R-H1 migration confirmation). Two further requests that are not APIs: the R-P5 suggestion to "push immediately on phase change", and the R-U4 request to write "the bottom bar does not clip" into the reference document.

---

## Step 3: Rulings

Ruled by: the API maintainer. Based on the step 1 replies, the step 2 author responses (36 accepted, 0 rebutted) and §4.3 item 4 (including clause four, "general basic capability"). This step only fixes conclusions, final forms and rounds; it does not implement.

Adjustments relative to the step 1 "Preliminary schedule":

- Round 2 was cut to fit the size budget (see "Core size estimate" below): secondary subtitles (R-T3 and its related fields), chapters and loops (R-M1, R-M2, R-H13), audio output device (R-A2–R-A4) and the frame blending node (R-V8) were moved to rounds 3a and 4a.
- The original round 3 was too large and is split into **3a** (input, window, plugin management, audio output, chapters and loops, secondary subtitles, confirmation dialog) and **3b** (interface composition: settings pages, theme, runtime images, drag and drop, backdrop layer, timeline mark layer).
- The original round 4 is split into **4a** (C-SIZE, display layer, picture-in-picture, frame blending) and **4b** (C-GPU and render options). Before 4b starts there must be a design, and a usable render wrapper under Safe Rust must be confirmed; if that cannot be done, escalate to the user per §7.
- The mpv path and the generated stream path (R-H9) of the audio output device go into the same round (3a) and are delivered at once; the "phase one note line" the author mentioned in the R-H9 response is no longer needed.

### 3.1 Ruling per item

Three conclusions: **reject + alternative**, **accept after rewrite**, **accept**. "Final form" is a one-line signature or behavior; for details, the suggested rewrite in the corresponding step 1 subsection applies.

| Item | Conclusion | Final form | Round |
| --- | --- | --- | --- |
| R-P1 Frame step | Accept after rewrite | `player.step(unit: "frame" \| "keyframe", direction: "backward" \| "forward", opts?: { ifCurrent?: PlayerState }): Promise<void>`; no `can.*`; `unsupported` when conditions are not met | 2 |
| R-P2 Keyframe | Accept after rewrite | Merged into R-P1's `unit: "keyframe"` (mpv `seek ±0 relative+keyframes`) | 2 |
| R-P3 Level 1 step size | Accept | `player.setStepSeek(options: { seconds: number }): Promise<void>` (0.1–3600, default 5; restored after the caller is disabled) | 2 |
| R-P4 Run core actions | Accept | `plugin.host.run(action: HostAction): Promise<void>`; `HostAction` is the 14 action ids of H1; `unavailable` when disabled; needs `commands` | 2 |
| R-P5 Gapless preload | Reject + alternative | No `preload`; playlist uses `onEnd → open(next, { ifCurrent })`, B26 downgraded to "fast advance"; the host switches to pushing immediately on phase change (see 3.2 item 1) | 2 (host behavior) |
| R-P6 Display size | Accept after rewrite | Merged into R-P7: `PlaybackStats.video.displayWidth/displayHeight: number \| null`, not in PlayerState | 2 |
| R-P7 Stats extension | Accept after rewrite | `PlaybackStats.video += { decoder; pixelFormat; colorspace; displayWidth; displayHeight; displayFps; bitrate }`, `audio += { decoder; format; channelLayout; bitrate }`, `avSyncMs`, `cacheSeconds`, all `\| null`; no `hwdec` | 2 |
| R-P8 Transfer characteristics | Accept after rewrite | `PlaybackStats.video.transfer: string \| null` (mpv `video-params/gamma` passed as is) | 4b |
| R-T1 External audio track | Accept | `player.addAudioTrack(path: string, opts?: { select?: boolean; ifCurrent?: PlayerState }): Promise<AudioTrack>` | 2 |
| R-T2 Select audio track | Accept after rewrite | `player.selectAudioTrack(id: number, opts?: { ifCurrent?: PlayerState }): Promise<PlayerState>` | 2 |
| R-T3 Secondary subtitles | Accept after rewrite | `subtitles.select(track \| null, opts?: { role?: "primary" \| "secondary"; ifCurrent? })`; `SubtitlesState.secondary: SubtitleTrack \| null`; shared `trackConfirmation` | 3a |
| R-T4 Subtitle loading | Accept after rewrite | `subtitles.load(bytes, opts?: { format?: "srt" \| "ass"; title?: string; select?: "primary" \| "none"; ifCurrent?; signal? })`; in round 3a `select` adds `"secondary"`; no `"vtt"` | 2 (`"secondary"` 3a) |
| R-T5 Per-line cue table | Reject + alternative | No `cues`; subtitles uses `plugin.tools.run("ffmpeg", ["-v","error","-nostdin","-i",src,"-map",`0:${track.streamIndex}`,"-f","srt","pipe:1"], { timeoutMs })`; disabled for network sources | — |
| R-T6 Track codec | Accept (with addition) | `SubtitleTrack.codec: string \| null`, `text: boolean`, `streamIndex: number \| null` | 2 |
| R-T7 Style fields | Accept after rewrite | `SubtitleStyle += { bold?: boolean; shadow?: number /*0–4*/; box?: number \| null /*0–100*/; assOverride?: "keep" \| "force" }`; round 3a adds `secondaryPosition?: "top" \| "aboveMain"` | 2 (secondary subtitle position 3a) |
| R-T8 Sync range | Accept | `SubtitleStyle.offsetMs` range −600 000–600 000 | 2 |
| R-V1 Hue | Accept | `{ kind: "hue"; degrees: number }` (−180–180, runtime) | 2 |
| R-V2 Sharpen | Accept after rewrite | `{ kind: "sharpen"; amount: number /*0–2*/; size: "small" \| "large" }` (unsharp); no `cas` | 2 |
| R-V3 Denoise | Accept after rewrite | `{ kind: "denoise"; lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` (hqdn3d, 0–20); no `temporalDenoise` | 2 |
| R-V4 Deblock/deband | Accept | `{ kind: "deblock"; strength: "weak" \| "strong" }`, `{ kind: "deband"; threshold; range }` | 2 |
| R-V5 Flip/rotate | Accept after rewrite | `{ kind: "transform"; rotate: 0 \| 90 \| 180 \| 270; flipH: boolean; flipV: boolean }` | 2 |
| R-V6 Deinterlace | Accept after rewrite | `{ kind: "deinterlace"; when: "interlaced" \| "all"; output: "frame" \| "field" }` (fixed bwdif) | 2 |
| R-V7 Crop/pad | Accept after rewrite | `{ kind: "reframe"; mode: "crop" \| "pad"; aspect: number /*0.25–4*/; x: number; y: number /*−1–1*/ }` | 2 |
| R-V8 Frame blending | Accept (with performance threshold) | `{ kind: "frameBlend"; fps: number /*24–120*/ }`; verified in the same batch as the C-SIZE render timing change | 4a |
| R-V9 Display layer | Accept after rewrite | `plugin.video.view(): VideoView`, `set(Partial<{ aspect: number \| null; fit: "keep" \| "stretch" \| "fill"; zoom; panX; panY }>)`, single owner (a second one gets `conflict`) | 4a |
| R-V10 Render options | Accept after rewrite | `plugin.video.render()`, fields as in step 1; released only after C-GPU is done, single owner | 4b |
| R-V11 JPEG export | Reject + alternative | Export PNG, then convert with `sips` (measured 0.05 s) | — |
| R-A1 Channels | Accept | `{ kind: "channels"; mode: "stereo" \| "left" \| "right" \| "mono" \| "swap" }`; multichannel is downmixed by the core first | 2 |
| R-A2 Device list | Accept after rewrite | `player.audioDevices(): Promise<{ id: string; name: string }[]>` | 3a |
| R-A3 Select device | Accept after rewrite | `player.setAudioDevice(id: string \| null): Promise<void>` (the core does not save it across launches) | 3a |
| R-A4 Output state | Accept after rewrite | `PlayerState.audioOutput: { requested: string \| null; active: string \| null }` (no `devicesRevision`) | 3a |
| R-A5 Audio delay range | Accept | Audio `delayMs` −10 000–10 000 (multiples of 10 ms), budget widened accordingly | 2 |
| R-W1 Window state | Accept after rewrite | `plugin.window.state: { fullscreen; videoArea: { width; height } }`, `onChange`; 4a adds `pictureInPicture: { supported; active }`; capability `window.control` | 3a (PiP field 4a) |
| R-W2 Video area size | Accept | `window.setVideoArea(size: { width; height }): Promise<{ width; height; clamped: boolean }>` | 3a |
| R-W3 Always on top | Accept | `window.setAlwaysOnTop(onTop: boolean): Promise<void>` | 3a |
| R-W4 Cursor hiding | Accept | `window.setCursorAutoHide(mode: "fullscreen" \| "playing"): Promise<void>` | 3a |
| R-W5 Picture-in-picture | Accept | `window.setPictureInPicture(on: boolean): Promise<void>` | 4a |
| R-W6 App information | Accept after rewrite | `plugin.app: { readonly version: string; build: string \| null; components: { mpv: string \| null; libav: Record<string, string>; slint: string } }` (synchronous read-only, no capability needed) | 3a |
| R-U1 Runtime images | Accept (with addition) | `plugin.ui.image(path: string): ImageRef`; `PropValue = Json \| ImageRef`; release rules in 3.2 item 5 | 3b |
| R-U2 Drag and drop | Accept after rewrite | `plugin.drops.provide({ extensions: string[]; drop(e: { paths; modifiers }): unknown }): Disposable`; `Panel.onDrop(listener: (e: { paths; at: { x; y }; modifiers }) => unknown)`; capability `ui.drop` | 3b |
| R-U3 Backdrop layer | Accept after rewrite | `panels.mount("backdrop", file)`; `panels.backdrop.state: { effective: boolean } \| null`, `onChange` | 3b |
| R-U4 Popup overflow | Reject + alternative | No interface; bottom bar content can be drawn into the video area with negative `y`; "the host does not clip" is written into reference document §6.3 (see 3.2 item 3) | 2 (documentation) |
| R-U5 Pop up the main menu | Reject + alternative | No interface; `S` stays unbound | — |
| R-M1 Chapters | Accept after rewrite | `player.chapters(): Promise<{ sourceId: number; chapters: { title: string \| null; start: number }[] }>` | 3a |
| R-M2 A-B loop | Accept | `player.setLoop(loop: { a: number; b: number; count?: number } \| null, opts?: { ifCurrent? }): Promise<void>`; `PlayerState.loop: { a; b; count: number \| null } \| null` (no `remaining`, since mpv cannot give the remaining count) | 3a |
| R-M3 Mark layer | Accept | `plugin.timeline.marks`, `onChange`, `layer(): TimelineLayer`; `layer.set(marks: { time; kind: "chapter" \| "bookmark" \| "marker"; title? }[], opts?)`; capability `timeline.marks` | 3b |
| R-K1 Binding management | Accept | `plugin.bindings.list() / set(ref, inputs) / reset(ref?) / onChange(listener)`; capability `bindings.manage` | 3a |
| R-K2 Ignore default keys | Reject + alternative | No interface; hotkeys writes empty overrides with K1 and records the entries it wrote in `ignore-defaults.json` | — |
| R-K3 Left single click | Accept | Bindable input `Mouse:LeftClick` | 3a |
| R-S1 Settings page | Accept after rewrite | `plugin.settings.page(file, { id; category: SettingsCategory; title; order? }): View & Disposable & { ready }`; `plugin.settings.show(pageId?)`; capability `ui.settings` | 3b |
| R-S2 Appearance | Accept after rewrite | `plugin.appearance: { readonly theme: ThemeId; onChange(listener); set(theme): Promise<void> }`; `set` needs `ui.theme` | 3b |
| R-G1 Plugin list | Accept after rewrite | `plugin.packages.list(): Promise<PluginInfo[]>`, `onChange` (no capability needed); `PluginInfo { id; version; origin; state; error; capabilities; directory; previousVersion }` | 3a |
| R-G2 Disable/enable | Accept | `packages.setEnabled(id: string, enabled: boolean): Promise<void>` (needs `plugins.manage`) | 3a |
| R-G3 Uninstall | Accept | `packages.uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>` | 3a |
| R-H1 Basic action handover | Accept | The core registers 14 actions (including `rate-normal`); old ids in the override layer are mapped once; switched over together per §5 | 2 |
| R-H2 C-SIZE | Accept | Render size follows the video area (throttled rebuild) | 4a |
| R-H3 C-GPU | Accept (design threshold) | libmpv GPU rendering embedded in Slint; design first and confirm a Safe Rust wrapper, otherwise escalate per §7 | 4b |
| R-H4 Numpad key names | Accept | `Numpad0`–`Numpad9`, `NumpadPlus`, `NumpadMinus`, `NumpadMultiply`, `NumpadDivide`, `NumpadEnter`, `NumpadDecimal` | 2 (same change as H5, moved earlier) |
| R-H5 Option key | Accept (defect) | Merge in winit `PhysicalKey` during recognition; fix the `Ctrl+[` misreading | 2 |
| R-H6 Theme push | Accept after rewrite | Color tokens become `in property`, corner radius stays constant; the host pushes to all views | 3b |
| R-H7 Options window | Accept after rewrite | Category tree + page area (440 wide) + close; remembers `settings.lastPage` | 3b |
| R-H8 Picture-in-picture frames | Accept | Sample buffer path; first confirm the AVKit bindings are usable under Safe Rust | 4a |
| R-H9 Generated streams follow the device | Accept after rewrite | Delivered in the same round as A2–A4 (no longer in phases) | 3a |
| R-H10 Disabled persistence | Accept | The disabled set is stored in the install store catalog | 3a |
| R-H11 Single-click detection | Accept | Delay detection only when `Mouse:LeftClick` has a binding | 3a |
| R-H12 ASS override | Accept | `assOverride` defaults to `keep` (`scale`); the default behavior change goes into the change notes | 2 |
| R-H13 Loop resets across files | Accept | The core clears `ab-loop-*` when the source changes | 3a |

### 3.2 Position on the author additions

1. **R-P5 "push immediately on phase change"**: accepted, done in round 2, handled as a defect improvement (no contract change). The `backend.rs` event loop sends one `source.state` right after a source phase change (including `ready → ended`), without waiting for the 100 ms cycle; the periodic push stays as is. Acceptance: TestHost unchanged; a host unit test asserts that the first broadcast after a phase change does not depend on the cycle timer.
2. **R-T5 timeout**: agreed that the caller gives the timeout; `tools.run` has no fixed upper limit (see 3.3).
3. **R-U4 bottom bar does not clip**: accepted as part of the contract. Reference document §6.3 adds the sentence: "Content of the bottom bar panel can be drawn into the video area above the slot (negative `y`); the host does not clip; side bars and floating windows are above it." The host adds an assertion to the composition test in `composition.rs` (the bottom bar outer element has no `clip`), so future composition changes are caught by the test.
4. **R-T6 `streamIndex`**: accepted. Value rule: for embedded tracks where the host judges `ff-index` reliable (the same lavf condition as in the existing `media_tracks.rs`), give the container stream number, otherwise `null`; always `null` for external tracks and network sources.
5. **R-U1 `ImageRef` release**: the rules are as follows.
   - An `ImageRef` belongs to the scope that created it and becomes invalid once that scope is released (writing an invalid reference fails with `invalid`).
   - The host caches decoded results by (plugin, path, file size, modification time), with a decoded memory limit of 64 MB per plugin; LRU evicts entries that are **not currently referenced by any property**; an evicted reference stays valid and is decoded again on next use.
   - If the file content at the same path changes, calling `ui.image` again gives a new reference; the old decoded result of the old reference is evicted once it is no longer referenced.
   - Validation (format, 16 MB, 4096×4096, path scope) happens when the host first uses the reference; failures are reported to `onError` as property write errors.
6. **R-H1 migration confirmation**: received. The migration method is written in the round 2 task sheet §3.1.
7. **R-K2 hotkeys handling**: agreed with the record and restore rules given by the author.
8. **R-U2 scroll position sent back** and **R-U3 cover art treated by mpv as a video track**: agreed; the latter is recorded in the visualizer README as known behavior, not a defect.

### 3.3 Ruling on the general core capability candidates

| Candidate | Ruling | Final signature | Round |
| --- | --- | --- | --- |
| Clipboard | **Into the host** | `plugin.clipboard.readText(): Promise<string \| null>`; `writeText(text: string): Promise<void>`; `writeImage(path: string): Promise<void>` (PNG/JPEG, ≤ 16 MB). Capability `clipboard`. Failure codes: `invalid` (format, size), `unavailable` (headless mode), `failed` (refused by the system) | 2 |
| Prevent sleep during playback | **Into the host** | `plugin.power.preventSleep(mode: "off" \| "playing"): Promise<void>` (needs `playback.control`). Takes effect if any plugin requests `playing`; a plugin's request is revoked when it is disabled. The host holds the assertion while "the source is ready and not paused" (including generated streams): with video frames it prevents display sleep (`caffeinate -d -i`), for audio only it prevents only system sleep (`-i`), bound to the host process with `-w <host pid>`. The author pointed out that the step 1 claim "a plugin crash leaves the assertion" was wrong; corrected: the reason to put it in the core is that the "is playing" decision is in the host | 2 |
| Confirmation dialog | **Into the host** | `plugin.dialogs.confirm(options: { message: string; detail?: string; confirm: string; cancel?: string; destructive?: boolean; checkbox?: string }): Promise<{ confirmed: boolean; checked: boolean }>` (needs `ui.dialog`). Uses NSAlert (safe methods of `objc2-app-kit`: `addButtonWithTitle`, `setShowsSuppressionButton` as the checkbox, button `setHasDestructiveAction`) | 3a |
| External tool calls | **Into the runtime (SDK layer)** | `plugin.tools.run(tool: "ffmpeg" \| "ffprobe", args: string[], options: { timeoutMs: number; maxOutput?: number /*default 16 MiB*/; signal?: AbortSignal }): Promise<{ stdout: Uint8Array; stderr: string }>`; `plugin.tools.available(tool): Promise<boolean>`. Location rule `VESTRA_FFMPEG` / `VESTRA_FFPROBE` / PATH; no shell; the process ends when the scope is released; failure codes `unavailable` (not found), `timeout`, `failed` (non-zero exit, `message` is the last line of stderr), `invalid` (output over the limit) | 2 |
| Settings file read/write | **Into the runtime (SDK layer)** | `plugin.storage.json<T>(name: string, options: { defaults: T; validate?: (value: unknown) => value is T; debounceMs?: number /*default 300*/ }): JsonStore<T>`; `interface JsonStore<T> extends Disposable { readonly value: T; save(next: T): void; flush(): Promise<void> }`. The file is at `dataDirectory/<name>.json`, temp file + rename; when it is corrupt or fails validation it is renamed to `<name>.json.bad-<timestamp>` and the defaults are used; `dispose` and plugin disable flush first | 2 |
| Show / open folder | **Into the runtime (SDK layer)** | `plugin.system.reveal(path: string): Promise<void>` (selects it in Finder); `plugin.system.openPath(path: string): Promise<void>` (opens the file or folder). `missing` when the path does not exist | 2 |
| Enter-URL dialog | **Into the host UI library (no protocol added)** | `@vestra/widgets.slint` adds `UrlInputRow` (input field + OK / Cancel, spec UI-DLG-02) | 3b |
| Runtime images / settings pages | Already in R-U1, R-S1 | — | 3b |
| `sips`, system information commands, keychain, `fs.watch` | **Not done** | Stay in their plugins | — |

Runtime (SDK layer) means implemented by the SDK inside the plugin process (`src/sdk/plugin/`, provided through the `plugin` object), adding no host protocol or Rust code; plugins still only `import type` the SDK.

### 3.4 Ruling totals

67 items in total:

| Conclusion | Count |
| --- | --- |
| Reject + alternative | 6 (R-P5, R-T5, R-V11, R-U4, R-U5, R-K2) |
| Accept after rewrite | 30 |
| Accept | 31 |

Also 7 general capabilities: 3 into the host (clipboard, prevent sleep, confirmation dialog), 3 into the runtime (external tools, settings files, show folder), 1 into the host UI library (enter-URL component); 4 not done.

### 3.5 Core size estimate

Baseline: host Rust 22 770 lines (`src/host/src/*.rs`), SDK and runner about 4 800 lines (`src/sdk/plugin/*.ts`, `src/runtime/`). Budget: net host Rust growth per round no more than about 3 000 lines (about 13%); split if exceeded. Estimates include unit tests, not documentation.

| Round | New interfaces (methods / modules) | Extended types and ranges | Host behavior | New capability names | Host Rust estimate | SDK / runtime estimate |
| --- | --- | --- | --- | --- | --- | --- |
| 2 | 7: `player.step`, `setStepSeek`, `host.run`, `addAudioTrack`, `selectAudioTrack`, `clipboard.*`, `power.preventSleep`; plus 3 runtime modules `tools`, `storage`, `system` | `PlaybackStats` fields, 3 `SubtitleTrack` fields, 4 `SubtitleStyle` fields, `load` options, 2 range widenings, 8 video nodes, 1 audio node | 5: H1, H4, H5, H12, immediate phase push | `clipboard` | about 2 600 | about 1 300 |
| 3a | 15: `window.state/onChange`, `setVideoArea`, `setAlwaysOnTop`, `setCursorAutoHide`, `plugin.app`, `packages.list/onChange/setEnabled/uninstall`, `bindings.*`, `audioDevices`, `setAudioDevice`, `chapters`, `setLoop`, `dialogs.confirm` | `role` of `select`, `"secondary"` of `load`, `secondaryPosition`, `PlayerState.loop`, `audioOutput`, `Mouse:LeftClick` | 5: H9, H10, H11, H13, secondary subtitles | `window.control`, `bindings.manage` | about 2 800 | about 900 |
| 3b | 7: `ui.image`, `drops.provide`, `Panel.onDrop`, `panels.mount("backdrop")`, `settings.*`, `appearance`, `timeline.*` | `PropValue`, widgets components | 2: H6, H7 | `ui.drop`, `timeline.marks`, `ui.settings`, `ui.theme` | about 2 900 | about 800 |
| 4a | 2: `video.view()`, `window.setPictureInPicture` | `frameBlend` node, `window.state.pictureInPicture` | 3: H2, H8, render timing | — | about 3 000 (high uncertainty for picture-in-picture) | about 300 |
| 4b | 1: `video.render()` | `PlaybackStats.video.transfer` | 1: H3 | — | undecided (design threshold) | about 200 |

Conclusions:

- The five batches add about 11 000 net host lines in total (excluding 4b), about 2 800 lines per batch on average, all within budget; the original rounds 2, 3 and 4 would each have exceeded it, so they were split as above.
- Interface surface: 35 new methods and modules in total (32 host, 3 runtime), 7 new capability names (all 6 in the original appendix A kept, plus `clipboard`); none is opened for the special need of a single plugin. The rejected and narrowed items remove about 15 interfaces or type branches (`preload`, `cues`, JPEG export, `commands.popup`, `ignoreDefaults`, two `can.*`, `cas`, `temporalDenoise`, `scaleX/Y`, `themes`, `corners`, `devicesRevision`, `AppInfo.plugins`, `onDragOver`, `mode` and so on).
- Round 4b (C-GPU) is the only batch that could get out of control: both size and feasibility depend on a render wrapper under Safe Rust. Before work starts the maintainer produces a design with a line estimate; if it exceeds the budget or needs `unsafe`, the user decides per §7, and work does not start by default.
