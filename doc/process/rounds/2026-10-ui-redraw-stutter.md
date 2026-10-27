# 2026-10 round: video stutter while the UI redraws (Windows, software rendering)

Started 2026-10-26. API maintainer, worktree `C:\vestra\wt-prevstutter`, branch `preview-stutter` (from main eae5f48). Status (2026-10-27): cause found and confirmed in the GUI (the AccessKit tree rebuild per hover `if` in the context menu, see "Final measurement"); the host side of this branch (pacing linger, frame-log diagnostics, host 0.5.2) is ready for the independent reviewer; the menu fix lives on `ui-fixes`; the upstream issue text and the SDK doc patch are below and in the maintainer's report.

## The report

Windows, software rendering (host 0.5.1), main window about half of a 2560×1600 display at 200%: moving the pointer quickly over the seek bar (the hover thumbnail keeps switching) or over the right-click menu (the lit row changes) makes the video stutter. A still pointer, or no UI change, plays smoothly.

## Hypotheses checked

| | hypothesis | result | evidence |
| --- | --- | --- | --- |
| a | each `pointing` flip re-sends the bar's props and the player decodes the 1600×900 sheets again | **no** | the SDK's `ViewBase.flush` sends a bound property only when its JSON changed (`@vestra-player/sdk/dist/plugin/ui.js`); the sheet's wire value (`{$vestraImage, ref}`) does not change on a flip |
| b | image props are not deduplicated by file | **no** | `ui_image::Images::load_within` caches decoded pixels by (plugin, path, size, mtime, maxSide) |
| c | sheets re-sent every second while generating | **yes, by design**, and each re-send decodes on the UI thread | `thumbs.ts` rewrites the dirty sheets at most once a second and hands out a new `ui.image` reference, which the player must read again; decoding a real 1600×900 sheet (JPEG q3, 77 KB) with the host's `image` 0.25.10 in release: **p50 11.5 ms, 7.9–16.3 ms**, on the UI thread |
| d | `source-clip` on a large source rescales the sheet or misses the texture cache every frame | **no**, but the preview `Image` is an `if` element | femtovg caches the texture by image identity (`i-slint-renderer-femtovg-1.18.1/images.rs` `TextureCacheKey`); the clip is a draw-time rectangle. The box is `if … seek-bar.pointing: Rectangle` and the picture `if preview.cell >= 0: Image`, so each time the pointer enters the bar, or crosses from sheet A to B, the element and its texture are made again (`Texture::new_from_image`, about 4 ms for 1600×900 after Appendix B.3 of the render round) |
| e | a moved tip or lit menu row makes a large dirty region | **not applicable** | femtovg repaints the whole window on every draw anyway; the UI's own draw is 1–2 ms |
| f | pointer moves flood the plugin RPC | **no** | `pointer-fraction`, the hover time and the cell are computed in Slint; only `pointing` (enter/leave) reaches the plugin; the menu's hover goes to the host loop as one event per row change |
| g | a UI-only redraw re-uploads the video texture | **no** | `vestra-video-texture` uploads only when `Feed::put` got a new picture; the GPU branch's `View::copy` skips a frame it already copied (sequence unchanged) |

### What was found instead: the pacing stopped between pictures

