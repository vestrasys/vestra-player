# Project history and current round

This file is the project's round record: how the project got here, the decisions in force, the lessons that changed how we work, the open topics, and the current round. Process rules live in [agentic-workflow](../../../agentic-workflow/README.md); architecture lives in [doc/architecture.md](../../architecture.md).

**How this file is used.** It is the blackboard of the current round. A new piece of work adds its state and log under [Current round](#current-round). When the work ends, its lasting decisions, lessons and open topics are folded into the numbered sections, and its log is deleted. Git keeps the old text, including the full logs this file was condensed from.

"User" marks a ruling by the project owner; the rest are coordinator or maintainer rulings that still shape the product. Dates are 2026, month-day.

## 1. Timeline

| Dates         | Release                           | What shipped                                                                                                                                                                                                                                         |
| ------------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 10-20/21      | —                                 | Rebuild: core UI, menu model, plugin UI, API negotiation (67 core requests settled); basic playback in the core; OS code only in `platform/`; first `unsafe` crates (PiP, GPU)                                                                       |
| 10-21/22      | —                                 | macOS GPU rendering (OpenGL into IOSurface, software fallback) and system PiP                                                                                                                                                                        |
| 10-22/25      | 0.4–0.5.0                         | Slint 1.18.1, Plugin Store (signed index, revocation, sideload), installers (DMG, Inno Setup, portable zip), own visual identity, English + 6 UI languages, Vestra Player AI 2.x                                                                     |
| 10-25/28      | 0.5.1–0.5.13                      | Windows 4K60 (30 → 60 fps), keyframe seeks, hover thumbnails, plugin management, built-ins in one shared Node process (≈1.1 GB → ≈60 MB), session storage, render modes                                                                              |
| 10-27 – 11-01 | 0.5.14–0.6.3                      | Lazy activation (behind a flag), Windows GPU path via wgpu/D3D12 (off by default), AI upscaling experiment, video primitives, large lists on ListView                                                                                                |
| 11-02/03      | 0.6.4–0.7.1, SDK 1.6.0            | Plugin process supervisor (B0–B7): one supervisor for every plugin process, on-demand start, restart policy, memory limits                                                                                                                           |
| 11-03/04      | 0.7.1–0.7.5, SDK 1.6.1–1.6.3      | Release version split from the host version (host reused by input hash); sidebar tabs on demand with » overflow                                                                                                                                      |
| 11-07/11      | 0.7.6, SDK 1.6.4–1.7.x            | Menu redesign: command palette, shortcut hints, host-owned button strip, `adjust` slot with Picture/Audio/Subtitles/Playback panels, slim right-click menu; no AccessKit; SDK 1.7.0 layering and docs refactor                                       |
| 11-09/12      | 0.8.0–0.8.2, SDK 1.7.x            | Clip Export store plugin (capture split: built-in keeps snapshots); Ask Vestra runs commands with approval sheets; AMD Motion Smoothing plugin; VestraBili retired                                                                                   |
| 11-13         | 0.8.3–0.8.4, SDK 1.7.20–1.8.0     | SDK 1.8.0 "plan A": additions only, existing behaviour frozen; `plugin.mpv` pass-through; Clip Export Highlights (local audio model)                                                                                                                 |
| 11-14         | 0.9.0, SDK 1.9.0                  | Capability declarations abolished as behaviour switches; per-panel compilation; explicit state reads; Silence Skipper and Loudness plugins                                                                                                           |
| 11-15         | 0.10.0–0.10.1, SDK 1.10.0         | Release A of events and triggers: `plugin.trigger()`, `plugin.player.stream(fn)`                                                                                                                                                                     |
| 11-15/21      | 0.10.2                            | Core thread: plugin requests, supervisor, triggers and playback control on their own thread; the UI thread is a client                                                                                                                               |
| 11-21/22      | 0.10.3                            | Guest windows ("play in a new window", Chrome's guest model); README refresh                                                                                                                                                                         |
| 11-23         | 0.11.0–0.11.3, SDK 1.11.0–1.11.3  | Release B: `subscribe()`/`onChange()` removed, every plugin migrated; capability permission lists removed; NVIDIA motion smoothing on our libmpv with FFmpeg 9.1 (0.11.1)                                                                            |
| 11-26         | 0.11.4–0.11.5                     | Fullscreen pointer freeze on AMD-driven built-in panels fixed (a tiny click-through window keeps DWM composing)                                                                                                                                      |
| 11-26/27      | 0.11.6                            | First public release: macOS signed and notarized again, branded DMG; docs cleaned for open-sourcing                                                                                                                                                  |
| 11-27/28      | 0.11.7–0.11.12, SDK 1.11.4–1.11.5 | Picture-in-Picture under Tools; Apple Motion Smoothing on macOS: our own macOS libmpv with the VideoToolbox filter `vtfrc` (0.11.12), SDK engine `"apple"`, store plugin `zysilm.applemotion`; VestraTube reads YouTube's LockupView results (0.5.7) |

## 2. Decisions in force

### Principles and process

- **Plugins are independent; the core does not know plugins.** Shared needs become general core capabilities; nothing for one plugin's special need; no compatibility layers; nothing built ahead of a user (user, 10-20).
- **One public API** `vestra/1` for built-in and third-party plugins (user, 10-26). SDK removals need no shims or messages while the owner is the only plugin author (user, 11-21); SDK version bumps are asked, not assumed (user, 11-23).
- **First-party Rust forbids `unsafe`** except isolated crates under `src/host/unsafe/` with SAFETY comments, checked by the unsafe-policy script (user, 10-21).
- **Follow established practice** (mpv and mainstream players; Chrome, VS Code for plugin models) without asking; ask only real trade-offs (user, 10-27). An explicit earlier ruling beats precedent (user, 11-10).
- **Refactors change names only**: any behaviour difference fails, tests and TestHost are never edited to pass, and differential checks prove it (user, 11-13).
- **One change, one merge**: each reviewed change merges to main on its own; no batch branches (user, 11-11).
- **Architecture issues go to the user before merge** (coupling, new host duties, bloat); bloating features are usually dropped (user).
- **When the user is away**, the coordinator takes the recommended option, records it and continues (user, 11-16); implementing agents decide open design points with reasons (user, 11-13).
- **Remote CI**: SDK, plugins and page repos rely on remote CI; the player runs local `ci:local` and its remote CI is paused until the repository is public (user, 11-21). SDK releases never wait on macOS runner capacity (user, 11-02).
- **Every media test runs silent** (user, 11-23).
- **Round records are condensed here** at the end of each round (user, 11-03).

### Plugin platform

- **Plugin management is install, uninstall, disable and enable**; no rollback. Built-ins can be neither uninstalled nor disabled; they run in place and update with the player. The Plugin Store is the only management UI and works offline (user, 10-26). A disabled plugin stays disabled across updates (11-02).
- **Built-ins share one Node process**; trust comes from provenance, never a manifest field; a hang or unattributed crash restarts all members (user, 10-27).
- **One supervisor decides when every plugin process exists**; on-demand start for everyone, no lazy modes (user, 11-02). Limits: idle 30 s, start 60 s, stop grace 5 s, 512 MB JS heap per own process, process budget `clamp(physMiB/2 × 0.30 / 85, 3, 16)` (user, 11-02).
- **Session storage** survives reload, upgrade, crash and standby; cleared on disable, uninstall and quit; 1 MiB per plugin (user, 10-27).
- **Events and triggers** replace high-frequency streams: `plugin.trigger(name?)` → `.firesOn(condition)` → `.enter`/`.leave` → `.off()`; state through `plugin.player.stream(fn)` with `.stop()`; a stream opened in `activate()` keeps the plugin for that run (user, 11-14).
- **`when` vocabulary** may cover SDK-exposed state, never per-plugin words (user, 11-23).
- **No capability permissions or disclosure**: no permission lists, no SDK scanner, no CI capability scan; a manifest `capabilities` field is an ordinary unknown-field error. Protection is official review, the signed index and revocation; host API boundary checks stay (user, 11-16, 11-21).
- **No OS sandbox for plugins** (user, 11-03/04): plugins run in Node; no ecosystem sandboxes full Node meaningfully.
- **Large lists** live in the host and show with ListView (user, 10-31). **Sidebar tabs on demand**: no cap, » overflow; only the current tab keeps its plugin running (11-04).
- **Guest windows** are separate host processes with a temporary profile: empty plugin data, read-only keychain and packages, no store changes, never the main window (user, 11-21).
- **Plugin-supplied temporal processing** (frame interpolation) is a long-term goal, not scheduled (user, 11-23).

### Interface

- **Slint mechanisms over hand-rolled code** (ListView, std-widgets, menus); one compact Vestra widget set; Slint pinned exactly (user, 10-31).
- **No screen-reader accessibility**; built without Slint's `accessibility` feature (user, 11-07).
- **Menus → task panels**: built-in menus replaced by graphical panels in the `adjust` slot, shortcut hints and a command palette; a slim right-click menu; a host-owned button strip with » overflow (user, 11-07 – 11-11).
- **Window state** (size, position, maximized) is remembered; fullscreen is not (user, 11-07). Escape toggles fullscreen (user, 11-01, reaffirmed 11-10). On-top default is "Never" (user, 11-15).
- **No new visible UI without the user's yes** (user).
- **Languages**: English source plus zh-Hans, ja, ko, de, fr, es; plugins write no translation code (user, 10-24). **Fonts**: system UI fonts with a CJK fallback ending in bundled Noto Sans CJK SC.
- **Mouse**: a click on the video plays or pauses; double click unbound (user, 10-26).
- **No compile cache and no hide-instead-of-destroy for windows** until an experiment bounds the memory (user, 11-03).

### Playback and media

- **Basic playback actions belong to the core**, usable without any plugin (10-20).
- **Keyframe seeks** on by default, one switch, never a second exact seek (user, 10-26/27). **Seek thumbnails** on for local files, after playback starts, software decode only (user, 10-26).
- **Recording and capture are general host features** for any playing source (user, 11-07); the built-in capture plugin keeps snapshots, clip export is the Clip Export store plugin (user, 11-09).
- **Ask Vestra runs commands only**, with approval sheets; no picture effects or shaders yet (user, 11-11/12).

### Rendering

- **Windows GPU rendering ships off by default**; only Video ▸ GPU Rendering or `VESTRA_RENDER=gpu` turns it on (user, 10-31). mpv D3D11 frames in D3D12 shared textures, shown in wgpu windows.
- **One render-mode decision** with a denylist, a local override file and a crash guard (user, 10-28). Hybrid laptops use the high-performance GPU (user, 10-31).
- **The UI reaches the display rate even in CPU mode** (user).
- **macOS stays on OpenGL with a software fallback**; PiP is the system PiP on macOS only (user, 10-21).
- **AI upscaling and motion smoothing are offered by plugins** (RTX VSR in Picture ▸ Rendering; AMD, NVIDIA and Apple Motion Smoothing as store plugins) (user, 10-27, 11-12, 11-22, 11-27).
- **macOS motion smoothing is Apple's VideoToolbox frame interpolation** in our own macOS libmpv (`vtfrc`: low-latency interpolator where the size fits, else the regular one); a size is refused only after two far-behind playbacks, cleared when the mode is set again; virtual Macs are refused (user, 11-27/28: released as is, 1080p Double holds on an M2 Air, Match Display at 1080p does not).

### Packaging and releases

- Bundled Node 26; installers on GitHub Releases; macOS arm64 from 15.0, signed and notarized ([macos-signing.md](../../releases/macos-signing.md)); Windows per-user installer plus portable zip; release notes at `doc/releases/v<version>.md` (user, 10-22).
- **Decided releases go ahead without a final go**; the update check is on by default (user, 11-01, 11-12).
- **Release the player less often; extend features mainly through plugin updates** (store or built-in plugin updates); player-only fixes are batched unless a user-facing bug cannot wait (user, 11-28).
- **The macOS libmpv is built by us** (`build-macos.sh`, the Windows build's mpv commit and patches plus `vtfrc`, Homebrew's FFmpeg) in `deps.yml` on macos-15 with Xcode 26; minimum macOS stays 15.0 (11-28).
- **The release version is separate from the host version**; a release reuses a published host whose inputs hash the same (`host.json`) (11-03).
- **Tags are never moved and nothing is force-pushed**; a tag whose run fails ships as the next patch (user).
- **GPL source is published with the public release**; no written offer before it (user, 11-23).

## 3. Lessons that changed how we work

Most lessons are rules in agentic-workflow.md, with their evidence. These are not, or explain a rule:

- **Measure before believing.** Three Windows frame-rate hypotheses were refuted by data; count frames the window actually presents.
- **Predictions about plugin behaviour were wrong by a wide margin**; measure, then fix with API work, not tuning.
- **Record every half of a ruling** and what it supersedes.
- **Real-machine GUI tests catch what unit tests cannot** (lost modifier keys, flashing recomposition, driver-specific white windows, a pointer freeze below DWM that no measurement saw).
- **Framework and driver facts are checked, not assumed** (Slint popups and layout timing; AMD adapter mismatches; a display-sized window triggering a mode switch).
- **A "behaviour frozen" refactor needs a differential check**: the first SDK 1.8 plan changed behaviour through declarations that also steered start-up; a 448-state old-vs-new check later caught eight changed command states.
- **`[skip ci]` in a tagged commit also suppresses the tag-push release run**; dispatch the release workflow on the tag instead.
- **Node 26 can deadlock in `process.exit` with a GC worker**; scripts set `process.exitCode` and return.
- **A headless measurement is not the player.** vtfrc doubled 1080p with 0 drops in an offscreen harness, but in the real player (window drawing on the same GPU) 1080p Match Display and 1080p30 fell back on an M2 Air; GUI numbers (`VESTRA_FRAME_LOG`) decide.
- **CI's macOS runners are virtual Macs.** A filter calling VideoToolbox on mpv's thread hung a test for 57 minutes there; real-hardware paths need deadlines in tests and must never block mpv's thread.

## 4. Open topics

| Area       | Topic                                                                                                                                      | Next step                                                                                |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| Release    | Windows code signing                                                                                                                       | After the public release (SignPath Foundation needs a public repo and CI-built releases) |
| Interface  | Whether a hidden sidebar should stand by the current tab's plugin                                                                          | User decision                                                                            |
| Supervisor | Store shows "Running" for a plugin in standby; hang attribution inside the shared runner                                                   | `store.ts`, `shared-runner.ts`                                                           |
| Platform   | Uninstall with data removal deletes the directory inline                                                                                   | `package_manager.rs`                                                                     |
| Rendering  | Windows Auto still resolves to CPU (GPU default not decided)                                                                               | `host/rendering.rs`                                                                      |
| Rendering  | Slint upgrade for present mode, latency and surface access (upstream slint-ui/slint#14969)                                                 | Slint pinned                                                                             |
| Rendering  | `unsafe/gpu-probe` still in the tree; wgpu Vulkan/GLES backends compiled in                                                                | Remove or reduce to a test                                                               |
| Rendering  | PiP on Windows and Linux                                                                                                                   | `pip_unsupported.rs`                                                                     |
| Hardware   | Never-run cases: Intel GPUs, WARP/RDP, device loss in cooldown                                                                             | When hardware is at hand                                                                 |
| Online     | Preview thumbnails for network sources; re-resolve an expired stream URL mid-playback                                                      | Host API needed                                                                          |
| AI         | Web search for Vestra Player AI                                                                                                            | Needs rulings (paid, default off, security)                                              |
| Fullscreen | Report the AMD cursor stall to AMD; recheck the unmerged fullscreen-entry branch                                                           | Coordinator                                                                              |
| Plugins    | Browser video bridge ("play in Vestra" from websites)                                                                                      | Discuss with the user                                                                    |
| Motion     | `vtfrc-no-block`: no VideoToolbox call on mpv's threads (support query off-thread with a timeout); then reconsider refusing virtual Macs   | Reviewed branch waits for the next batched player release                                |
| Motion     | Size learning: no strike while the converter was never ready; strikes never expire                                                         | Maintainer                                                                               |
| Rendering  | macOS presenter shows ~45 of 48 frames/s in Double on a 60 Hz display                                                                      | Maintainer, measure with `VESTRA_FRAME_LOG`                                              |
| Interface  | Right-click menu shows a checkmark before Play/Pause while paused; Apple status line reads "30 → 30" until output is measured              | Interaction lead / applemotion author                                                    |
| SDK        | `no-gpu` docs should mention virtual Macs                                                                                                  | Next SDK patch                                                                           |
| Plugins    | VestraTube: Retry after a failed continuation reuses it; unread nodes in mixed pages are not logged; live test does not tell layouts apart | Its author                                                                               |
| Plugins    | clipexport highlights drag test flaky on the macOS runner (PR #83)                                                                         | Its author                                                                               |

## 5. Superseded

Kept so that old commits and comments make sense:

- **Lazy activation** (`VESTRA_LAZY`, `lazy.rs`) → the supervisor (0.6.7–0.7.0).
- **Plugin Manager plugin and rollback** → the Plugin Store, no rollback. **Seeding built-ins** → built-ins run in place.
- **"No GPU rendering on Windows main"** → on main, off by default. **D3D11 → OpenGL bridge and a Vulkan path** → wgpu/D3D12.
- **Fullscreen keys Enter, Alt+Enter, F11** → Escape only.
- **Gapless playback (preload)** → "fast advance". **The "classic" theme** → removed.
- **Bottom-bar buttons contributed per plugin (eight)** → host-owned strip with » overflow. **A sidebar tab cap** → no cap.
- **SDK patch bumps by default** → bumps asked per release (minor bumps 1.7–1.11).
- **Capabilities as declarations, then as a code scan with disclosure (SDK 1.8.0)** → removed entirely (11-16, Release B).
- **`subscribe()`/`onChange()`** → `plugin.player.stream(fn)` and triggers (SDK 1.11.0).
- **Core loop on Slint's event loop** → the core thread (0.10.2).
- **Player repository private until release** → public with 0.11.6.

## Current round

### State

| Item               | State                                                                                                             |
| ------------------ | ----------------------------------------------------------------------------------------------------------------- |
| macOS signed build | 0.11.6 signed and notarized; tested OK on 11-27                                                                   |
| Release 0.11.6     | Next: publish the GitHub release                                                                                  |
| Open-sourcing      | Next: make vestra-player and vestra-player-page public, then enable branch protection and remote CI on the player |
| Download page      | Next: deploy vestraplayer.com                                                                                     |
| Windows signing    | After the above                                                                                                   |

### Open items

- Release 0.11.6.
- Make vestra-player and vestra-player-page public.
- Enable branch protection and CI on the player repository.
- Deploy vestraplayer.com.
- Then Windows signing.

### Log (newest first)

- 11-28: Apple Motion Smoothing project (loop) done: player 0.11.12 with our macOS libmpv (`vtfrc`), SDK 1.11.5, store plugin `zysilm.applemotion` 0.1.0; VestraTube 0.5.7 side lane; promo takes recorded on the Mac for the Windows coordinator. Lasting decisions, lessons and open topics folded into §1–§4.
- 11-27: macOS signed 0.11.6 tested OK; history condensed before open-sourcing.
