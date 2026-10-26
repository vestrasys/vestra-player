# 2026-10 round: plugin center and UI polish

Registered 2026-10-25. **This document is the only progress source for this round.**

**Start condition**: all other 0.5.0 work is done, including i18n round 2, translation and verification of the five languages, and the host fixes. Until then this round is only registered and no work is assigned.

## The user's requirements (key points of what the user said)

- The plugin store (the interface that can show Markdown) takes over all plugin management functions; disabling, uninstalling and other actions are all done there.
- Categories such as "Installed / Not installed" belong in a tab bar.
- Entries such as repository management and manual install become one icon that opens a menu, or each item is an icon of its own.
- Each row of the plugin list has install and uninstall buttons, like VS Code, and shows the plugin's logo.
- After that, find every unreasonable part of the interface and fix it. Known examples:
  - the AI workshop buttons are too large and the layout is unreasonable;
  - the drop-down menus for the API settings on the Vestra Player AI settings page have the wrong style.
- The user's criticism: these problems should have been noticed while writing, or spotted by ourselves, instead of waiting for the user to point them out.

## Why they were missed (retrospective)

- Author agents cannot see the rendered result when writing Slint; they can only confirm that the code compiles and the tests pass.
- The coordinator's real-machine screenshots covered only the changed features, without a visual review of each interface.
- Authors did not check their work against ui-spec and the existing interfaces, which led to oversized buttons and self-drawn drop-down boxes.

The countermeasures are all in the process below; the key point is that whoever writes an interface has **looked at** the rendered result themselves before delivering.

## Process

### Scope (user ruling, 2026-10-25)

**Do not redesign the interface** (不重新设计界面). The existing interface was researched and settled; the standard is the existing [ui-spec.md](../../product/ui-spec.md). This round does only two things:

- the plugin center changes the user named (step 1), limited to the points the user listed;
- find and fix the ugly parts (step 2): code that violates ui-spec, clearly mismatched sizes and layout (for example oversized buttons), and controls whose style differs from the other interfaces (for example self-drawn drop-down boxes). The fix is to align with ui-spec and the existing interfaces; no new specification, no changes to the theme, colours or layout structure.

### Step 0: preparation (maintainer)

- **Offscreen screenshot tool**: one command renders every window and settings page of a given plugin to PNG, with a chosen language (en, zh-Hans, de) and sample data. Built by extending the existing slint-check prototype. Author agents look at the images with the Read tool.
- From now on, task instructions include one line: before delivering, look at en and de with the screenshot tool and check against ui-spec.