`frame_pacing.rs` (Appendix B.5 of the render round) draws the main window once per vertical blank while its clock runs, and stopped the clock at the first blank that had nothing to draw. With a video playing the clock mostly ran, but every refresh without a new picture (jitter, 24 or 30 fps content, mpv's arrival phase) stopped it. A UI change in the next refresh (the lit menu row, the tip following the pointer) then drew **at once**, off the display's phase; the next picture owed a draw at the blank, which came right after: two draws in one refresh, the second blocking in `SwapBuffers` until the following blank, so the UI thread was unavailable for a whole refresh and the picture after that one waited.

Fix (host 0.5.2): the clock keeps running for `LINGER` = 100 ms after the last draw (longer than the frame interval of any video down to 15 fps). During playback every draw, also one for a UI change, waits for the blank. An idle window still draws its first change at once. Test: `frame_pacing::tests::a_ui_change_between_pictures_waits_for_the_blank` (24 fps pictures with a hover change in each refresh: 25 draws, never two within one refresh; fails on the old gate).

Visible latency does not change: femtovg swaps with `SwapBuffers` at interval 1, so an eager draw appeared at the next blank anyway; only the CPU work moves to after the blank. Input handling is not held, only the draw request.

## Coordinator measurements (2026-10-26/27, Windows, AMD 780M, 60 Hz, `VESTRA_FRAME_LOG=1`)

- First series (machine memory-starved: 0.3–1 GB free, a backup and a game running): with the software build, whenever the UI redrew at 11–19 draws/s (bar shown, time label ticking, pointer over the window) `produced` and `presented` fell to 8–16; with 0 redraws 60/60 at once. Menu sweep 40 moves/s: presented 9–15, tick max 2–4 ms. GPU build (`wt-gpu`): menu sweep produced 60, presented 52–55.
- Second series (quiet machine), stage-sf (main) and stage-prev (this branch), 4K60 H.264, window maximized (target 2000×1368) and 1292×770, cached thumbnails and a fresh profile generating them: **all phases 60/60**, redraws 0 (away / pointer still with the bar shown / menu sweep 40 moves/s / seek-bar sweep). The coordinator's reading: the first series was memory starvation, not a UI→video coupling. The user's menu-hover stutter in normal use remains to be reproduced with the new counters.
- One second of the seek-bar sweep on the fresh profile: presented 56, **tick max 72 ms**: a UI-thread stall while hovering the bar during generation (see the playback findings).
- stage-prev, maximized: `mpv: calls 60 … took max 17.9 mean 16.1 ms dropped 0 delayed 0`; `pacing: moves 30 held 58–60 eager 0 close 0–1 refused 53–119 skipped 0 late max 0.2–8.5 ms paint max 8.8–9.6 mean 5.0–7.5 ms`; UI tick max 11–13 ms (stage-sf: 2–3 ms).

### Reading those counters

- **`took` 16 ms is not load.** mpv waits inside `mpv_render_context_render` until the frame's display time (`render.h`, `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`, default 1), so the call takes about one frame interval whatever the work. The line now also prints `work` (the call less that wait, from `MPV_RENDER_PARAM_NEXT_FRAME_INFO`'s `target_time`). Measured headless with the host's options and the wait off (scratch program on `rsmpv-sys`, below): the real work is 4–6 ms and nearly independent of the target size.
- **`eager 0 close 0–1`**: the pacing fix works as designed in these phases (no draw off the blank).
- **`refused 53–119`** is the designed one-refresh hold of Appendix B.5, not a loss: a picture that arrives after the blank but before that blank's draw ran (`late` plus the draw) is kept in the frame slot and set right after the draw, for the next blank. The waker's attempt and each 16 ms tick's retry both count, so a picture that waits once counts about twice. mpv's frames are audio-timed and drift through the display's phase, so the count moves between 0 and about 120 a second over tens of seconds; `presented 60` shows nothing is lost. Cost: the picture's latency alternates between one and two refreshes with that phase. Unchanged from main; only counted now.
- **`paint` mean 5–7.5 ms** is Slint's draw plus `SwapBuffers`; the draw itself was 1.4–1.5 ms in B.6, so most of it is the swap waiting for the blank or the driver. It runs on the UI thread, once per refresh.
- **UI tick max 11–13 ms on this branch against 2–3 ms on main** is not explained by the code: the tick body gained only atomic counter reads and one longer log line a second, and during 60 fps playback the pacing runs identically on both branches (the clock never stops in either). Candidates: the stderr write of the longer line through the measurement's pipe, or run-to-run variance (the runs were not back to back). To settle: run stage-sf and stage-prev alternately in one session, twice each, same clip and window, and compare `tick max` of the steady seconds; also a stage-prev run without `VESTRA_FRAME_LOG` cannot show it, so compare `tick mean` too, which was not reported.

### Headless: where the software renderer's time goes (2026-10-27)

