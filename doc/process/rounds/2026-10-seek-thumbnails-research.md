# Research: thumbnail preview on seek-bar hover and drag

Researcher report, 2026-10-26. Read-only; no code was changed. Sources:

- Our code: `C:\PROJECT\vestra-player` at `19fd7b6`. Paths below are relative to `src/`.
- Upstream code at these commits:
  - IINA `96f605f6`
  - mpv_thumbnail_script `682becf5`
  - thumbfast `0f711de3`
  - uosc `074b0869`
  - ModernX (cyl0) `b90403ce`
  - mpv `baa3e11d`
- Measurements: my own, on this machine (AMD Ryzen 9 7940HS, 16 threads, Windows 11), using the shipped ffmpeg `n9.0.2` from `C:\vestra\deps\ffmpeg`.

Items I could not verify are marked **[unverified]**.

## Summary

- **What we have.** I07 runs one ffmpeg process per hovered slot after the pointer rests for 80 ms. It works only for local files and is off by default.
  - On 4K long-GOP HEVC, one thumbnail takes **1.2–2.8 s**. About 0.2 s of that is just starting the ffmpeg process.
  - During a drag it does run, but in practice it shows a stale picture.
- **What IINA does.** It pre-generates about 100 keyframe-snapped thumbnails per file when the file opens. It keeps them in a disk cache and shows them on hover but **not during a drag**.
- **What thumbfast does.** It keeps a second mpv running and seeks it on demand: first to a keyframe, then to the exact frame.
- **Recommendation.** Rework I07 into a **pre-generated, keyframe-only sprite sheet**:
  - built in the background by the bundled ffmpeg, with `-skip_frame nokey`;
  - shown by clipping one runtime image in Slint, so hover and drag need no round trip to the plugin;
  - **shown during drag**, and **on by default** for local files.
- **Network sources.** Use the sites' own sprite sheets (Bilibili `videoshot`, YouTube storyboards). That needs one small host API so the resolving plugin can hand them over.
- **Effort.** About 2–3 days in `vestra.playback` for local files, plus about 2–3 days (maintainer and two plugins) for network storyboards.

---

## 1. What we already have (I07)

### 1.1 How it works

- **Spec.** `doc/product/feature-requirements.md:455-482`:
  - "a toggle, off by default (needs ffmpeg and uses CPU)";
  - local files only;
  - a 160×90 picture above the time tip;
  - persisted as `hoverThumbnail`, default `false` (`plugins/playback/settings.ts:32`).
- **Slint side** (`plugins/playback/ui/bar.slint`):
  - `pointing: touch.has-hover || touch.pressed` (`:157`), so the tip stays up while the button is pressed.
  - `report-hover()` sends `hover-changed(fraction)` at most every 50 ms (`:223-246`).
  - The tip draws a 160×90 box with `Image { source: PlaybackBridge.thumbnail }` (`:494-517`).
- **Plugin side** (`plugins/playback/thumbs.ts`):
  - **Cache key:** a slot of `max(duration × 0.5 %, 2 s)` (`:29-37`), so at most 200 slots, and at most 200 cached pictures per source (`:12`).
  - **Cache hit:** shown at once (`:162-170`).
  - **Cache miss:** a timer waits until the pointer has rested for `SETTLE_MS = 80` (`:14`, `:172-178`). Then **one ffmpeg at a time** runs (`:216-268`):
    `ffmpeg -nostdin -v error -ss <t> -i <path> -frames:v 1 -vf scale=160:90:force_original_aspect_ratio=decrease -q:v 5 -y <slot>.jpg` (`:40-56`)
    via `plugin.tools.run("ffmpeg", …, { timeoutMs: 3000 })` (`:233-237`).
  - **Cache location:** `dataDirectory/thumbs/<sha1(realpath + mtime)>/`, emptied at start and when the source changes (`:98-100`, `:191-197`, `:204-214`). Nothing survives a restart or a file change.
  - **When it is active:** local paths only (`isLocalPath`, `:152`), with video (`:134`), and fewer than 5 failures in a row (`:22`, `:132`).
  - **Wiring:** `bar.ts:136-137` routes `hoverChanged` / `hoverEnded`, and `bar.ts:86-88` binds `hoverThumbnail`, `hasThumbnail` and `thumbnail`.

