# 2026-10 round: video primitives (plugins own their video algorithms)

## 1. Goal (user, 2026-10-31)

Plugins must be able to own their video algorithms instead of only toggling host features. Two pure SDK additions (§1.6 "additions": the maintainer's call, patch release):

1. A shader channel for many plugins. `plugin.video.render().shaders` is single-owner and the built-in video plugin takes it at activation, so a third-party plugin could never add a shader.
2. Whitelisted filter primitives: `convolution` and `lut3d` video nodes, typed, no raw lavfi strings.

User rulings that bound it: `render().shaders` keeps working unchanged; the built-in video plugin is **not** migrated (other plugins are not changed now).

Versions: SDK **1.5.39** (prepared, not tagged), host **0.5.20** (the next free patch after 0.5.18 lazy B1 and 0.5.19 GPU polish, which land on main first; this branch will be rebased onto them).

## 2. Design

### 2.1 Shader channel `plugin.video.shaders(id?)`

- **API**: `plugin.video.shaders(id = "shaders")` → `VideoShaders { set(paths), clear(), status, onStatus, dispose }`, like a `VideoFilter`: `set` sends the handle's whole list (last write wins, one request in flight), `clear()`/`dispose()` withdraw it, nothing is sent before the first `set`. One handle per id per plugin; id 1–128 bytes. Wire: `video.shaders.set { id, paths }` (`[]` removes); push `video.shaders.status { id, state: "failed", error }`.
- **Order** (documented in `VideoShaders` and the architecture §4.5): mpv's `glsl-shaders` = the render options' own `shaders`, then every channel handle's list sorted by (built-in first, then installed plugins by first-install time, then development loads; ties by manifest id; a plugin's handles by id). The rank comes from the host (`Host::install_rank`), never from the plugin.
- **Limit**: at most 16 shaders across all channel handles; a set that would exceed it fails with `limit` and changes nothing. The render options keep their own 16 (counting them in would have made `render().set` refuse lists it accepts today: a behaviour change).
- **Paths**: the plugin's own files only: absolute, inside its package directory or data directory after following links (`dunce::canonicalize`, then `starts_with` a canonical root; the same rule as `ui.image`), regular files ≤ 16 MiB. Outside: `invalid`; absent: `missing`. Checked on the main thread when the request arrives (`plugin_files::shader`), so a plugin cannot name an arbitrary path through the API. This is not a sandbox: the check holds at request time only (mpv/FFmpeg read the file later, so a plugin can swap it for a link or use a hard link afterwards), and plugins are unsandboxed Node processes anyway.
- **Revocation**: `owner.disabled` and `owner.parked` remove the owner's lists and rewrite `glsl-shaders`; `owner.resources` reports "video shaders" (a parkable plugin holding shaders stays live). If mpv refuses the shorter list, `set_channel` writes the old one back; the channel then keeps the owner's entries as stale (matching mpv), reports and blames nothing for them, and every later write (another `set`, another `disable`) leaves them out again (review L1).
- **Compile errors**: mpv logs the compiler's line but not the file. The message form comes from everyone whose shaders are on now (the render owner if `render().shaders` is non-empty, plus each live handle with paths): one holder, mpv's line as before; several, "the player cannot tell whose failed" with the count. Only holders not yet told for their current list hear it (once per list per holder). So a good list joining a broken one, or a channel owner joining a broken `render().shaders`, is never blamed with certainty (review M1).
- **GPU only**: in CPU mode a non-empty list is `unsupported` (clearing is always accepted); a GPU → software fallback pushes `unsupported` to every handle with shaders and clears every list (they no longer count against the 16 or keep the plugin resident); a plugin sets them again once the player draws on the GPU (after a restart) (review L2). Shaders act at draw time: never in snapshots or exports.
- **Capability**: none new; `processing.video`. Justification: the shader list was already reachable through `render().shaders` under `processing.video` (its store label is "Process the picture (filters, shaders)"); the channel only lets several plugins do the same, with a stricter file rule (plugin-owned files instead of any existing file); it touches no data, changes only the drawn picture while the plugin runs, and is revoked when the plugin stops. A separate capability would add a store line with no new risk to explain.

### 2.2 `convolution` node