Scratch program (`rsmpv-sys`, `vo=libmpv`, the host's `vf` and `hwdec`, `ao=null`, blocking off), 10 s per run after 2 s warm-up, the render call timed, Windows test machine (Ryzen 9 7940HS, AMD 780M):

| clip | hwdec | target | fps | render call mean / p50 / p95 / max ms | mpv dropped |
| --- | --- | --- | --- | --- | --- |
| H.264 4K60 | `auto-copy-safe` (`d3d11va-copy`) | 780×554 | 59.9 | 4.30 / 4.06 / 5.57 / 7.59 | 0 |
| | | 1292×770 | 60.0 | 3.92 / 3.75 / 5.29 / 11.21 | 0 |
| | | 2000×1368 | 60.0 | 4.62 / 4.51 / 5.81 / 7.81 | 0 |
| | | 3840×2160 | 60.1 | 5.41 / 5.32 / 6.45 / 11.41 | 0 |
| H.264 4K60 | `no` (software decoding) | 780×554 | 59.9 | 2.90 / 2.91 / 3.65 / 6.38 | 0 |
| | | 1292×770 | 59.9 | 3.08 / 2.91 / 4.21 / 9.78 | 0 |
| | | 2000×1368 | **50.7** | 13.26 / 9.12 / 36.81 / **123** | **79** |
| | | 3840×2160 | 60.0 | 5.09 / 4.97 / 6.59 / 11.78 | 0 |
| HEVC 4K60 10-bit | `auto-copy-safe` (`d3d11va-copy`) | 780×554 | 60.0 | 4.33 / 4.21 / 5.50 / 7.51 | 0 |
| | | 1292×770 | 60.0 | 4.05 / 3.95 / 5.39 / 7.45 | 0 |
| | | 2000×1368 | 60.0 | 3.75 / 3.49 / 5.29 / 12.46 | 0 |
| | | 3840×2160 | 60.1 | 5.17 / 5.05 / 6.24 / 8.07 | 0 |
| HEVC 4K60 10-bit | `no` | 780×554 | 60.0 | 2.32 / 2.22 / 3.23 / 6.50 | 0 |
| | | 1292×770 | 59.9 | 2.49 / 2.35 / 3.59 / 11.25 | 1 |
| | | 2000×1368 | 59.8 | 3.77 / 2.84 / 7.72 / 44.21 | 3 |
| | | 3840×2160 | 58.1 | 6.89 / 5.53 / 14.04 / **104** | 20 |

- **With copy-back the render call costs 3.7–5.4 ms and hardly depends on the target** (780×554 to 3840×2160: +1 ms). The cost is the copy-back and the conversion of the 4K source, not the scaling to the window. At 60 Hz the render thread has about 11 ms of headroom per frame; the GUI's `took` 16 ms was mpv's wait.
- **Software decoding (`hwdec=no`) is not cheaper overall**: the call itself is 2.3–3 ms, but decoding 4K60 on the CPU competes with everything else, with drops and 44–123 ms spikes (the H.264 2000×1368 and HEVC 4K rows). The default stays `auto-copy-safe`.
- Scaler options (`scale`, `dscale`) apply to mpv's GPU renderers, not to `libmpv`'s software output (zimg/swscale), so there is nothing cheap to change there; the ~4 ms are the floor of this path. The remaining budget question for 4K60 is the UI thread (upload 1–2 ms, draw, swap), which the GPU path of 0.6.0 removes along with the copy-back itself (Appendix D.2: 3–5% CPU against 21–26%).
- Caveat: the scratch polls `mpv_render_context_update` every 1 ms instead of waiting on the update callback; the host waits on the callback with a 5 ms timeout, which does not change the call's cost.

## Reproduced: the context menu at a 4K render target (coordinator, 2026-10-27)

`VESTRA_RENDER_TARGET=3840x2160` (the user's external 4K screen), window maximized on the 2560×1600 panel, 4K60 H.264 and AV1 10-bit, quiet machine:

| build | phase | produced | presented | counters |
| --- | --- | --- | --- | --- |
| stage-sf (main) | away, still, bar sweep | 60 | 60 | tick max 2–4 ms |
| stage-sf | **menu sweep** | 61–65 | **13–19** | tick max 2–4 ms |
| stage-prev | away, still, bar sweep | 60 | 60 | `took mean 16.0`, `paint max 13.9 mean 9.6 ms`, `held 59–60 eager 0 close 0 refused 0–73 skipped 0` |
| stage-prev | **menu sweep** | 61–62 | **10–17** | `calls 62 copied 13 updates 62 took max 24.1 mean 16.2 dropped 0 delayed 0`; `moves 19 held 7 eager 0 close 0 refused 11 skipped 42 late max 75.1 ms paint max 16.7 mean 10.2 ms`; tick max 2.5–5.7 ms |

What the counters say: at 4K the draw plus swap already takes 10 ms of every 16.7 ms refresh on the UI thread; during the menu sweep the UI thread misses 42 blanks a second and a blank waits up to 75 ms, while the host tick stays small and `paint` grows only a little. Only 19 pointer moves a second reach the window although 40 are sent: Windows coalesces `WM_MOUSEMOVE` while the thread does not pump messages, so the thread is blocked for long stretches by work that is **neither the tick nor the draw**. mpv itself keeps producing (`updates 62`, `dropped 0`); the frames are lost on the UI thread (`copied 13`: buffers held longer).

### Checked and ruled out (code)

- The menu is not a separate window or render pass: `host.slint` draws `MenuView` inside the main window under `if root.menu-open`, so a menu redraw is a normal full-window femtovg frame (femtovg has no partial redraw).
- A UI-only redraw does not upload the 4K picture again: `vestra-video-texture` uploads only in a draw that follows `Feed::put`; the `Image` item keeps its texture between draws.
- The menu's `drop-shadow-blur` is cached (`BoxShadowCache` in `i-slint-renderer-femtovg-1.18.1/itemrenderer.rs`), one image per shadow shape.
- A hover over a command row does not rebuild the menu model: `MenuAction::Hover` → `menu::hover_target` → `set_menu_path` returns early when the path is unchanged; only a submenu opening (after 400 ms) calls `refresh_menu` → `set_menu_model`, and that runs inside the tick, which stayed small.

### What a hover does that the seek bar does not: it creates and destroys items

`menu-view.slint` `MenuItem` has `if root.lit: Rectangle { … }` (the 2 px accent bar). Every hover change destroys the previous row's `Rectangle` component and creates the next one's. In Slint's winit backend each destruction calls `unregister_item_tree` → `AccessKitAdapter::unregister_item_tree` → `reload_tree()` → `build_new_tree(…)` **of the whole window** (`i-slint-backend-winit-1.18.1/accesskit.rs:223-233`, `:205-221`), posted as a closure to the event loop, and the update is pushed to UI Automation. `update_if_active` makes this a no-op unless a UIA client has attached to the window (Windows sends `WM_GETOBJECT` when one asks; on Windows 11 that can be the input experience, Narrator, Magnifier, PowerToys, or any tool querying UIA). The seek bar's tip toggles its `if` only on enter and leave and once per made cell, never per move, which fits "bar sweep fine, menu sweep not"; and at smaller targets the same extra work fits into the 11 ms the draw leaves, which fits "only at 4K". This is the leading hypothesis, not yet proven; the two things below prove or refute it.

### Instrumentation (host 0.5.2, this branch)

`ui: turn max A timers max A cursor max A pre max A mean B ms pointer max A mean B ms other max A ms` at the end of the frame log line: UI-thread time between winit's callbacks, by what started each stretch (one loop turn; Slint's timers; a pointer move; a draw request's preparation up to `BeforeRendering`; the stretches in which Slint processed the coalesced pointer moves, that is hit testing, hover changes, callbacks and the `if` toggles; other window events). Requests the gate holds are not counted anywhere (Slint does not run for them), so `paint` is the mean of real draws. Posted closures (the AccessKit rebuild, `show_frame`) count with the stretch they land in. During the menu sweep, `turn max` ≫ `paint max` with `tick max` small localizes the work; `pointer max` large points at Slint's hover processing.

### A/B build for the coordinator

`C:\vestra\stage-prev-noa11y\VestraPlayer-0.5.1-windows-x64\vestra-host.exe` is this branch built with Slint's `accessibility` feature off (`default-features = false` on `slint` and `slint-interpreter`, every other default kept; a local build, not committed). Run the same forced-4K menu sweep on `stage-prev` and `stage-prev-noa11y`:

- noa11y presents 60 while prev drops: the AccessKit tree rebuild per hover is the cause (and a UIA client is active on the machine; `Get-Process | ? { $_.Name -match 'Narrator|Magnify|TextInputHost|PowerToys' }` may name it).
- both drop the same: the cause is in Slint's own hover processing or the draw; the `ui:` buckets then say which.

### A/B result (coordinator, 2026-10-27): AccessKit confirmed, a second cost remains

Forced 4K, window maximized, 4K60 H.264, quiet machine, menu sweep:

| build | produced | presented | pacing | ui |
| --- | --- | --- | --- | --- |
| stage-prev (accessibility on) | 62–66 | **10–14** | `skipped 48 late max 123.6 paint max 21.9 mean 12.8` | **`turn max 134–176 timers max 117–160`** `cursor max 0.1 pointer max 16–22 mean 12–15` (still phase: `turn max 14.4 timers max 4.0`) |
| stage-prev-noa11y | 54–61 | **29–43** | `skipped 12 late max 30 paint max 35 mean 15.9` | `turn max 32–46 (one 101) timers max 6–10 (one 84) pointer max 28–35 mean 15–22`; after the menu closed `pointer` mean ~1 ms |

UIA clients on the machine: `TextInputHost`, `ctfmon` (no Narrator, Magnifier or PowerToys).

- **Confirmed**: with accessibility the 120–160 ms per hover lands in the `timers` segment (AccessKit's `reload_tree` is posted with `invoke_later` and runs with the timers at the start of the next turn) and disappears without the feature. The `menu-view.slint` request below stands; the `ui-fixes` maintainer has it.
- **The `pointer` number of this run overstates Slint's hover work.** Slint flushes the coalesced pointer moves right before it draws (`winitwindowadapter.rs:1282-1289`, in the same `RedrawRequested` dispatch), so the segment that carried the flush was mostly the draw and the swap: `pointer` mean 15–22 against `paint` mean 15.9. The instrumentation now splits the draw's segment at `BeforeRendering` (`pre`: the request's callback to the start of the draw, which holds the flush; `paint`: the draw and the swap), so the next run separates Slint's hover processing from the draw.
- **What did change without accessibility is the draw itself**: `paint` mean 9.6 ms with the menu closed, 15.9 with the menu swept (max 35), at the same 4K upload. That alone leaves no room in a 16.7 ms refresh. Ruled out by code: text layout is cached across draws (`sharedparley` with `TextLayoutCache`); the panel's drop shadow is cached (`BoxShadowCache`); a hover does not reset the menu model. Candidates: the per-hover create/destroy of the `if root.lit` element (also without accessibility: `register_item_tree`, the renderer's per-item graphics cache), extra GPU work the swap then waits for (20 anti-aliased rounded rectangles and the shadow over a 4K video draw), and the plain fact that frames come in during the sweep at a different phase. The `visible:` change removes the first; the next run's `render` (the draw alone, already in the line) against `paint` (draw plus swap) tells whether the rest is CPU or the GPU/swap.

### Final measurement with the draw split (coordinator, 2026-10-27, forced 4K, maximized, quiet machine, 8 s phases)

| build | phase | presented | ui / pacing |
| --- | --- | --- | --- |
| stage-prev (accessibility on) | still | 60 | `paint mean 4.4`, `render 3–5`, `pre 0.5` |
| stage-prev | menu open, pointer still | **60** | `paint 4.5`, `render 4–7`, `pre 1.4`, `timers max 2.6` |
| stage-prev | menu sweep | **8–19** | **`timers max 94`**, `pre max 3.4 mean 1.5`, `pointer max 3.4 mean 1.5`, `paint max 10.4 mean 5.3`, `render 3.9–5.0`, `skipped 47`, `late 95.6` |
| stage-prev-noa11y | still, menu open still | 60 | `paint 5.0`, `render 4–6` |
| stage-prev-noa11y | menu sweep | **60 every second** | `paint max 11.2 mean 4.8`, `pointer max 2.0 mean 1.1`, `render 3.9–5.5`, `skipped 0` |
| `ui-fixes` build (`visible:` instead of `if root.lit`, accessibility on) | menu sweep | 54–60 | nearly fixed; the residual is to be measured once `ui-fixes` and this branch's pacing fix are together |

(The `paint` means here are lower than the earlier 9.6–15.9 because `paint` now starts at `BeforeRendering`; the earlier number held the pointer flush and the request's queueing. `render`, the draw alone, is 4–6 ms at 4K in every phase.)

**Conclusion.** The stutter while the pointer moves over the context menu is entirely the AccessKit full-tree rebuild that Slint 1.18.1 runs for every element destroyed under the pointer (`if root.lit` in `MenuItem`): 94–160 ms on the UI thread per hover change, in the `timers` segment, while a UI Automation client (`TextInputHost`, `ctfmon`, present on every Windows 11 desktop with a text service) keeps the adapter active. Slint's own hover handling costs 1–2 ms and drawing the menu over a 4K picture costs nothing measurable; without the rebuild the 4K60 video presents 60/60 through a 40 moves/s sweep. The pacing linger of this branch is a separate, smaller correction (two draws in one refresh) and stays.

### Recommendation on the `accessibility` feature

Keep it on. It is what makes the menu, the bar's buttons and the plugins' controls readable by screen readers and UI Automation, and the cost is not the feature but one pattern in one component, which `visible:` removes. Turning it off would also hide the problem for every plugin author who writes the same pattern, and the player cannot know whether the user's machine has a UIA client (this one did, with no assistive tool running). The `stage-prev-noa11y` build was a diagnostic and is not for release.

**Rule for every Slint file in this repository and for plugin authors**: nothing that follows the pointer or a hover may create or destroy an element; `if` is for rare changes (a mode, a click selection, a loaded list), hover feedback uses `visible:` or `opacity:` on a permanent element. For plugin authors the rule goes into the SDK's "Common pitfalls" (`docs/concepts/ui.md`); the patch is in the maintainer's report (`sdk-ui-pitfall.patch`, a doc-only SDK patch release). In this repository the `ui-fixes` maintainer audits the host's own Slint files; the playback bar's tip is an `if … pointing` but toggles only on enter and leave (finding 1 above still applies for the texture).

### Upstream issue for the coordinator to file (slint-ui/slint)

Title: **AccessKit: every destroyed component rebuilds the whole accessibility tree; a hover `if` costs 100+ ms per change in a large window**

Body:

> **Slint 1.18.1**, winit backend with femtovg, Windows 11, Rust (`slint-interpreter`). With a UI Automation client present (on a stock Windows 11 desktop `TextInputHost.exe` and `ctfmon.exe` are enough; no screen reader), every destruction of a component calls `AccessKitAdapter::unregister_item_tree` → `reload_tree()` → `build_new_tree()` for the **entire window** (`internal/backends/winit/accesskit.rs`, 1.18.1 lines 205–233). `reload_tree` debounces several destructions in one turn (`pending_update`), but a pattern as common as
>
> ```slint
> if touch.has-hover: Rectangle { … }
> ```
>
> rebuilds the tree on every hover change. Measured in our player (a window with a video `Image`, a control bar and a context menu of about 20 rows, each with `accessible-role: button` and labels; about 10 nodes per row): **94–160 ms on the UI thread per hover change**, visible as a stall of the whole window; at 40 pointer moves a second a 60 fps video drops to 8–19 presented frames a second. The same build with the `accessibility` feature disabled, or with the `if` replaced by `visible:`, presents 60/60. Hover handling itself measures 1–2 ms.
>
> Expected: destroying a component should update only the affected subtree (remove its nodes and update the parent's children), or at least coalesce the rebuild so it never runs more than once per frame and off the hot path; a per-frame full rebuild of a few hundred nodes should also not take 100 ms, so part of the cost may be in building each node (property evaluation through the interpreter) or in pushing the update to the platform adapter synchronously.
>
> Reproduction: a window with an `Image` and a `for` of 20 rows, each `Rectangle { if touch.has-hover: Rectangle {} ; TouchArea {} ; accessible-role: button; accessible-label: "row"; }`; move the pointer across the rows with a UIA client active (on Windows 11 nothing needs starting) and time the event loop turns, or watch a 60 fps animation stutter.

### What to measure once `ui-fixes` and this branch are together

Forced 4K, accessibility on, the same four phases: pass is 60/60 in all of them with `timers max` under one refresh during the menu sweep. The residual 54–58 of the `ui-fixes` build alone should disappear with the linger (its sweep still drew hover changes at once between pictures); if not, `pre`/`pointer`/`timers` say where the rest is.

### Request to the owner of `menu-view.slint` (`ui-fixes` branch; to apply after it merges)

In `MenuItem`, replace the conditional accent bar by a permanent, hidden one, so a hover change changes properties only and never creates or destroys items:

```slint
    // The lit row: a 2px accent bar at its left edge (always present, hidden
    // when not lit: creating and destroying an element per hover rebuilt the
    // window's accessibility tree, 2026-10-27).
    Rectangle {
        visible: root.lit;
        x: 0;
        y: 5px;
        width: 2px;
        height: 14px;
        border-radius: 1px;
        background: Theme.accent;
    }
```

The other `if`s in `MenuItem` (`checked`, `radio`, `has-children`, `keys != ""`) depend on the model only and do not toggle on hover. The same rule for future work: nothing under the pointer should toggle an `if` per move while a video plays; use `visible:` or `opacity:`.

If the A/B confirms AccessKit, two further options for the maintainer, both needing a ruling: report the per-toggle whole-tree rebuild upstream (Slint 1.18.1 `reload_tree` on every `unregister_item_tree`), and whether the player should keep the `accessibility` feature on (it is what makes the menu readable by screen readers).

## Changes (host 0.5.2)

- `frame_pacing.rs`: `LINGER`; `Gate::vblank` returns `Blank::{Draw, Wait, Stop}`; per-second pacing statistics in the frame log (`pacing: moves held eager close refused skipped late paint` and the `ui:` buckets), counted in the winit hook (`new_events`, `window_event`, `about_to_wait`) and the blank handler.
- `render_target.rs`: `Stages`, the software renderer's per-second counters (`mpv: calls resize-only resized copied updates took work dropped delayed`), fed by the render thread (`backend.rs`) and the control worker (mpv's `frame-drop-count`, `vo-delayed-frame-count`, read only with `VESTRA_FRAME_LOG=1`).
- `main.rs`: both lines appended to the windowed frame log line; the stages also to the headless one.
- `doc/architecture/README.md`: the new fields and the linger rule.
- Version 0.5.2 (`Cargo.toml`, `Cargo.lock`).

## Findings for the playback plugin author (not changed here; the plugin is not in the maintainer's scope)

1. **The preview box and its pictures should be hidden, not rebuilt.** `ui/bar.slint` makes the tip `if … seek-bar.pointing: Rectangle` and the picture `if preview.cell >= 0: Image { source: cell >= 100 ? sheet-b : sheet-a }`. Each time the pointer enters the bar, and each time the cell crosses from sheet A to sheet B, femtovg creates the 1600×900 texture again (about 4 ms on the UI thread, B.3). Suggested: a permanent `Rectangle { visible: … }` with two `Image` elements, one per sheet, each `visible:` for its half of the cells. A version of this passed the plugin's 90 tests (reverted here, out of scope); `tests/preview.test.ts` line 615 matches the source text and needs updating with it.
2. **Each sheet re-send during generation decodes on the UI thread: 11.5 ms p50 for a 1600×900 JPEG**, at most once a second per sheet, only while the pointer is on the bar. Together with 1, this is the likely source of the 72 ms tick the coordinator saw once while sweeping the bar on a fresh profile (a decode, a texture, plus the ffmpeg cell batch finishing and `scope.update()` re-evaluating the whole bar binding). Options for the author: send the sheets less often while generating (every 2–3 s, or only when the pointer is still), or ask the maintainer (§4) for decoding off the UI thread, which would be an additive host change.
3. `pointing` flips do not re-send the sheets (checked, hypothesis a), so no change is needed there.

## Checks

2026-10-27: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (460 passed, 6 ignored), typos on the changed files, all clean. Release build staged at `C:\vestra\stage-prev\VestraPlayer-0.5.1-windows-x64\vestra-host.exe` (a copy of stage-sf with this exe; no plugin changed). A headless run (`--headless --silent --seconds 10` with a clip) prints the new fields but plays nothing (`host.warning open: no plugin opens …`: headless runs have no opener), so `work` has only been exercised through its unit test and the scratch measurement; the first GUI run shows the real values.

## GUI acceptance for the coordinator

Clip `C:\vestra\clips\h264-2160p60.mp4` (and `hevc-2160p60-10bit.mp4`), software rendering, window about half the screen, `VESTRA_FRAME_LOG=1`, 10 s each: (i) away, bar hidden; (ii) pointer still, bar shown; (iii) context menu open, pointer swept over the rows at 20–60 moves/s; (iv) pointer swept over the seek bar; with cached thumbnails and once with a fresh profile.

Pass: `produced` and `presented` ≥ 59.5 in (ii)–(iv) as in (i); `eager 0` and `close 0` during playback; `work` mean well under 16 ms; `skipped 0`; `tick max` without stalls above one refresh except the generation case of finding 2.