### 1.2 Does it need ffmpeg? Yes, but packaged players ship it

- `plugin.tools.run` finds ffmpeg through `VESTRA_FFMPEG`, then `PATH` (SDK `llms-full.txt:6234-6260`).
- A **packaged** player sets `VESTRA_FFMPEG` to its bundled ffmpeg for every plugin process (`host/src/runtime_paths.rs:5-18`, `:140-165`).
- The installers copy ffmpeg in:
  - Windows: `packaging/bundle-windows.mjs:88-89`, from the BtbN gpl-shared build in `packaging/deps.json:25-29`;
  - macOS: `packaging/bundle-macos.mjs:94`, `:121`.
- So "Needs ffmpeg" (`thumbs.ts:10`, `:114-122`) only happens in development builds or a broken install.
- The spec's main reason for off-by-default, "needs ffmpeg", is therefore out of date for end users. The other reason, CPU cost, still holds (§1.4).

### 1.3 Does it work while dragging?

**Partly. It asks for thumbnails during a drag, but a moving drag almost never gets a fresh one.**

- **The request does happen.** `pointing` includes `touch.pressed` (`bar.slint:157`), so `hover-changed` keeps firing during a drag (`:223-233`). The drag itself goes through `dragging(true)` / `seek-changed` (`bar.slint:304-317`, `bar.ts:156-164`).
- **But the settle timer resets on every move.** Every new uncached slot restarts the 80 ms timer (`thumbs.ts:172-178`). During a continuous drag the pointer crosses a 0.5 % slot every few pixels, so ffmpeg starts only when the drag pauses.
- **Then the result is late.** It arrives after the times in §1.4. Until then the box shows the *previous* picture (`thumbs.ts:173`, "the previous picture stays until this one is ready") under the *current* time text, so picture and time disagree.
- **Two decoders compete.** During a drag the main video is also seeking, with `seek <t> absolute+exact` on every scrub step (`host/src/source_backend.rs:1463-1465`). The thumbnail ffmpeg and mpv's exact-seek decode run at the same time.
- **Why a drag thumbnail matters for us.** On 4K clips with a 10 s GOP, the main picture already freezes for about 0.6–1.5 s per keyframe step (`doc/process/rounds/2026-10-seek-keyframes.md:14-15`). Exact seeks on the same clips are slower still. A thumbnail during a drag would be the only quick visual feedback.

### 1.4 How fast is it? (measured)

**Method.** Wall-clock time of the exact I07 command, run from Git Bash. The times include process start; bash adds a few ms. Single runs unless noted. Test clips are in `C:\vestra\clips`; the 20 s clips have a keyframe every 2 s, the `-gop10` clips every 10 s.

| Clip | t = 1.9 s | t = 9.9 s | t = 19.5 s | Keyframe-only (`-skip_frame nokey`), same t |
|---|---|---|---|---|
| h264 1080p60 (GOP 2 s) | 451 ms | 380 ms | 378 ms | 297 / 333 / 228 ms |
| h264 2160p60 (GOP 2 s) | 771 | 674 | 626 | 452 / 454 / 334 |
| HEVC 10-bit 2160p60 (GOP 2 s) | 1430 | 1369 | 1150 | 551 / 626 / 324 |
| AV1 10-bit 2160p60 (GOP 2 s) | 711 | 742 | 737 | 308 / 295 / 259 |
| h264 2160p60 **GOP 10 s** | 477 | 1173 | 1100 | 344 / 368 / 378 |
| HEVC 10-bit 2160p60 **GOP 10 s** | 803 | **2261** (reruns: 2635, 2815, 2525) | **2320** | 460 / 472 (reruns: 629, 491, 439) / 2392\* |