`{ kind: "convolution", planes: (ConvolutionPlane | null)[] }`, 1–4 entries (plane 0–3 of the decoded format; YUV: luma, Cb, Cr, alpha), at least one non-null. `ConvolutionPlane { mode: "square" | "row" | "column", matrix: number[], rdiv, bias }`:

| Field | Rule |
| --- | --- |
| `matrix` | integers −1024…1024 (FFmpeg reads whole numbers: `0.5` became `0`, measured); square: 9, 25 or 49 entries; row/column: odd count 1–49 (FFmpeg refuses even counts) |
| `rdiv` | 0–256; 0 = 1/sum (1 when the sum is 0), FFmpeg's default |
| `bias` | 0–65535 (FFmpeg's minimum is 0) |

Unknown fields, string matrices and unknown modes are refused (`deny_unknown_fields`); graph: `convolution@nN=0m='…':0rdiv=…:0bias=…:0mode=…` for each given plane.

### 2.3 `lut3d` node

`{ kind: "lut3d", file, interpolation: "nearest" | "trilinear" | "tetrahedral" | "pyramid" | "prism" }`. The file: a `.cube` (by the canonical name: FFmpeg picks the parser by extension) with `LUT_3D_SIZE` 2–65 before the data (1D cubes refused), ≤ 16 MiB, in the package or data directory (`plugin_files::lut`); the capability layer rewrites `file` to the canonical path before the worker sees it. Graph escaping: the path is quoted for FFmpeg's option parser (`'…'`, a `'` written `'\''`) and backslash-escaped for its graph parser (`\ ' , ;`); `[` and `]` are refused because the player wraps the graph in mpv's `lavfi=[…]`, which ends at an unbalanced bracket. Tested with a directory named `O'Brien, looks; v2` in mpv (playback) and FFmpeg (offline).

Both nodes go through the existing chain: frames in memory (copy-back decoding, as for every non-player node), the same graph for processed snapshots and exports, revoked with the chain. The file must stay where it is while the chain uses it (it is read again for each source, snapshot and export).

### 2.4 Limits

`VIDEO_PROCESSING_LIMITS` (SDK, `@vestra-player/sdk/testing`; values in the SDK site's Reference: Limits, _Video primitives_) repeats the host constants `video_shaders::{MAX_SHADERS, MAX_ID}`, `plugin_files::{MAX_SHADER_BYTES, MAX_LUT_BYTES, MAX_LUT_SIZE}` and `effects::{MATRIX_VALUE, MAX_RDIV, MAX_BIAS, MAX_LUT_PATH}`; `tests/sdk/host-agreement.test.ts` checks that they agree.

## 3. Hardware cases for the coordinator (≤ 60 s each, one at a time)

Test plugin: `src/tests/hardware/video-primitives-probe/a` and `/b` (ids `test.video-primitives-probe-a` / `-b`; the same `main.ts`, shaders and LUTs, keep them identical). Load one or both as development plugins, for example `vestra-host --plugin src/tests/hardware/video-primitives-probe/a --plugin src/tests/hardware/video-primitives-probe/b <clip>` (the installed player's `--plugin` takes the same paths). Each adds Video ▸ Primitives Probe A / B with: Convolution Sharpen, Convolution Blur 5x5, LUT Warm, LUT Invert, Clear Filter, Shader Tint (Red), Shader Invert, Shader Broken, Clear Shaders, Processed Snapshot (a PNG in the plugin's data directory, path on the OSD). Copy A also has keys Ctrl+Alt+1 … 9, 0 in that order; copy B is menu only. Every status change goes to the OSD and to the log as `video-primitives-probe A|B: …`. Development loads sort after installed plugins, A before B (by id), so in H4 A's shader runs first. H8 needs a `render()` owner with a broken shader: add the probe's `shaders/broken.glsl` to the built-in Picture Adjustments plugin's pixel shader list (it copies the file into its own data directory) and turn pixel shaders on, or a scratch plugin that calls `plugin.video.render().set({ shaders: [that file] })`.

| # | Mode | Action | Expected |
| --- | --- | --- | --- |
| H1 | CPU (default on Windows) | `plugin.video.filter("k").set([{ kind: "convolution", planes: [{ mode: "square", matrix: [0,-1,0,-1,5,-1,0,-1,0], rdiv: 1, bias: 0 }] }])` on a 1080p clip; then a 5×5 box blur (`matrix` 25 × `1`, `rdiv: 0`) | the picture visibly sharpens, then blurs; `status` `applied`; playback continues |
| H2 | CPU | with H1's blur on, take a processed snapshot (`plugin.media.capture({ processed: true })`) | the snapshot is blurred like the picture |
| H3 | CPU | `lut3d` with a `.cube` from the plugin's `looks/` (for example an inverting 2-point LUT or a warm grade), `interpolation: "tetrahedral"` | the picture is recoloured; a path outside the plugin fails `invalid`, a missing file `missing` |
| H4 | GPU (Video ▸ GPU Rendering (Experimental) on, restarted) | two development plugins A and B each `plugin.video.shaders().set([own .glsl])` (A: a visible tint, B: an invert, both `//!HOOK MAIN`) | both apply; mpv's `glsl-shaders` (log or `VESTRA_FRAME_LOG`) lists A's then B's file when both are development loads (order by manifest id), built-ins first |
| H5 | GPU | stop plugin B (disable it in the store or quit its process) | B's shader goes, A's stays; `glsl-shaders` holds only A's file |
| H6a | GPU | A sets a broken shader alone; then B sets a good one | A: `failed` with mpv's plain line; when B joins and mpv recompiles, B hears `failed` with "cannot tell whose failed" (never the plain line); A is not told twice |
| H6b | GPU | B sets a good shader first; then A sets a broken one | A and B both hear `failed` with "cannot tell whose failed" (2 handles) |
| H7 | CPU | `shaders().set([file])` | `failed` with `unsupported` |
| H8 | GPU | the built-in Picture Adjustments plugin's `render()` given a broken shader (or a dev plugin owning `render()` with one), then a channel plugin sets a good shader | the render owner: `failed` with mpv's plain line; the channel plugin: `failed` with "cannot tell whose failed" |
| H9 | GPU, then a run-time fallback to software (`window-fallback`: a window that cannot show GPU frames) | two plugins with shaders on when the player falls back | every handle hears `unsupported`; `glsl-shaders` is empty; `clear()` succeeds; a `set` with files is `unsupported` until GPU rendering is back |

## 4. Docs pages to correct (SDK `docs/`, restructured on another branch; not edited here)

The statement "only macOS renders on the GPU / Windows has no GPU renderer" is out of date since host 0.5.14 (Windows: GPU rendering, off by default, Video ▸ GPU Rendering (Experimental)):

- `docs/reference/platforms.md` rows _App.system()_, _Rendering_, _plugin.rendering_ and _Render options_: the Windows column still says software / `no-gpu-backend` / `unsupported`.
- `docs/how-to/gpu-rendering.md` line 85: "`no-gpu-backend`, Windows today".
- `docs/concepts/processing.md`: add the shader channel (a row in the table at the top and a section after _Render options_) and the `convolution`/`lut3d` nodes.

Edited in `docs/` anyway because `docs:check` requires generated blocks to be current: `docs/reference/limits.md` (a new _Video primitives_ section with the `VIDEO_PROCESSING_LIMITS` generated table; the `MAX_SHADERS` row's generated text), `docs/reference/capabilities.md` and `docs/reference/manifest.md` (generated rows only).

## 5. Status

| Item | State |
| --- | --- |
| Host 0.5.20: `effects.rs` nodes, `plugin_files.rs`, `video_shaders.rs`, `video_render.rs` single writer, backend routing and revocation, capability layer | implemented, unit tests; hardware cases §3 pending |
| SDK 1.5.39: types, `plugin.video.shaders`, TestHost, limits, CHANGELOG | implemented, tests; not tagged |
| Agreement: conformance fixture (convolution, lut3d), `VIDEO_PROCESSING_LIMITS` test, capabilities.json | updated |
| Review (fix first): M1 message form, L1 stale entries, L2 fallback clears, L3 SDK id control characters, L4 `lutPathBytes` limit, L5 honest wording, L6 note below | fixed |

## 6. Follow-ups

- **UI thread work** (review L6): `processing.replace` with `lut3d` nodes and `video.shaders.set` run `dunce::canonicalize`, a `stat` and (for `.cube`) a read of up to 64 KiB per file on the main (UI) thread, in the capability layer that holds the plugin's roots, like `ui.image`. Cheap on local disks; if it shows up in frame logs (network drives, many files per request), move the check to a worker thread and answer the request from there.