**Done (maintainer, 2026-10-26)**: `npm run ui:shots -- <plugin...> [--lang en,zh-Hans,de] [--out <dir>]` in `src/`; design, options and the differences from the player in [doc/architecture/ui-shots.md](../../architecture/ui-shots.md). There was no separate slint-check prototype in the repository; the tool extends the nearest thing, the recorded-props check (`scripts/ui-record.mjs` and `ui_types.rs`, [ci.md](../ci.md#recorded-plugin-props)):

- Sample data are the plugin's own tests: with `VESTRA_UI_STATES` the recorder logs every window and settings page the tests open and every value sent to them, in order, with the test's name; each view's last state per test is rendered (identical states and identical pictures once). Windows no test opens are rendered with their default values. No per-plugin sample files and no plugin changes.
- Rendering is the host's (`src/host/src/ui_shots.rs`, an ignored test the script runs once per language): `ui::compiler` with `@vestra/*` and the palette, the plugin's catalog for `@tr` and for text values, `ui_markdown` / `ui_image` / `ui_values` conversion and type check, `ui_fonts::install` (CJK fallback) and `theme::apply_fonts`, Slint's software renderer offscreen at the declared size; settings pages inside the player's Options window (640×480), plus the whole page when it is taller.
- The output (default `<system temp>/vestra-ui-shots/<plugins>`) is the PNG files and `index.md` / `index.json`: per image the plugin, the view, the language and the state.

Line for task briefs, from now on: **before delivering, look at en and de with the screenshot tool and check against ui-spec** (`npm run ui:shots -- <plugin> --lang en,de`, then Read the PNG files listed in `index.md`).

### Step 1: plugin center (merging the store and plugins plugins)

1. **The coordinator writes the product specification**, below in this document, with wireframes; it keeps the store's existing window, colours and controls and changes only what the user named:
   - one window, with tabs at the top: Installed, Available, Updates;
   - each list row: logo, name, author, version, an inline primary button in the style of the VS Code extension list (Install / Uninstall / Update) using the existing Button component; Disable goes in the row's secondary actions;
   - details (Markdown) on the right;
   - a gear or "…" icon at the top right, whose menu has repository management, manual install (sideloading) and open data folder.
2. **User ruling (2026-10-26): the Plugin Manager (`vestra.plugins`) is removed.** The store is the only place where plugins are managed. **Further ruling (2026-10-26): built-ins can be neither uninstalled nor disabled.** Users manage only third-party plugins, with install, uninstall and disable/enable. This supersedes "built-ins can be disabled" from the plugin-management round. The host refuses disable for built-ins, and existing users' disabled built-ins are re-enabled on upgrade. The store's settings text "to keep the store offline, disable the Plugin Store in Plugin Manager" must change (offline = turn off automatic update checks), and the question of the store disabling itself is moot. Disable/enable for third-party plugins moves into the store, together with anything else the manager still offers that is not install/uninstall: reload, open data folder, and the details it shows (package path, data folder, capabilities). "Settings ▸ Plugin Manager…" and the manager's settings page go away. **The store author and the plugins author** negotiate per §4 only on *how* each function moves: which row or detail action, the texts, and the migration of the manager's tests. Whether the manager stays is decided. Host or SDK capabilities they need, such as icon buttons, a tab bar component or an API to read plugin logos, are requested from the maintainer per §4.1. **Offline requirement (user, 2026-10-26): the store must work without a network.** Opening the store, the Installed tab and its details, uninstall, disable/enable, and install from a `.vxp` file must all work offline. Only Available and Updates need the repositories. Offline, they show the last known state, or a calm "offline" note, with no long spinner, no error dialog and nothing that blocks the rest of the store; retry happens on Refresh or the next scheduled check. Acceptance includes a GUI check with networking off (the coordinator disables the network adapter, or blocks the repository host) plus store tests for a failing fetch. The store author checked offline behaviour (2026-10-26, store 0.1.8) and fixed two small defects:
- installed details now fall back to the package's own README/CHANGELOG;
- Updates no longer claims "up to date" when no index was ever read.

Open items for this round:
- **Slow failure on a blackholed network.** When DNS resolves but connections hang, each repository waits 15 s (headers) to 30 s (total). Without a cache the list shows "Reading repositories…" for that long. Use a shorter timeout for the refresh on open, or show the offline note first.
- **Icons offline.** Installed plugins from a repository show the store's default icon instead of the package's own.
- **External docs.** If an installed plugin's README or changelog is on another site, the details page doesn't fall back to the local copy.
- **Unlabelled fallback.** The offline fallback shows the installed version's docs without saying so.
- **Status tone.** The offline status line uses the warning colour; it should read calmer.

Also check whether a disabled store can still be re-enabled: today the manager is the way back if the user disables the store. Propose a safe answer, for example that the store cannot disable itself, and present it to the user. Host or SDK capabilities they need, such as icon buttons, a tab bar component or an API to read plugin logos, are requested from the maintainer according to §4.1.
3. Implementation → the author checks with their own screenshots → the coordinator accepts with screenshots (real Mac) → the user looks on Windows.

### Step 2: full interface review

1. **The coordinator takes screenshots of every interface**: the main window, each menu, every page of the Options window, every plugin window and sidebar panel, in en, zh-Hans and de (the longest). On a real Mac; for Windows, ask the user for a few key screenshots.
2. **Interface reviewer** (a temporary role, an independent agent that writes no code):
   - reviews each screenshot against ui-spec and the other interfaces, records only ugly problems, and makes no "could be designed better" suggestions;
   - each problem states the screenshot, the location, which ui-spec rule it violates (or which interface it is inconsistent with), and the severity (clear error / mismatched / minor flaw);
   - the problem list goes below in this document.
3. The coordinator rules: confirm, reject, merge duplicates, then assign the confirmed problems to authors by plugin, and host component problems to the maintainer.
4. The author fixes them and attaches before and after screenshots → the reviewer reviews again → the coordinator accepts on the real machine.
5. Problems not solved within two rounds are listed for the user to decide.

### Out of scope

- No interface redesign and no new interface specification. There is only one reference: the plugin center's list rows may follow the VS Code extensions view (named by the user). No other interface uses any software as a reference (the reference research has been done; its conclusions are the current ui-spec and interface), and task instructions do not mention any.
- No changes to interaction logic or feature scope, except the parts of the step 1 plugin center the user named.
- No changes to the theme and colours. If the review finds a problem in ui-spec itself, only register it and ask the user.

## FS-BAR-1: fullscreen bar only on hover over its band (§4.1 request, playback author, 2026-10-25)

User request (exact behaviour): in fullscreen the bottom control bar is hidden and appears **only** while the pointer is over the band where the bar sits. It must not appear on pointer motion elsewhere, on key actions (seek, volume, pause, speed, ...), on entering fullscreen, or on the OSD. It hides again shortly after the pointer leaves the band. Windowed mode is unchanged.

**Requesting plugin / code location**: `vestra.playback` (owner of the `bottom` slot), `src/plugins/playback/bar.ts`, `ui/bar.slint`.

**Who controls it today (evidence)**: only the host. The plugin has no say beyond `setVisible` (the user preference) and `setInteraction` (locks that *keep* the bar).

- `src/host/src/window_state.rs` `Policy::evaluate`: in fullscreen the bottom slot is `effective` unless 3000 ms (`IDLE_MS`) passed since `last_activity_ms` and nothing suppresses it (hover, locks, menu, dialog); so **any** `activity()` reveals the bar.
- `activity()` callers in `src/host/src/main.rs`: `Event::WindowInput` (line ~814; sent by `host.slint` for pointer motion anywhere on the video, a pointer press, and every key the binding table does not take), `Event::Binding` (~920; every bound key, i.e. seek, volume, pause, the plugin's own commands), `Event::MenuAction` (~822), dialog done (~1047/1052), mounts (~2590), `ui.interaction` (~3868, any plugin lock write), and `set_fullscreen(true)` (entering fullscreen reveals the bar).
- `src/host/src/composition.rs` (~265): in fullscreen the bar is an overlay (`y: -self.height`) with `visible: root.bottom-visible`; its hover `TouchArea` is inside the invisible element, so a hidden bar has **no hover signal** at all. The video `TouchArea` in `host.slint` exports `pointer-x/pointer-y/video-hover`, which the host already samples every tick (`main.rs` ~1490).
- Cursor auto-hide (`main.rs` ~1540, UI-SHELL-09) is tied to the bar: in fullscreen the pointer hides when `bottom.reason == Idle`.

**Why the existing API cannot do it**: the plugin cannot stop the host from revealing the bar on activity (no lock *hides* it; `setVisible(false)` changes the remembered user preference and would hide the bar for good, including on hover), and a hidden panel receives no pointer events, so the plugin cannot even detect the hover. Holding a lock forever and drawing the bar transparent would break cursor auto-hide (`bottom_locked` makes `idle()` false) and still eat clicks in the band. Minimal reproduction: fullscreen, playing, pointer resting mid-screen; press Ctrl+→ → `Event::Binding` → `activity()` → `bottom-visible = true` for 3 s.

**Desired behaviour (host only, no API or SDK signature change)**, in fullscreen:

1. **Reveal only by hover over the band**. The band is the bar's own rectangle: full width of the screen minus the popped-out sidebar (`sidebar-pop-width` while `sidebar-popped`), from `logical-height - bar height` to the bottom edge, where the bar height is the bottom panel's laid-out height (`bottom-keys.height`, which Slint computes also while it is invisible; export it, e.g. `out property <length> bottom-band-height`). The host decides "in band" from the samples it already reads: `bottom-hover` while the bar shows, else `video-hover && pointer-y >= logical-height - bottom-band-height` (and the pointer is left of a popped-out sidebar). All in logical px, so it works on any monitor size and scale; a detached sidebar takes nothing from the band; a popped-out sidebar narrows it like it narrows the bar today. No extra `TouchArea` over the video (the band must keep its clicks and double-clicks).
2. **Hide 500 ms after the pointer leaves the band** (the same value as the sidebar pop-out's `LEAVE_MS`); entering the band shows it at once. Re-entering within the delay cancels it.
3. **Nothing else reveals it**: `activity()` (pointer motion outside the band, keys, bindings, menu actions, dialogs, mounts, lock writes) and entering fullscreen no longer make the bar effective. Proposed shape: in `Policy`, the fullscreen bottom becomes `effective` iff `in_band || left_band_at + LEAVE_MS > now || bar locks (plugin dragging/focused/popupOpen)`; a menu or dialog opened while the bar is shown **holds** it until closed (so the bar's own popup menu or Open dialog do not pull the bar away), but does not reveal a hidden bar. `next_deadline_ms` covers the leave delay.
4. **OSD unchanged**: key seeks still show the OSD message (it is a separate host layer, `osd-visible`); only the bar stays hidden.
5. **Cursor auto-hide keeps working, decoupled from the bar**: in fullscreen the pointer hides when playing and `policy.idle(now)` (3 s without pointer motion or keys, no menu or dialog, the pointer not over the bar, no bar lock), i.e. the windowed `"playing"` rule without the active-window condition; `setCursorAutoHide` semantics unchanged. Keys still count as activity for the cursor and for the popped-out sidebar's idle collapse; they just no longer touch the bar.
6. **Windowed mode unchanged** (the bar is docked and does not auto-hide; `setCursorAutoHide("playing")` as today).

**State and SDK**: `SlotState` stays as is; a hidden fullscreen bar keeps `effective: false, reason: "idle"` (meaning becomes "fullscreen, pointer not over the band"). Document in `doc/plugins/reference.md` §6.3 and `doc/product/interaction-spec.md` (G-04 acceptance items 3 and 4, F-06), and update the TestHost only if it models the bar's fullscreen idle (it does not today). Failure / missing capability: headless runs have no bar, nothing changes.

**Does it break a contract**: it changes host default behaviour (when the fullscreen bar shows), not a signature. Affected: `vestra.playback` (wants it); the window plugin's `setCursorAutoHide` (behaviour kept, see 5); `vestra.playlist` (sidebar pop-out logic untouched, but the "bar lies left of the popped-out sidebar" pointer case in `main.rs` ~1500 must keep working). No other plugin mounts the bottom slot.

**Minimal acceptance**: host unit tests in `window_state.rs`: (a) fullscreen + `activity()` alone (keys, motion outside the band, entering fullscreen) → bottom `effective: false`; (b) pointer into the band → `effective: true` at once; out of it → still effective at +499 ms, hidden at +500 ms; back in within the delay → stays; (c) a bar lock (dragging) holds it after the pointer leaves, release starts the 500 ms; (d) a menu opened while shown holds it, opened while hidden does not reveal it; (e) band x-range excludes a popped-out sidebar; (f) cursor hides after 3 s idle in fullscreen while the bar is hidden, never while the pointer is in the band. Real GUI (coordinator): fullscreen on two monitor sizes, with the sidebar popped out and detached; Ctrl+→ shows "Forward 30 s: ..." on the OSD with no bar.

**Plugin side (done in `feat/fullscreen-bar-hover`)**: nothing in the plugin reveals the bar; its only locks are real drags on the seek and volume sliders, released on pointer up. `src/plugins/playback/tests/fullscreen-bar.test.ts` pins that: key seeks and speed in fullscreen show the OSD and take no lock; a drag holds the bar and releases it. No plugin code change is needed, so the plugin version stays at 0.1.6.

**Maintainer reply** (2026-10-25): **accepted** as written; classified as a host default-behaviour change with no API or SDK signature change (no existing API can do it: the bar's visibility is host-only, and a hidden panel has no hover signal). Implemented in `feat/fullscreen-bar-hover`:

- `window_state.rs`: `Policy::bottom_band(over_bar, over_video, viewport, band_height, now)` decides "in band" (bar hover, or the video pointer at `y >= height - band_height` and left of a popped-out sidebar; no band outside fullscreen, without a laid-out bar or on non-finite samples). The fullscreen bottom is `effective` iff in band, a bar lock or hover, a menu or dialog opened while it showed, or less than `LEAVE_MS` (500) since that hold ended; otherwise `reason: "idle"`. `activity()` and `set_fullscreen(true)` no longer touch the bar. `next_deadline_ms` now reports the pending leave-delay hide. `idle()` also excludes the pointer in the band.
- `composition.rs`: the shell exports `out property <float> bottom-band-height: bottom-keys.height / 1px` (0 without a bottom mount); `bottom-keys` keeps its explicit height while invisible, so the band exists while the bar is hidden. No new `TouchArea`.
- `main.rs`: samples the band every tick before `Policy::tick`; the pointer auto-hide in fullscreen is now `playing && policy.idle(now)` (decoupled from the bar); the windowed `setCursorAutoHide("playing")` rule is unchanged. The activity sources stay (the pointer and the popped-out sidebar's idle collapse use them); they just no longer reveal the bar.
- Tests (acceptance (a)–(f)) in `window_state.rs`; docs: `doc/plugins/reference.md` §6.3 and §6.6, `doc/product/interaction-spec.md` (G-01 item 6, G-04, new FS-BAR-1, F-02/F-03/F-06/F-09), `doc/product/ui-spec.md` (UI-SHELL-09, C1), `doc/architecture/core-ui-design.md`. TestHost does not model the fullscreen bar, so it is unchanged.

Deviations / notes (none change the requested behaviour):

1. A bar lock (`setInteraction`) also shows a hidden bar, as the proposed shape (`... || bar locks`) says; the plugin locks only during real drags, so in practice it only holds.
2. The leave delay after a lock release or menu close starts at the next host tick (about one frame later), because lock setters take no timestamp.
3. A side effect of decoupling: in fullscreen the pointer now also auto-hides when the bar is user-hidden or no bottom panel is mounted (it used to follow the bar's `idle` reason and never hid then).
4. SDK text that describes the old behaviour, for an SDK patch (not edited here): `SlotState.reason` `idle` ("hidden after 3 seconds without activity"), `Panel.setInteraction` remarks ("The bottom bar hides after 3 seconds without activity in fullscreen"), and `Panels.mount` `"bottom"` ("In fullscreen it floats over the video and hides after 3 seconds without activity"); new wording: "in fullscreen it shows only while the pointer is over its band at the bottom edge and hides 500 ms after the pointer leaves". Same text in the generated `llms-full.txt`.

## STORE-ROW-LOGO: images inside row values (§4.1 request, store author, 2026-10-26)

**Requesting plugin / code location**: `vestra.store`, `src/plugins/store/store.ts` `#row()`, `ui/store.slint` `StoreRow` / `ListRow`.

**User action and expected result**: the plugin-center spec ("List rows") puts the plugin's 32px logo at the left of every list row; offline, an installed plugin's logo comes from its own package.

**Why the existing API cannot do it**: `PropValue` allows an `ImageRef` only as the whole value of a property; `PropData` (struct fields, array items) has no image. The host agrees: `ui_image.rs` `convert_prop` accepts an image reference only at the top level (`image reference as the whole value`), while `MarkdownRef` works at any depth since SDK 1.5.4. Minimal reproduction: `window.set({ rows: [{ key: "a", icon: plugin.ui.image(path) }] })` with `icon: image` in the Slint struct fails the type check (`Type 'ImageRef' is not assignable to type 'PropData'`). A fixed set of top-level `icon-0…icon-n` properties is not a fix (the list has no upper bound, and the rows come and go with search and tabs). Reading the icon file itself already works: the store copies an installed package's icon into its data folder (`icons.ts` `local()`), so no new file API is needed.

**Desired usage**: as for Markdown since 1.5.4, an `ImageRef` at any depth of a property value:

```ts
window.bind(() => ({ rows: items.map((item) => ({ key: item.key, name: item.name, icon: plugin.ui.image(iconPath(item)) })) }));
```

```slint
export struct StoreRow { key: string, name: string, icon: image }
```

**State, resources, failure**: the host owns the decoded images (the existing cache and its 16 MB budget, keyed per property, like `assign_many` for Markdown images); the plugin owns the files (package or data folder, as today). A path that fails the existing checks makes that field an empty image (as a Markdown image that fails), never the whole property. Missing capability on an older player: the row shows no logo (the field is the default image).

**Contract / affected plugins**: compatible extension (a value that was refused becomes accepted); no plugin changes. SDK: widen `PropData` with `ImageRef`, and the TestHost conversion.

**Minimal acceptance**: the store's Installed tab shows each plugin's logo in its row, offline too; 18 built-ins plus a few third-party rows do not exceed the image budget; the screenshot tool renders the logos.

**Store side meanwhile**: rows ship without a logo; the details pane shows the icon (now also offline, from the installed package). The row layout reserves nothing, so adding the logo later is one `Image` in `ListRow`.

## STORE-RELOAD-BUILTIN: restart a running built-in plugin (§4.1 request, store author, 2026-10-26)

**Requesting plugin / code location**: `vestra.store`, `store.ts` `#controls()` / `#switch()`.

**User action and expected result**: spec acceptance 1: a built-in row's `…` menu offers Reload and Open Data Folder. Reload stops and starts the plugin (recovering one that misbehaves without restarting the player).

**Why the existing API cannot do it**: the only restart is `packages.setEnabled(id, false)` then `true`. Since the ruling "built-ins can be neither uninstalled nor disabled", the host refuses `setEnabled(id, false)` for a built-in with `unsupported` (`package_manager.rs` `set_enabled`, and the TestHost the same). `setEnabled(id, true)` on a running plugin is a no-op. So a **failed** (or stopped) built-in can be started again, but a **running** one cannot be restarted. Minimal reproduction: `await plugin.packages.setEnabled("vestra.video", false)` → rejects `unsupported`.

**Desired usage** (one of):

```ts
await plugin.packages.reload(id); // stop and start; built-ins included; never leaves it disabled
```

or let `setEnabled(id, false)` stay refused but add `packages.restart(id)`. Semantics: stop, then start from the same package; the persisted disabled flag is never written; `invalid` for oneself (as today), `busy` during an install, `missing` unknown; a failed start leaves the plugin `failed` with the reason, as an enable does.

**State, resources, failure**: the host owns the process and the persisted state; nothing on disk changes. Missing capability (older player): the store keeps the menu entry disabled for a running built-in.

**Contract / affected plugins**: a new optional method; no behaviour change elsewhere. The store would also use it for third-party plugins (one call instead of two, so a reload can never leave a plugin disabled if the second call fails).

**Minimal acceptance**: `reload("vestra.video")` on a running built-in stops and starts it (`packages.onChange` shows `starting` → `running`), the built-in is never reported `disabled`, and a restart of the player shows it running.

**Store side meanwhile**: Reload is offered for built-ins and enabled only when the built-in is not running (failed or stopped: `setEnabled(id, true)` starts it); for a running built-in the entry is shown disabled.

## STORE-ICONS: refresh, settings and "more" icons in the V-Line set (§4.1 request, store author, 2026-10-26; low priority)

**Requesting plugin / code location**: `vestra.store`, `ui/parts.slint` `StoreIcons`.

**User action and expected result**: the spec's ⟳ refresh button, ⚙ menu button and each row's `…` button are icon buttons drawn in the player's icon style.

**Why the existing API cannot do it**: `@vestra/icons.slint` (and `icons-v-line.json`) has no refresh, settings or more icon. It is not blocking: `Button` takes Path data (`icon`, `stroked`), so the store draws its own three V-Line style paths (1.5px round strokes in a 14×12 box) in `parts.slint`.

**Desired usage**: `Icon { name: "refresh"; }`, `"settings"`, `"more"` (and `Button { icon: Icons.commands("refresh"); stroked: true; }`), so that other plugins get the same glyphs (the workshop and the hotkeys page have similar buttons).

**State / failure / contract**: none; additive. **Acceptance**: the store switches to the named icons with no visible change in its screenshots.

## Progress

- [x] Step 0: screenshot tool (`npm run ui:shots`, [ui-shots.md](../../architecture/ui-shots.md); maintainer, 2026-10-26; coordinator review pending)
- [ ] Step 1: plugin center specification → negotiation → implementation → acceptance
- [ ] Step 2: full screenshots → review → fixes → second review → acceptance
- [ ] FS-BAR-1: fullscreen bar on hover only (accepted and implemented in `feat/fullscreen-bar-hover`; coordinator GUI test pending)
