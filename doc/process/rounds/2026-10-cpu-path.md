# 2026-10 round: the CPU rendering path on Windows (tearing, 4K60 at 30–40 fps, games stutter)

Started 2026-10-31. API maintainer, worktree `C:\vestra\wt-cpufix`, branch `cpu-path-fixes` (from main bcada4e, host 0.5.14). Status: F1, F3 and F5 implemented (host 0.5.15), static checks and tests pass; hardware confirmation by the coordinator is open (see "To measure on hardware"). F2 and F4 change behaviour and wait for the user.

## The report

The user installed 0.5.12 with the stop-gap `SLINT_BACKEND=winit-software` (Slint's software window renderer, now chosen automatically on AMD 26.10.1 drivers) and an external 4K monitor:

- tearing while dragging over the seek bar's thumbnails;
- 4K60 video plays at 30–40 fps;
- other games stutter while the player runs.

## Research findings (read-only, 2026-10-31)

- Per-frame path at 4K (10-bit source): `d3d11va-copy` read-back (24.9 MB) → mpv software render with zimg to RGB24 (thread pool = logical cores − 1) → `Image::from_rgb8` (zero copy) → Slint's software compositing of RGB8 into XRGB32 on the UI thread, single-threaded (33 MB) → softbuffer `BitBlt` per damage rectangle (33 MB) → DWM. Four full-frame CPU copies, about 13 GB/s at 60 fps (estimate), sharing memory bandwidth with the 780M.
- Tearing: GDI presentation has no vsync, and softbuffer blits each damage rectangle separately; while dragging the thumbnail, the video, the thumbnail and the bar are separate rectangles. The host's vertical-blank gate only aligns the start of the blit. Not proven; only an external 4K monitor can confirm it.
- 4K60 at 30–40: the single-threaded 4K composite plus `BitBlt` scales with the window's pixels, so `VESTRA_RENDER_TARGET` on the internal panel cannot reproduce it (it reproduces the mpv side only).
- Game stutter: most likely zimg waking all cores 60 times a second at normal priority, plus the memory traffic and DWM composing a 4K GDI window. The host changes no timer resolution, priority or MMCSS; libmpv does what mpv does (1 ms timer resolution per wait, MMCSS "Pro Audio" for WASAPI).
- Defect: on the software window path `presented` was always 0: `count_draws` fell back to `on_winit_window_event`, and `bind_window_events` later replaced that single per-window slot (in `main.rs` and in `mounts.rs` on a shell swap).
- GPU mode (M3: wgpu, flip model with vsync, zero copy) removes the CPU copies, zimg and GDI: likely fixes the tearing; the RTX 4070 measured 55–61 fps at 4K, the 780M 39–45.

Candidate fixes: F1 count presents on the software window path (bug fix); F2 cap `zimg-threads` (behaviour change, deviates from mpv's default: the user decides); F3 replace the 2 ms / 5 ms polling loops with events (no behaviour change); F4 skip software rendering while minimized or occluded (behaviour change, precedent unverified: the user decides); F5 reopen the vertical-blank clock on `Moved` / `ScaleFactorChanged` (bug fix).

## What changed (host 0.5.15)

### F1: presented frames under the software window renderer

Slint's software renderer has no rendering notifier, so `Host::count_draws` installed its own winit event hook to count `RedrawRequested`. A window has one such hook; `bind_window_events` sets the shell's hook after `attach_gpu`/`count_draws` (start-up and every shell swap), so the counting hook was always replaced and `presented`/`redraws` stayed 0.

Now `count_draws` sets `Host::winit_draws` instead, and the shell's hook counts each `RedrawRequested` while it is set (`video::count_winit_draw`, first thing in the hook, before any early return). Slint calls the frame pacing's `CustomApplicationHandler` first and skips the window's hook when the pacing holds a redraw (`i-slint-backend-winit` 1.18.1 `event_loop.rs`), so only redraws that are really drawn count, also when the pacing clock failed and on systems without pacing. The femtovg (notifier) and wgpu (`frame_pacing::on_draw`) paths are unchanged and clear the flag, so no draw counts twice. `render` stays `-` there (no draw times without a notifier); the pacing segment's `paint` covers the draw and the blit. Test: `host::video::tests::a_software_window_counts_its_redraws_in_the_shell_hook`.

### F3: backend threads wait for events

| Thread | Before | Now | Worst-case latency |
| --- | --- | --- | --- |
| Software render thread | `recv_timeout(5 ms)` on the update channel: about 200 wake-ups a second, playing or not | blocks on one coalescing channel (1 slot) fed by mpv's update callback, `render_target::Shared` when the request changes (UI area, display zoom; new `set_waker`) and the stop | mpv update: unchanged (it already woke the wait). A resize while paused: at once (was up to 5 ms). Stop: at once (was up to 5 ms). Nothing in this loop has a deadline of its own: mpv waits for a frame's display time inside the render call |
| Control worker | `sleep(2 ms)` after every turn: about 500 wake-ups a second | waits on `backend::Wake` (mutex and condvar; a wake-up before the wait is kept). Woken by every command (`Backend::commands` is now `backend::Commands`, whose `send` also wakes; the five call sites are unchanged), mpv's wake-up callback (`set_wakeup_callback`, new events), the stop (`Drop`) and the render thread when it leaves GPU rendering. Otherwise it sleeps until the next `source.state` push (`StatePush::next`, 100 ms) or audio device check (`Sources::device_check_in`, 1 s) | Commands, mpv events, the stop: at once (were up to 2 ms). The 100 ms state push: at its deadline. The audio device check: at its deadline |
| Control worker, while something ends without an event | `sleep(2 ms)` | still at most 2 ms (`POLL`), and any event cuts it short | unchanged: a source resolution's deadline, a cover art listing still running, a generated stream (frames, acknowledgements, failures from its own threads), the orientation being prepared from mpv properties (`Geometry::polling`), a frame or keyframe step landing (`CoreActions::pending`), a turn that took the full 32 commands |

Polls left in place, and why:

- The control worker's 2 ms poll in the cases in the last row: each reads state that no event announces (mpv properties, other threads' queues, deadlines). Turning them into events means wake-ups from the producer, the cover lister and the resolution book; that is a larger change in modules owned elsewhere for cases that are short-lived or rare (generated streams aside).
- `GpuRun`'s 20 ms check for a fallback or a failed GPU renderer (50 a second, GPU mode only): the GPU renderer's failure is read from the platform; out of this round's scope.
- `cover_listing` and `source_backend` 1–5 ms sleeps: test code (except the cover listing's `recv_timeout(wait)`, which is a real deadline).
- `producer.rs` 10 ms and 5 ms sleeps: the generated stream's own threads, waiting for a full ring or the next frame; they run only while a generated stream plays.

Tests: `playback::backend::tests::the_worker_sleeps_until_the_next_push`, `a_wake_up_is_kept_until_the_wait_takes_it`, `commands_wake_the_worker`; `playback::render_target::tests::a_changed_target_wakes_the_renderer`.

Measured (maintainer, Windows, headless `--headless --silent`, no file, `VESTRA_HWDEC=no`, per-thread context switches from `Win32_PerfRawData_PerfProc_Thread` over 2 s): installed 0.5.14 release about 2300 a second in total (threads at 975, 975, 190, 140, 27); this branch's debug build about 340 a second in total (threads at 197, 108, 36; the headless loop's own 10 ms wait is among them). Which thread is which was not identified (the counters carry thread ids only); the two threads near 1000 a second are gone. Release against debug build: a rough comparison, to be repeated on two release builds. Headless runs do not open files (opening goes through the plugins), so playback was not measured here.

### F5: the vertical-blank clock follows a moved window at once

`platform::VBlankWaiter::wait` checked which display the window is on every 64th wait (about a second at 60 Hz), so after moving the window to another display the pacing followed the old display's blanks for up to a second. Now the frame pacing's hook sets `Clock::moved` on the main window's `Moved` and `ScaleFactorChanged`, and the clock thread passes it to the next wait, which checks the window's display (`MonitorFromWindow`) and reopens the clock only when the display changed. The 64th-wait check stays for display changes without a move (the display arrangement changed). The clock is never opened every frame: a drag sets the flag many times, and each wait checks the monitor handle once.

## To measure on hardware (coordinator)

- F1: software window renderer (`SLINT_BACKEND=winit-software` or the AMD 26.10.1 automatic choice), `VESTRA_FRAME_LOG=1`, 1080p60 and 4K60 clips: `presented` and `redraws` are now non-zero, `presented` close to `shown` and to the pacing line's `counts: drawn`. Also check the OpenGL (femtovg) and wgpu windows: `presented` unchanged from 0.5.14.
- F3: playing (software rendering, 1080p60 and 4K60) and idle (no file, paused): Performance Monitor `\Processor(_Total)\Interrupts/sec`, `\System\Context Switches/sec` and the per-thread `\Thread(vestra-host/*)\Context Switches/sec` (or the `Win32_PerfRawData_PerfProc_Thread` query above) before (0.5.14) and after; expected: two threads at about 500–1000 a second disappear, idle total about 2300 → about 350. Functional: open, seek (keyboard and seek bar), frame step and keyframe step, pause/resume, resize while paused (the picture follows the new size), plugin install/uninstall, quit (no hang), a generated stream (VestraNES) still at full rate, GPU mode fallback (`VESTRA_RENDER=gpu` then a window that cannot show GPU frames) switching `hwdec`.
- F5: two displays with different refresh rates (or the external 4K and the internal panel): move the window across while playing; the frame log's `refresh` and the pacing line's blank-to-draw times settle within one or two lines (before: up to a second on the old display's phase, visible as `held`/`close` changes).
- The user's symptoms (tearing, 30–40 fps at 4K, game stutter) are not expected to change from these three fixes; F2/F4 and GPU mode address them.