**Other measurements:**
- **Process start alone:** `ffmpeg -version` takes 184–205 ms (5 runs). That cost is paid for every thumbnail, because I07 starts a new process each time.
- **`-noaccurate_seek` did not help** here: same or slower in every row. I did not find out why. The ffmpeg docs say it should keep the keyframe instead of decoding to the target ([ffmpeg.html, `-accurate_seek`](https://ffmpeg.org/ffmpeg.html#Advanced-options)).
- \*The 2392 ms outlier: at t = 19.5 s in a file with keyframes at 0/10/20 s, ffmpeg's default accurate seek with `-skip_frame nokey` drops every frame before 19.5 s, so it waits for the keyframe at 20 s. My inference; not traced.

**Why it is slow.**
- The comment at `thumbs.ts:39` says "`-ss` before `-i`: a fast seek to the keyframe". That is not what ffmpeg does by default.
- With input `-ss`, ffmpeg seeks to the keyframe *before* the target and then, because `-accurate_seek` is on by default, **decodes and discards every frame up to the target** ([ffmpeg.html, `-ss`](https://ffmpeg.org/ffmpeg.html#Main-options)).
- On a 10 s GOP at 60 fps that means up to about 600 4K frames per thumbnail. This explains the 2.3–2.8 s results.
- Requirement 3 of the spec (`feature-requirements.md:466`) has the same misunderstanding.

### 1.5 Why it is off by default

- **What the spec gives:** "needs ffmpeg and uses CPU" (`feature-requirements.md:457`). Its acceptance test only targets a 1080p file, "within 200 ms" (`doc/process/rounds/2026-10-features-r3b.md:223`, in an older worktree copy).
- **What the measurements add:** even the 1080p case takes about 380–450 ms on this machine, which is over that target. 4K long-GOP is 2–3 s.
- **Remaining real reasons to keep it off today:**
  - CPU spikes, one 4K decode burst per hover pause;
  - pictures that lag and disagree with the time text;
  - nothing during a drag.

---

## 2. How other players do it

### 2.1 IINA: pre-generated at open, libav in-process, disk cache, hidden during drag

- **When.** Generation starts on the first video load and again on later file loads (`MainWindowController.swift#L2598-L2601`, `PlayerCore.swift#L2201-L2204`, [link](https://github.com/iina/iina/blob/96f605f6d8c4a3ed7d5d0afac644d8681f766757/iina/PlayerCore.swift#L2907-L2949)).
- **Skipped sources:** network URLs (`!info.isNetworkResource`), and files on remote volumes unless `enableThumbnailForRemoteFiles` is on (default off) (`PlayerCore.swift#L2907-L2949`, `Preference.swift#L1126-L1129`).
- **Interval:** a fixed count, `THUMB_COUNT_DEFAULT 100`, with interval `duration / 100` (101 seek points) ([FFmpegController.m#L28, L204-L211](https://github.com/iina/iina/blob/96f605f6d8c4a3ed7d5d0afac644d8681f766757/iina/FFmpegController.m#L204-L211)).
- **Seek:** `av_seek_frame(..., AVSEEK_FLAG_BACKWARD)`. It keeps the **first decoded frame**, which is the keyframe, with no decode-forward. Duplicate keyframes are skipped, and each thumbnail records its real timestamp (`FFmpegController.m#L213-L272`).
- **Decoding:** software only, no hwaccel (`#L147-L165`), on one serial queue (`#L74-L101`).
- **Resolution:**
  - Default `thumbnailWidth` is 120 points; it is generated at 2× = 240 px wide, height from the aspect ratio (`Preference.swift#L1126-L1129`, `PlayerCore.swift#L2907-L2949`, `FFmpegController.m#L173-L201`).
  - Thumbnails are kept in memory as `NSImage`s (`#L299-L330`).
- **Disk cache:**
  - Location: `~/Library/Caches/<bundle>/thumb_cache/<md5 of path>`.
  - Format: one binary file per video, a header (version, file size, mtime) followed by `[len][realTime][JPEG q0.75]` blocks ([ThumbnailCache.swift#L18-L134](https://github.com/iina/iina/blob/96f605f6d8c4a3ed7d5d0afac644d8681f766757/iina/ThumbnailCache.swift#L18-L68)).
  - It is written only after the whole set succeeds.
  - Limit: `maxThumbnailPreviewCacheSize = 500` MiB. Above that, the oldest 50 % are deleted (`CacheManager.swift#L36-L62`).
- **Progress:** partial results are reported every 0.2 s or more, but the main-window preview waits for `thumbnailsReady`, which means all of them (`MainWindowController.swift#L3031`, `PlayerCore.swift#L3230-L3246`). Only the Touch Bar uses partial results.
- **Drag:** **not shown.** `PlaySliderCell.startTracking` hides `thumbnailPeekView` (`PlaySliderCell.swift#L175-L183`). The Touch Bar slider does show it while touching (`TouchBarSupport.swift#L366`).
- **Time to generate:** **[unverified]**; no benchmark or issue found. Estimate for our case: §3.6.

### 2.2 mpv_thumbnail_script (TheAMM): pre-generated in workers, coarse-to-fine, unmaintained

- **Status:** last commit 2020-01-16.
- **Process:** one subprocess per thumbnail, using either mpv encode mode with `--hr-seek=yes`, or `ffmpeg -noaccurate_seek -ss T -i … -frames:v 1 … rawvideo bgra` ([thumbnailer_server.lua#L9-L87](https://github.com/TheAMM/mpv_thumbnail_script/blob/682becf5b5115c2a206b4f0bdee413d4be8b5bef/src/thumbnailer_server.lua#L9-L87)).
- **Count:** default `thumbnail_count = 150`, interval clamped to 5–90 s. Files over 1 h are not auto-generated (`options.lua`).
- **Order:** coarse-to-fine, with strides 64…1, so the whole bar gets rough coverage early. A missing slot shows the nearest finished one ([thumbnailer_shared.lua#L209-L229, L305-L320](https://github.com/TheAMM/mpv_thumbnail_script/blob/682becf5b5115c2a206b4f0bdee413d4be8b5bef/src/thumbnailer_shared.lua#L305-L320)).
- **Cache:** raw BGRA files per `<name>-<size>`, never evicted.
- **Network:** off by default (`thumbnail_network=no`). When on: 60 thumbnails, 15–120 s apart, always with mpv.
- **Drag:** probably shown; no drag check in `patched_osc.lua#L846-L899` **[inferred from code, not run]**.

### 2.3 thumbfast (po5): on demand, a second persistent mpv, keyframe first then exact

- **Process:** a second mpv started on the first hover, controlled over IPC, with ([thumbfast.lua#L438-L489](https://github.com/po5/thumbfast/blob/0f711de3138c9bd6718209d819ac54022c23ded2/thumbfast.lua#L438-L489)):
  - `--hr-seek=no` (exact only for files under 30 s);
  - a 128 KiB demuxer cache;
  - `--vd-lavc-skiploopfilter=all --vd-lavc-fast --vd-lavc-threads=2`;
  - `hwdec=no` by default;
  - output to a single raw BGRA file in encode mode.
- **Seeks:** each request sends `seek T absolute+keyframes` at once. When requests stop for about 2 ticks of 3/60 s, it sends `absolute+exact` (`#L648-L679`).
- **Display:** a 1/60 s poll renames the file and calls `overlay-add` (`#L597-L714`).
- **Network:** off by default (`network=false`); when on, it uses `--ytdl-format=worst` (`#L404-L409`).
- **Defaults:** max 200×200, scaled for HiDPI.
- **Benchmarks:** none published **[unverified]**.

### 2.4 Hover-only vs drag in mpv UIs

- **uosc:** hides the thumbnail once a drag exceeds about 5 px (`Timeline.lua#L459-L506`). Instead it live-seeks the main player: keyframe seeks when the cursor moves fast, exact seeks when slow (`#L124-L175`).
- **ModernX:** draws it whenever the pointer is on the bar, so probably also during a drag **[inferred from code]**.

### 2.5 VLC and PotPlayer

- **VLC 4:** an open merge request "Timeline Thumbnail Preview" ([MR 7822](https://code.videolan.org/videolan/vlc/-/merge_requests/7822)). Its state is **[unverified]**; the site served a bot-check page. I found no official documentation.
- **PotPlayer:** forum posts describe a "Show thumbnail previews" option under the navigation/progress bar preferences ([VideoLAN forum](https://forum.videolan.org/viewtopic.php?p=424356), [VideoHelp](https://forum.videohelp.com/showthread.php?p=2254041)). There is **no official documentation**, and how it works is unknown.

---

## 3. Recommendation for Vestra

### 3.1 On demand or a pre-generated strip? A pre-generated keyframe sprite sheet

Compare a drag across the bar under each approach:
- **On demand** needs one decode per new position. At 0.3–2.8 s each (§1.4), it cannot keep up with a moving pointer.
- **A pre-generated set** makes every hover and drag position a lookup.

Proposal:
- **Timestamps:** N = min(200, duration / 2 s) points, which keeps I07's 0.5 % / 2 s granularity.
- **Seek mode:** each point snaps to the keyframe at or before it, IINA-style. Record the *actual* frame time and show that time next to the picture, or show the hover time and accept being up to one GOP off, as IINA does.
- **Order:** coarse to fine (every 64th point, then every 32nd, …), as mpv_thumbnail_script does, so a partly built set already covers the whole bar. A missing slot shows the nearest finished one.
- **Output: sprite sheets, not 200 files.**
  - Each sheet is a 10×10 grid of 160×90 tiles, one JPEG per 100 thumbnails: 1600×900 px.
  - This is the same layout Bilibili serves (§3.7).
  - Slint's `Image` can show part of one image through `source-clip-x/y/width/height` ([Slint Image docs](https://docs.slint.dev/latest/docs/slint/reference/elements/image/)). The bar can therefore compute the tile from the pointer fraction **inside Slint**, with no plugin round trip, the same way the hover time is already computed locally (`bar.slint:156-159`).
  - The plugin only sends a sheet again when more tiles are finished (every ~1 s, as IINA throttles). Hover and drag then cost zero IPC.
  - **[unverified]:** that `source-clip-*` works on a runtime `ImageRef` property in our host. Prototype this first.

### 3.2 A second libmpv/ffmpeg process or ffmpeg seeks? ffmpeg CLI through `plugin.tools.run`

- **Second libmpv in the host:** this needs host Rust work and a new API, and violates nothing, but costs more.
  - `mpv_create()` makes independent instances (`client.h#L428-L481`), but the docs do not explicitly promise that several instances in one process are supported **[unverified]**.
  - Whether `screenshot-raw` works with `vo=null` on a never-rendered handle is also **[unverified]**.
- **thumbfast-style persistent mpv process:** we do not ship an `mpv` executable, only `libmpv-2.dll` (`C:\vestra\deps\components.json`). Plugins must not run OS commands (workflow §1.5).
- **Bundled ffmpeg through `plugin.tools.run`:** already cross-platform and already used by I07. **Recommended.** Two ways to run it:
  1. **Per point, a small pool** (2 workers): `ffmpeg -skip_frame nokey -ss T -i file -frames:v 1 -vf scale=160:90… out.jpg`. Measured at about 0.3–0.6 s per 4K point, including about 0.2 s process start (§1.4), so 200 points take about 30–60 s with 2 workers. Then tile with a final cheap pass, `ffmpeg -i %03d.jpg -vf tile=10x10`, or write the sheet in Node.
  2. **Several inputs per process** (`-ss t1 -i f -ss t2 -i f …`, one output per input). This spreads the 0.2 s start over many points. **[unverified]**: not measured; worth a quick benchmark.
  - **Not recommended:** one decode pass over the whole file, as in the official `-skip_frame nokey … tile=8x8` example ([ffmpeg-filters tile](https://ffmpeg.org/ffmpeg-filters.html#tile)). It demuxes the entire file, so on a 50 GB remux the disk read dominates. On our 20 s 4K clips it took 0.46–1.05 s for 10 keyframes, which looks fine only because the clips are small.

### 3.3 Keyframe-only extraction: yes

- `-skip_frame nokey` cut the 4K HEVC GOP-10 case from about 2.5 s to about 0.5 s (§1.4). IINA does the equivalent by keeping the first decoded frame (§2.1).
- Two related fixes:
  - Drop the misleading comment at `thumbs.ts:39`.
  - Do not rely on `-noaccurate_seek`; it measured no gain here.
- **Hardware decode** (`-hwaccel d3d11va`) did not help for keyframe-only strips on these clips (0.6–0.8 s against 0.46–1.05 s software). Leave it off; IINA and thumbfast also default to software.

### 3.4 Default: on for local video files, generated lazily

- **Why on is now reasonable:** packaged builds ship ffmpeg (§1.2), and the sprite approach removes the per-hover CPU bursts and the stale pictures.
- **When to start generation:** when the pointer first enters the seek bar, or about 5 s after a file starts playing, whichever comes first. Run at most 2 workers, and pause them while a drag is in progress so the main exact seeks get the CPU.
- **Keep the menu item** so users on slow machines or battery can turn it off.
- **What this changes:** the documented default (`feature-requirements.md:476`) and `settings.ts:32`, a product decision for the coordinator and the user.

### 3.5 Show it during drag: yes

- **Why:** our main picture uses exact seeks during a scrub (`source_backend.rs:1464`) and lags by up to 1–2 s on long-GOP 4K (§1.3). The thumbnail gives instant feedback. IINA and uosc hide it during a drag, but they pause or keyframe-seek the main video instead.
- **How:** with the Slint-side tile lookup (§3.1), the existing `pointing` rule (`bar.slint:157`) already covers drag. No new callback is needed.

### 3.6 Memory and disk cost for long files

N is capped at 200, so the cost does not grow with duration; only the spacing does.

| Item | Estimate |
|---|---|
| One 160×90 JPEG at `-q:v 5` | about 2.1 KB (measured, `h_0005.jpg`) |
| 200 tiles on disk (2 sheets of 1600×900) | about 0.5–1 MB per file **[estimate]** |
| Decoded sheets in the UI | 1600×900×4 B ≈ 5.8 MB each, 2 sheets ≈ 11.5 MB |
| HiDPI variant (320×180 tiles, 3200×1800 sheet) | 23 MB per sheet; within the 4096×4096 runtime-image limit (`feature-requirements.md:4232`) |
| Generation time, 2 h 4K HEVC film, 200 points, 2 workers | about 30–60 s, extrapolated from §1.4 **[estimate, not measured on a long file]** |

- **Persistent cache.** Today I07 deletes thumbnails on every source change and at start (§1.1). Recommended instead:
  - key sheets on `realpath + size + mtime`, as I07 already does and IINA also does;
  - keep them across sessions with an LRU cap of about 200 MB in `dataDirectory/thumbs`. IINA's cap is 500 MiB.
- **Decode on the UI thread.** Runtime-image decoding runs on the UI thread, about 0.1 s for a 4096² JPEG (`doc/process/rounds/2026-10-features-r3b.md:269`, older worktree copy). A 1600×900 sheet sent about once per second during generation is acceptable; a 3200×1800 sheet should be sent less often.

### 3.7 Network streams (YouTube, Bilibili): use the sites' own sprite sheets

- **ffmpeg on a network stream:** each seek costs a ranged HTTP fetch plus a decode, and the URLs expire (Bilibili: 2 h). IINA, mpv_thumbnail_script and thumbfast all skip network sources by default (§2).
- **Bilibili** serves a ready-made sprite. A live anonymous probe today of `GET https://api.bilibili.com/x/player/videoshot?bvid=BV1GJ411x7h7&index=1` returned `code:0` with:
  - `img_x_len:10, img_y_len:10, img_x_size:160, img_y_size:90`;
  - `image:["//i0.hdslb.com/bfs/videoshot/137649199.jpg"]`;
  - an `index` array of timestamps in seconds.
  - That is exactly the 10×10 / 160×90 layout of §3.1.
- **YouTube** exposes storyboard sprite specs in the player response (`storyboards.playerStoryboardSpecRenderer`). youtubei.js surfaces them as `storyboards` **[unverified for the current youtubei.js version; see the online-video report]**.
- **The problem: plugin independence.** `vestra.playback` draws the bar, but only the YouTube/Bilibili plugin knows the storyboard, and plugins may not talk to each other (workflow §1).
- **Candidate host request** (for the plugin authors to file per workflow §4.1; I am not filing it):
  - Add an optional `preview` field to `ResolvedMedia` (SDK `llms-full.txt:6515-6533`), for example `{ sheets: [{url, headers?}], columns, rows, tileWidth, tileHeight, times: number[] }`.
  - The host downloads the sheets into a cache.
  - The host exposes the result to every plugin as read-only state, for example `player.state.preview` or `player.previewSheets()`, with runtime-image paths.
  - Playback then uses the same tile renderer for local and network sources.
  - Alternative: playback runs ffmpeg on network streams at low density (as mpv_thumbnail_script's remote mode does, 60 points). Not recommended: slow, uses bandwidth, and the URLs expire.

### 3.8 What the host needs

- **For local files: nothing new**, if `source-clip-*` on a runtime image works (§3.1). `tools.run`, `ui.image`, the bundled ffmpeg and `dataDirectory` exist today.
- **Fallback if clipping a runtime image does not work:** I07's single-image property, updated per tile (one file per tile). This costs one IPC round trip and one image decode per new slot, about 50 ms of throttling, but still no ffmpeg per hover.
- **For network sources:** the `ResolvedMedia.preview` capability above, with SDK, host and docs changes.
- **Optional:** `player.stats()` or a new field exposing the keyframe interval, to choose N or decide when keyframe snapping is too coarse (for example, intra-only or very long GOPs). Not required.

### 3.9 Effort estimate

| Work | Owner | Estimate |
|---|---|---|
| Prototype: `source-clip-*` on a runtime `ImageRef` in the bottom bar | playback author | 0.5 day |
| Background keyframe sprite generator (pool, coarse-to-fine, sheet writing, persistent LRU cache, pause during drag), replacing on-demand ffmpeg in `thumbs.ts` | playback author | 1.5–2 days incl. tests |
| `bar.slint` tile lookup, drag display, real-time label; default change | playback author | 0.5 day |
| `ResolvedMedia.preview` plus host cache and read API (SDK patch release, docs, host) | API maintainer | 1.5–2 days |
| Bilibili `videoshot` and YouTube storyboard mapping in the two plugins | plugin authors | 0.5 day each |
| Coordinator GUI check (4K GOP-10 hover and drag, 2 h file, network source) | coordinator | 0.5 day |

**Main risk:** that `source-clip-*` does not work on runtime images in our host, which makes hover and drag cost one IPC round trip per tile. After that, CPU and heat on laptops during the 30–60 s generation; mitigate with 2 workers, a lazy start, a pause during drags and the persistent cache.

---

## User ruling (2026-10-26)

Proceed with the recommendation (keyframe sprite pre-generated in the background, shown on hover **and** drag, on by default for local files). **Hard constraint: opening a video must not take longer.** Generation starts only after playback has started, never competes with the open, and pauses while the user drags. Measure open-to-first-frame before and after; a small measurement tolerance is allowed.
