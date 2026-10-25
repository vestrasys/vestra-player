# 2026-10 round: sidebar tabs and opening NES files

Started 2026-10-24. **This document is the single source of progress for this round.**

## Sidebar tabs

Design: [panel-slots.md](../../architecture/panel-slots.md). The user chose option A.

- [x] Step 0: a mount reports `conflict` directly and no longer fails activation along with it; built-in plugins mount first (#2, verified on a real machine)
- [x] Step 1 (#3, 2026-10-24)
  - [x] Maintainer: host, runtime, SDK 1.5.2 (released); also fixed windows moving up by one title-bar height when reopened (floating sidebar, main window and Options window all had it)
  - [x] Playlist author: 0.1.4, F6 now uses `list.toggle()`, and the plugin hides its own tab strip when the host tab strip is present
  - [x] Real-machine verification: tab switching, context menu, detach and attach, restore after restart, the three F6 cases

## Request R1: hand local files to a plugin-produced stream by extension (vestra.nes, §4.1)

**User action**

Opening a `.nes` file with Open File (⌘O), drag and drop, or the playlist (including Previous/Next) should start the emulator directly.

Today all of these entry points hand the file to mpv, opening fails, and the playlist shows a warning; only "Emulator ▸ Open NES ROM…" works. After the NES plugin is disabled, the current behavior returns.

**Why the existing API is not enough (author's evidence)**

1. `drops.provide({extensions})`: covers only one entry point, dropping onto the video area. ⌘O, the playlist and Previous/Next all go through `player.open(path)`.
2. `resolvers.add`: accepts only http/https plus a host name, and can only return a URL for mpv, not a produced stream.
3. Listen to `onChange`, wait for the mpv error, then call `openStream`:
   - the failure message has already appeared, and the playlist has already marked the item with a warning;
   - `StreamOptions` has no `ifCurrent`, so it races with other open operations;
   - after opening, `source` becomes `producer://<id>`, the original path is lost, and the playlist's current row and Previous/Next break;
   - it races with the sequence's auto-advance;
   - the plugin can only guess the open intent.
4. Neither the manifest nor the capability table has any field related to file types.
5. Keeping only the emulator menu entry is exactly the problem the user reported. Having playback, playlist and history each recognize `.nes` would need either plugin-to-plugin communication or a core that knows about plugins; both violate §1.

Author's conclusion: open intent, conditional commit, source naming and failure display all belong to the host's source arbitration (§4.3 item 4), and this is a general capability that future emulators, synthesizers and procedural sources will also use.

**Desired usage**

`resolvers.add` gains `extensions: ["nes"]`, and `resolve(path, signal)` can return `{ kind: "stream", width, height, frame, onStop }`. The plugin's own menu entry also becomes "dialog + `player.open(path)`". The maintainer decides the exact shape.

**Semantic requirements**

- Reuse the arbitration, `ifCurrent` and operation id of the original `open`.
- Once selected, `kind === "stream"` and `source` is the original file path.
- On failure, `phase: "error"`, with the error text supplied by the plugin.
- Resources belong to the plugin that registered the resolver.
- Cancellation is passed through `signal`.
- Only one registrant per extension; later ones get `conflict`, without affecting their activation.
- The resolver is unregistered when the plugin is disabled.
- No OS-specific concepts appear.

**Compatibility**

An optional extension at the type level. One semantic point needs the maintainer's confirmation: once a produced stream's `source` becomes a file path, plugins that look only at the path without checking `kind` are affected.

**Acceptance scenarios**

Open a ROM with ⌘O; Previous/Next in a mixed playlist; drag and drop; stop with F4; error on a bad ROM; superseded by a new open while reading; fallback after disabling. Covered with TestHost.

**Issues the author found along the way (handled separately)**

- The host does not accept files passed from the command line or the system.
- A produced stream's `source` is actually `producer://<id>`, while the requirements document says "`source` is empty"; the two disagree.

**Maintainer reply (2026-10-24): accepted after rewrite**

- **Checking the author's evidence**: items 2, 3 and 5 hold. Item 1 has one inaccuracy: a `.nes` dropped onto the video area is currently ignored silently, not handed to mpv. None of the alternatives that leave the API unchanged (A1–A5) achieve what the user wants. It meets §4.3 item 4: source arbitration belongs to the host, dispatching by source is part of the core's open responsibility, and it is a general capability.
- **Scope**: local absolute paths only, produced streams only; http(s) resolvers are unchanged.
- **Shape**: `ResolverOptions` becomes a union type with a new `{ id, extensions, resolve(path, signal): Promise<ResolvedStream> }`. `ResolvedStream = { kind:"stream", width, height, frame, started?(stream) }`: the author's `onStop` is replaced by handing over the `Stream` handle, so there is only one concept.
- **Wire**: `register` gains the `{id, extensions}` form; `respond` gains `{kind:"stream", producerId}`, and the host replies `{stamp}`.
- **Capabilities and manifest**: requires `source.resolve` and `producer.av`; no new manifest fields; NES sets `minPlayerVersion`.
- **Matching**: the source contains no `://`, is an absolute path, and the extension is case-insensitive; the extension branch comes before URL parsing (Windows `C:\`).
- **Conflicts**: only one registrant per extension; later ones get `conflict`, which does not fail activation.
- **Source naming**: `source` keeps the original path, `kind === "stream"`; a direct `openStream` source is still `producer://<n>`.
- **Failure**: the error uses the plugin's original text; the `resolver failed: ` prefix is removed for all resolvers.
- **Cancellation, disabling, old hosts**: cancellation reuses the existing mechanism (superseded, 30-second deadline, disabled, unregistered); after disabling, behavior returns to today's; old hosts are blocked by `minPlayerVersion`.
- **Version**: SDK 1.5.3 (patch, after 1.5.2).
- **Plugins needing regression**:
  - must change: playlist (`queue.ts:531` detaches on a stream);
  - to confirm: marks, info, subtitles (`hasMedia` does not look at `kind`);
  - tests only: the other plugins, youtube, bilibili.
- **Effort**: about 2–2.5 days for the maintainer (human estimate), about 40 lines for NES, a few lines for playlist.
- **Two trade-offs awaiting a ruling**:
  1. Should host-form resolver conflicts also stop failing activation?
  2. Does removing the error prefix apply to all resolvers?
- **Found along the way**:
  1. Command-line and system file opening get a separate host item (platform modules all deliver to the `Open` intent); R1 does not depend on it.
  2. `producer://` stays unchanged; the documents change: `PlayerState.source` in `reference.md` is changed by the maintainer; for `feature-requirements.md:508` the maintainer supplies the wording and the coordinator writes it in.

**Author reply (2026-10-24): accepted, no objections**

The author checked each acceptance scenario; all can be met. The author also asks the maintainer to state the following 4 points in the implementation and the docs:

1. If cancelled after `resolve` returns but before activation: release the producer and do not call `started`.
2. `started(stream)` is called synchronously after activation, before any `onStop`.
3. `player.open(path, {paused:true})` is passed through to the stream's `startPaused`.
4. Calling `player.close()` while resolving aborts the `signal` of `resolve`.

NES change plan (about 40 lines):
- resolver, drag and drop (`drops.provide` → `player.open`);
- the entry becomes "dialog + `player.open`";
- remove the hand-written open and cancel logic;
- add tests, bump the version to 0.3.0, `minPlayerVersion`.

**Ruling (maintainer, 2026-10-24): accepted after rewrite, implemented**

Implemented on branch `feat/file-resolvers` (player worktree `wt-r1`) and SDK 1.5.3 (`sdk-r1`).

- All 4 points from the author are implemented and covered by tests.
- The two trade-offs:
  1. Resolver conflicts never fail activation, including the host form, consistent with the principle set in #2;
  2. The `resolver failed: ` prefix is removed for all resolvers; the SDK now sends `error.message`.
- Old hosts: plugins that use file resolvers set `minPlayerVersion` to the next player version (tentatively 0.4.1).
- Merge order: SDK 1.5.2 (sidebar tabs) → rebase 1.5.3 onto it → release → the player updates the dependency → the NES author and the playlist author adapt following the migration notes.

## Adapting to R1 (2026-10-24)

- [x] Playlist author: 0.1.5 (PR #9). In a mixed list, passing through a produced stream keeps the current row, Previous/Next and auto-advance working; closing with F4 while resolving does not count as a failure; a local produced stream opened from elsewhere is also taken into the list (except `producer://` sources).
- [ ] NES author: 0.3.0 (in progress)
- [ ] Real-machine verification

## Request R2: list the file types plugins can resolve (vestra.playlist, §4.1 draft)

- **User action and expected result**: dragging a ROM into the list, or scanning a folder containing ROMs, should add the ROM to the list just like a video; today it shows "Skipped N unsupported items".
- **Why the existing API cannot do it**: there is no interface that lists the extensions of registered file resolvers. Hard-coding `.nes` violates §1; accepting everything would add unrelated files to the list too.
- **Desired usage**: `resolvers.fileTypes(): string[]` plus a change event. Read-only, lower-case extensions only, registrants not exposed.
- **Who owns state and resources; on failure, cancellation, missing capability**: state belongs to the host; when a plugin is disabled, its extensions disappear with it; without the capability it returns an empty list, no error.
- **Does it break existing contracts**: purely additive.
- **Minimal acceptance scenario**: with NES enabled, drag game.nes into the list and it is added; with NES disabled, the same drop shows "Skipped".
- **Author's side suggestion**: when resolution is cancelled, give a separate error code (such as `cancelled`), so plugins no longer have to compare the text "source resolution cancelled".
- **Maintainer reply (2026-10-24): accepted after rewrite (not implemented, waiting for the author's reply)**
  - Checked: when dragging into the list or adding files or folders, the list filters by its own extension table, and the plugin has no way to know which extensions have resolvers. ROMs opened with ⌘O or by dropping onto the video area are already taken into the list by 0.1.5.
  - Shape: a read-only snapshot `plugin.resolvers.fileTypes` plus `resolvers.onFileTypesChange`. Contains only the extensions of registered file resolvers (lower-case, deduplicated, sorted), without registrants; reading needs no capability, change notifications need events; returns `[]` on old players. SDK 1.5.8.
  - `cancelled` error code: rejected. The test is `code === "backend" && player.state.phase === "empty"` instead, written into reference §11.
- Related fix: SDK 1.5.7. When a stream is replaced or closed, `onStop` receives null; TestHost reclaims the producer just like the real host (a defect found during NES testing).

## Defect report D1 and request R3: panel properties for large lists (vestra.playlist, §4.1, 2026-10-24)

Source: on a real Windows machine (0.4.2) the user added about 3000 files, and the list was much slower than PotPlayer and standalone mpv. Playlist 0.1.6 already solved it inside the plugin (it sends only the 200 rows near the view and reads durations only for rows on screen); below is the part the plugin cannot work around and the maintainer should handle.

### D1 (defect): property value limits are not documented, and TestHost does not enforce them

- **Code location**: `src/host/src/ui_values.rs`: `MAX_ARRAY = 1024`, `MAX_NODES = 4096`, `MAX_TEXT_BYTES = 1 MiB` (field names count too). Every `ui.setMount` and `ui.set` goes through the `encode` check.
- **Symptom**: each playlist row is a struct with 8 fields, counting as 9 values, so above about 450 rows the whole `rows` write fails. When 0.1.5 added 3000 files, the simulation sent `rows` 6417 times in total (about 1 MB each, about 2.8 GB in total), all of which the host rules would reject. In the real player the list stays on the old content; the host also parses each 1 MB of JSON on the UI thread before reporting the error, which is the main reason the user felt it was "laggy".
- **Why plugin tests did not catch it**: TestHost's `ui.setMount` accepts any value. reference.md mentions "4096 in total" only in the Markdown section; these limits are not written down for ordinary properties.
- **Expected**: reference §6 (properties) states these three limits and the failure semantics; TestHost performs the same checks, and a failed property write goes to the same `onError` as on the host. Follow the precedent of the command limits (reference.md line 1837: TestHost did not enforce the limit, and plugin tests did not catch it).
- **Minimal reproduction**: `panel.bind(() => ({ rows: Array.from({ length: 500 }, (_, i) => ({ a: String(i), b: "", c: "", d: "", e: false, f: false, g: false, h: false })) }))`. Passes in TestHost; every write fails in the player.

### R3 (request): let the host hold long lists, and let plugins insert, delete and change rows

- **User action and expected result**: with 3000 to 20000 items in the list (`ROW_LIMIT`), dragging the scroll bar to any position shows rows immediately with no blank area, as in PotPlayer.
- **Why the existing API cannot do it**: bound by the D1 limits, the plugin can only send a window (0.1.6: 200 rows, about 68 KB); a Slint `ScrollView` sets the height from `row-count` and the position from `first-row`. The window follows the scroll callback, which takes a "Slint → plugin → Slint" round trip. The wheel and small drags within the 40-row margin are unaffected; when dragging far in one go, rows at the new position appear one round trip later. Also, any change to a row (selection, duration, current item) resends the whole window, and during reconcile the host compares the 200 rows one by one with `to_json`.
- **Desired usage** (sketch; the maintainer decides the shape):
  ```ts
  const rows = panel.model("rows");           // [PlaylistRow] on the Slint side, held by the host
  rows.splice(start, deleteCount, ...items);  // insert/delete, ≤ 1024 rows per call
  rows.set(index, item);                      // change one row
  rows.reset(items);                          // replace everything (after sorting), can be batched
  ```
  This way a Slint `ListView` can virtualize locally, and scrolling does not go through the plugin.
- **Who owns state and resources**: the model belongs to the host and is released together with its panel when the panel is unmounted or the plugin is disabled. Beyond the total limit (such as 20000 rows): `limit`; without the capability: `unsupported`.
- **Does it break contracts**: purely additive; existing whole-value properties are unchanged.
- **Alternative (no API change) and its cost**: the current 0.1.6 window approach, with the cost described above: a brief blank on large jumps, and 200 rows resent on every window move.
- **Minimal acceptance scenario**: in a 20000-row list, drag the scroll bar from top to bottom with rows in every frame; selecting one row changes only one row.
- **Impact**: only vestra.playlist for now; future long-list plugins such as a media library, bookmarks and history will hit it too.

### Measurements (TestHost + in-process plugin timing, 3000 empty `.mkv` files, fake ffprobe answering immediately; on the host side every write is validated by the rules in `ui_values.rs`)

| Operation | 0.1.5 | 0.1.6 |
| --- | --- | --- |
| Add files (dialog, 3000) | 27.4 s, 5529 writes, 2.8 GB; serial realpath; every ffprobe result resends all rows | 0.12 s, 10 writes, 134 KB; ffprobe 49 times (only rows on screen) |
| Drop folder / add folder | 18.5 s / 19.6 s, about 1.8 GB | 0.42 s / 0.39 s, about 200 KB |
| One scroll step | 8 ms CPU (stringifies a snapshot of the whole list every time) | 1.3 ms CPU (including TestHost); window moved 24 times in 200 steps |
| One player state push (progress, etc.) | 9 ms CPU | 1.4 ms |
| Click to select, select all, play a row | about 1 MB resent each time | 68 KB |
| Next / Previous | about 80 ms CPU each, 3 MB resent | about 10 ms, 200 KB |
| Sort (title, folder, shuffle…) | 18 to 80 ms | 3 to 60 ms (size and date need stat) |
| Delete 3000 rows after select all | O(n²) (indexOf, splice and filtering the pending queue per row) | one filter pass |
| Writes the host would reject | 6417 (rows 3000 > 1024) | 0 |

### Maintainer reply to D1 (2026-10-24): fixed as a defect, implemented (branch `fix/ui-value-limits`, SDK 1.5.10, not committed)

- **Checked**: confirmed. The limits appear only in the TSDoc of `PropValue` and the Markdown section of the reference; `TestHost` does not check them. Two corrections:
  1. Rejected writes are not completely silent: the SDK hands the first failure of each run of consecutive failures to `onError` (code `host`), but the playlist has no `onError`, so it only went into the plugin log.
  2. JSON parsing happens on the host's pipe reader thread, not on the UI thread; the UI-thread conversion stops at the limit. The "lag" of 0.1.5 was mainly on the plugin side (2.8 GB of stringify) and in the pipe. A more dangerous point: when a single message exceeds 1 MiB, the host stops the plugin outright (`plugins.rs`). With 3000 rows and longer paths, 0.1.5 was already close to that line.
- **Fix**:
  - **Docs**: reference §6.1 gains a "Property value limits" table stating the failure semantics; §13 `limit`, §14 TestHost and the §17 change notes are updated to match; the TSDoc of `PropValue`, `View.set` and `View.bind` is updated too.
  - **TestHost**: `ui.open`, `ui.set`, `ui.mount`, `ui.setMount`, `settings.register` and `settings.set` perform the same checks as the host (same order and messages). A new constant `UI_VALUE_LIMITS` is exported from `@vestra-player/sdk/testing`; `tests/sdk/host-agreement.test.ts` checks that it matches `ui_values.rs`.
  - **Failures made visible**:
    - The code for exceeding a limit changes from `host` to `limit`, and the message starts with the property name (`rows: UI array exceeds 4096 rows`).
    - Initial properties of panels and settings pages, and `settings.set` while the settings window is not open, were not checked before; they were quietly dropped when the view was rebuilt (logged only in the host log). Now they are checked at request time:
      - `settings.set` fails when over the limit;
      - an initial property over the limit is left out of the mount, and `ui.error` is sent. The mount is not made to fail because a mount failure during activation fails the whole plugin activation: playlist 0.1.5 on main would do exactly that when restoring 20000 rows, and running `C17: a list above 20 000 rows is refused` with TestHost 1.5.10 reproduces it.
      - `ui.open` for tool windows still fails as a whole.
    - UI failures the host only discovers later (an image deleted before a rebuild, a callback argument that fails to convert, a floating sidebar that fails to prepare) are sent as `ui.error`, now with a `code`; previously the SDK did not handle this event at all, and now it goes to the root scope's `onError`.
    - The plugin runner rejects requests above 1 MiB before sending (`limit`), instead of letting the host stop the plugin.
- **The limits themselves (ruling: relax two, keep the rest)**:
  - Measured in a release build (Apple M series), the UI-thread cost of one write (conversion plus reconcile) for 8-field rows: about 1.3 ms for 200 rows, about 4.9 ms for 1000 rows. Most of the time is in reconcile: it converts the whole value to JSON for one comparison, then converts each row again. It now reuses the first result, cutting reconcile by about 45%: about 0.8 ms for 200 rows, about 3.2 ms for 1000 rows, about 5.8 ms for 1800 rows.
  - After the change each value costs about 0.35 µs. So `MAX_NODES` 4096 → **16384** and `MAX_ARRAY` 1024 → **4096**: a full write is about 1800 rows and about 6 ms, estimated at around one frame on a slow Windows machine, and only when writing.
  - Depth 8, 64 KiB per string and 1 MiB of total text are unchanged. Total text is bounded by the 1 MiB message limit, so relaxing it is pointless; relaxing it would mean changing the pipe framing, which is costly, and this also shows that whole-value properties are not the way to go for long lists (see R3).
  - This lets medium lists (chapters, subtitle entries, file lists of a few hundred to a little over a thousand items) be sent as whole values; 20000 rows is still not possible, and should not be done this way.
- **Impact on the playlist**: the 200-row window of 0.1.6 is unaffected. The hard-coded "1024 rows and 4096 values" in the `scroll.ts` comments should reference `UI_VALUE_LIMITS` once SDK 1.5.10 is released. `WINDOW_ROWS` can be increased; for the trade-off see A1 under R3.

### Maintainer reply to R3 (§4.3 step 2, 2026-10-24): leaning towards accepting after rewrite, not implemented yet

**First, checking the author's arguments**

- Both costs of the window approach hold:
  - A far jump waits for a "Slint → plugin → host → Slint" round trip before rows at the new position appear.
  - Any change to a row resends the whole window.
- The actual round-trip time has not been measured on a real machine. Estimated from the parts: callback enqueue, Node handling and stringify of 68 KB, pipe, and reader-thread parsing about 0.3 ms; UI-thread conversion plus comparison about 0.8 ms (the figure after the D1 change). With the plugin idle that is about 1–2 frames; with the plugin busy (sorting, a batch of ffprobe results coming back) it is longer. The author is asked to confirm or refute this estimate with real-machine data in step 3.

**Approaches without API changes and their costs**

| Option | Approach | Cost / what it cannot do |
| --- | --- | --- |
| A0 current (0.1.6) | 200-row window; `row-count` sets the height, `first-row` sets the position; the scroll callback moves the window | One round trip of blank on far jumps; every change resends 200 rows (about 68 KB, about 0.8 ms on the host) |
| A1 larger window (possible after the D1 relaxation) | `WINDOW_ROWS` becomes 600–1000, with a correspondingly larger margin | The wheel and drags almost never move the window, but far jumps are still blank; the amount resent per change and the host cost grow linearly with the row count (1000 rows about 340 KB, about 3 ms) |
| A2 placeholder rows | In Slint, draw placeholders (index plus skeleton bar) for cells outside the window according to the viewport, and replace them when data arrives | A jump shows placeholders instead of blank, as Explorer and most virtual lists do; only the plugin's Slint file changes, about 30 lines. Cannot give "real rows in every frame" |
| A3 split out volatile state | Send row content separately from selection, current item and duration: `current-row: int`, a `[bool]` selection table the same length as the window, and durations in a separate table | Clicking, playing and duration back-fill send only a few hundred bytes and no longer resend rows; bindings between plugin and Slint get somewhat more complex. Does not solve jumps |
| A4 lower the requirement | Accept "about 1–2 frames of placeholders after a jump" as the acceptance criterion instead of "rows in every frame" | A visible gap from the PotPlayer experience, only when dragging the scroll bar quickly |

A1 to A3 can be combined and need no core work, so the author can do them first. But none of them meets the acceptance scenario as written: dragging 20000 rows from top to bottom with real rows in every frame. That requires the data to be on the Slint side, and a Slint model can only be held by the host; the plugin cannot get at it.

**Leaning ruling: accept after rewrite**

Based on §4.3 item 4:

- **Needs host-internal state**: the Slint model lives in the host process, and only the host can let `ListView` virtualize locally.
- **General base capability**: media library, history, bookmarks, logs, search results; every future long-list plugin will hit this. Letting each plugin approximate it with a window plus placeholders is reinventing the wheel.

Points of the rewrite:

- Not limited to panels: windows, panels and settings pages can all use it.
- Rows are read-only: the plugin is the only data source, and the host does not read back.
- Whole replacement is atomic and never shows a half-sorted list.
- Volatile state such as selection does not go into the rows (see the suggestions below).

**Shape, if accepted**

- **API** (new in the SDK, purely additive):
  ```ts
  const rows = panel.model<PlaylistRow>("rows"); // View.model(property): this property becomes a host-held list
  await rows.replace(items);                     // replace everything (sort, reload); the SDK sends chunks, the host swaps them in once all arrive
  await rows.splice(start, deleteCount, ...items);
  await rows.set(index, item);                   // one row
  rows.length;                                   // SDK-side mirror, read-only
  rows.dispose();                                // the property returns to an ordinary property, as an empty list
  ```
  - Using `set` or `bind` on the same property afterwards throws (`conflict`).
  - Calls run in order and are not merged, unlike `Latest`.
  - If a call fails: the SDK resynchronizes the whole list with `replace` and hands the error to `onError`.
  - On the Slint side, declare `in property <[PlaylistRow]> rows;` as usual, with a `ListView`. `in-out` edits inside rows are not passed back; the docs state "rows are read-only, edits go through callbacks".
- **Wire**:
  - `ui.model.splice {view, property, start, deleteCount, rows}`
  - `ui.model.set {view, property, index, row}`
  - `ui.model.replace {view, property, rows, final}`: chunks are staged and swapped in when `final: true`
  - `ui.model.release {view, property}`
  - `view` is a window id, `$mount` or `$settings:<id>`, the same as `ui.callback`.
  - The host holds a `ModelRc` of `VecModel<Value>`. When a view is rebuilt (another panel mounts, the theme changes, floating and docked swap), the same model is put back into the new instance; the docked and floating instances share one model.
- **Validation**:
  - Each chunk of `rows` counts against the D1 property value limits (one chunk ≤ 4096 rows, ≤ 16384 values, ≤ 1 MiB of text, and the whole request ≤ 1 MiB).
  - New rows must have the same shape as existing rows (the host's existing `fits` rule), otherwise `invalid`.
  - An out-of-range index is `invalid`.
- **Limits**:
  - Each list ≤ 100 000 rows and ≤ 32 MiB of text in total; each plugin ≤ 8 lists. Beyond that: `limit`.
  - Memory estimate: 20000 rows × about 300 B per row, about 6 MB.
  - Filling 20000 rows the first time takes about 12 chunks, each about 6 ms of UI-thread time, spread over several frames.
- **Lifecycle**:
  - A list belongs to its view: released when the window closes, and on the next `show` the SDK calls `replace` again from its mirror.
  - Released together when the panel is unmounted, the scope is released or the plugin is disabled.
  - Old players do not know these methods; they are blocked by `minPlayerVersion`, and the SDK docs state the required version.
- **TestHost**:
  - `host.window(id).props.rows` and `host.mounted().props.rows` give the full array with splices applied.
  - Performs the same validation and limits.
  - `host.sent("ui.model.set")` is used to assert "selecting one row changes only one row".
  - View rebuilds are not simulated.
- **Design suggestions for the author**:
  - Do not put selection or the current item into `PlaylistRow`: selecting all of 20000 rows would become 20000 `set` calls.
  - Use `current-row: int`, and for selection a `[bool]` list also held by the host, replaced as a whole on select all. Or use ranges (`[{from, to}]`), checked by `index` on the Slint side.
- **Effort (human estimate)**:
  - Host about 2 days: RPC, per-view registration, putting back on rebuild, release, limits, Rust tests.
  - SDK about 1 day: API, chunking and resynchronization, TestHost, TSDoc, reference, example.
  - Playlist migration about 1 day (author): remove the window logic, change `ScrollView` to `ListView`, move selection into a separate list.
  - The coordinator also needs to run one GUI regression on a real Windows machine.
- **Schedule**: D1 ships first (SDK 1.5.10 and the matching host). R3 is ruled on after the author's step-3 reply, and implemented as a separate round; it is not merged with D1, because D1 is a defect fix and should not wait for a new capability.

**Questions for the author to answer in step 3**

1. Whether you accept the shape above, in particular "rows are read-only" and "selection does not go into the rows".
2. After A1 + A2 + A3 land, how many frames the placeholder or blank lasts on a far jump on a real machine (Windows, 20000 rows). If it is only 1–2 frames and you find that acceptable, R3 becomes a rejection, with A1–A3 as the alternative.
