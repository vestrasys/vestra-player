# Palette lead, `commands.run` and the built-in-only requests (maintainers)

Host side of Ask Vestra (design: `C:\vestra\design\ask-astra\PROPOSAL.md`, user decisions of
2026-11-11). Ask Vestra itself is the built-in Vestra Player AI plugin's (`src/plugins/workshop`);
the host adds three general pieces that plugin is built from. Names are by what they do, not by
Ask Vestra, so a calculator, a library search or a jump-to-time row can use them later (PROPOSAL
§10).

## 1. Built-in-only requests

`Info::builtin_only` (`capabilities/mod.rs`) marks an area of the API (`commands_run.rs`:
`commands.list`, `commands.run`; `palette_lead.rs`: `palette.lead.*`) that only built-in
plugins may use. Since player 0.9.0 (SDK 1.9.0) manifests declare nothing, so this is the only
refusal by who is asking; until then they were two built-in-only manifest declarations. Built-in means **by provenance**: `Host::is_builtin` (listed in `builtin.json`,
running from that directory, recorded built-in in the store). A manifest field or a load mode never
makes a plugin built-in, so a development load of a built-in's id is refused too (PROPOSAL Q10 A).

Where it is checked:

```mermaid
flowchart TB
    R["request from a plugin"] --> D["Host::dispatch: route"]
    D --> B{"its area builtin_only?"}
    B -- "no" --> H["area handle"]
    B -- "yes, Host::is_builtin" --> H
    B -- "yes, not built-in" --> E["error builtin-only: for built-in plugins only"]
    P["recorded call at start-up"] --> RP["capabilities::replay"]
    RP --> RB{"builtin_only and not is_builtin_package(origin)?"}
    RB -- "yes" --> E
    RB -- "no" --> CR["area replay"]
```

Until player 0.9.0 `packages.validate` also refused a third-party package whose manifest declared
one of the two; with the field gone there is nothing to refuse at install, and the
requests themselves are refused as above. TestHost refuses them the same way (`TestHost.builtin`).

## 2. `commands.run` (built-in only)

`capabilities/commands_run.rs`. Holds nothing for standby.

| Method | Params | Result | Errors |
| --- | --- | --- | --- |
| `commands.list` | `{}` | `{ revision, commands: [{ id, label, labelSource, title, category, path, place, keys, plugin, builtin, enabled, reason?, checkable, checked?, radio, group, busy, dangerous }] }` | — |
| `commands.run` | `{ id, ticket, step?, of?, then? }` | `{ ran: true }` or `{ ran: false, declined: true }` | `missing` (also an unknown `then` id, with `detail.id`), `disabled` (with `reason`), `busy`, `invalid` (the palette's own command, also in `then`; bad `step`/`of`/`then`), `ticket`, `limit` |

- The list is the palette's (`Host::palette_items`, `ui/command_palette.rs` `items`): the same
  commands, labels and places. `id` is the global id `<manifest>/<local>` (`host/<id>` for the
  player's). `revision` is the registry revision. `keys` are every active key label
  (`bindings::Table::labels`). `builtin` says whether the command's plugin is built-in. `dangerous` is for this caller (§2.2: it
  follows the plugins it trusts) and does not move `revision`, which is the registry's.
- **Personal data**: the host cannot tell which commands a plugin generated from the user's data
  (a Recently Played item titled with a file name), so the rows carry no such mark. A caller that
  must keep personal data from a model recognises them by owner and place: it leaves out the rows
  whose `path`/`place` is a plugin's list of files (Recently Played, a playlist) and the rows of
  plugins it does not know, and sends the rest. A general mark (`CommandOptions.personal`) would be
  a later addition once a second user needs it.
- A run follows `PaletteAction::Run`: the same `enabled` and `busy` checks, the palette's own
  command refused, then `Host::execute_command`. It does not touch the palette's recent list.
- **Tickets** (`CommandRuns`): minted only by a user's activation of the palette lead (§3), bound
  to the lead's holder, one live ticket per plugin (a newer request ends the older). A ticket ends
  on `palette.lead.done`, when its plugin stops or parks, or 120 s after it was minted or last ran a
  command. A parked holder woken by the lead event keeps it (`rekey`).
- **Rates**: 4 runs a second per ticket, 30 per ticket, 120 a minute per plugin (`limit`). A run
  counts when it passes the command checks, before any confirmation.
- **Which commands ask** (`dangerous`): a command marked `CommandOptions.destructive`, the
  player's own Close (`host/stop`) and Quit (the host's list names only the player's commands),
  anything under the built-in `@settings` and `@reset` menus (judged on the whole path normalized,
  `commands::normalize`: an old root "设置" with items below it is Settings), and every command of
  a plugin that is not built-in (PROPOSAL Q3 A) unless the caller trusts that plugin ("Don't ask
  again", §2.2: then only its destructive commands ask). The core never lists plugin commands: **plugins mark their
  destructive commands with `CommandOptions.destructive`**, and the built-in plugins mark theirs
  (Remove All, Clear History, Delete, …) before any caller uses `commands.run` (a gate the
  coordinator records).
- **Plans** (`then`, SDK 1.7.16; the SDK's TestHost has the same rules and messages): the global
  ids the same ticket runs next, in order. Checked in this order: the ticket given, `step`/`of`,
  `then`'s shape, the command, `then`'s ids, then the ticket and the rates (`count`); a refused
  call is not counted. Absent or null names no new plan; `[]` is a new plan of this command
  alone (it drops the rest of the old one and asks only if this command asks). Shape errors are
  `invalid`:
  "then is an array of command ids" (not an array), "then has at most 7 command ids" (with the
  run's own at most `PLAN_STEPS`, 8: the sheet's rows and `COMMAND_RUN_LIMITS.plan`), "then has
  command ids: non-empty strings". Each id must name a listed command (`missing`, "no command
  <id>", `detail.id`) other than the palette's own (`invalid`); repeats are allowed; its state
  (enabled, busy) is checked only when it runs, as an earlier step may change it.
- **Asked once, before the first step** (§2.1): a call with `then` drops what is left of the
  ticket's earlier plan. When any command of `[id, ...then]` asks, the host shows the approval
  sheet for the whole plan before anything runs; Run runs `id` and keeps `then` as the ticket's
  plan (repeats counted); Don't Run runs nothing, keeps no plan and answers
  `{ ran: false, declined: true }` (the caller ends the request). When none asks there is no sheet
  and no plan is kept. A call without `then` whose id is in the plan uses up its first occurrence
  and runs without asking, even a destructive command (`CommandRuns::take_planned`); any other
  command asks as it would alone (a sheet of one step), so a used-up plan entry asks again. The
  plan lives as long as the ticket (done, a new request, the plugin stopping or parking, 120 s
  unused). The caller cannot answer, skip or word the sheet. The ticket and the command are
  checked again after the answer. One sheet at a time, whoever asks: a run that would ask while any
  sheet waits fails `busy` ("another command is waiting for the user's answer"), decided before
  it is counted or touches the plan (`CommandRuns::admit`); the waiting sheet is the user's open
  question, so a later caller neither replaces it nor answers it in the user's place;
  an open native dialog no longer refuses a run (PROPOSAL F4): the sheet waits for it. Headless
  there is no sheet: `VESTRA_TEST_CONFIRM=run` answers Run for the whole sheet (tests), `always`
  Run with "Don't ask again" checked (remembered only where the sheet would offer it, §2.2),
  anything else Don't Run; `commands.run: <id> asks to run <global>, <global>, … (headless:
  <answer>[, remember])` goes to stderr.
- **Where the run is named**: when the caller holds the palette lead (its ticket is the lead's),
  the step goes to the status strip (§3.1): "i/n · <command label>" with `step`/`of`, the label
  alone without, the line filled to i/n (full for a lone command). Any other caller (none yet: both
  are built-in only and only the lead mints tickets) gets the OSD line
  "<plugin> · <command label>" (or "<plugin> · i/n · <label>"). Either way the line shows before
  the command runs, and `commands.run: <id> ran <global> (strip: …)` (or `(osd: …)`) goes to
  stderr and `commands.run: <id> ran <global>` (with ` (step i/n)`) to the host log: ids only,
  never the label or the line, which may name the user's file (README §4.3, Host log).

`CommandOptions.destructive` (`commands::Declaration::destructive`, optional, absent = false) is
the general part: any plugin may mark a command, and other places (the bar's `»` menu) may ask
before running such a command later.


### 2.1 The approval sheet

Design: `C:\vestra\design\ask-vestra-ux\PROPOSAL.md` §4.3, §4.6, §4.7 (user rulings of
2026-11-12; step 3: H3, H7). `host/review_sheet.rs`, `ui/review-sheet.slint`, `ui/host.slint`,
`capabilities/commands_run.rs` (`ask`, `reason`).

A menu-layer mode beside the palette (`MenuMode::Review`) in the main window, drawn by the host from
its own data: where the palette and the status strip are (centred, y 16, below the fullscreen
title strip while it is out), on the palette's plate at its width (520px, at most the window less
16px); the status strip yields to it as to any menu. No system dialog, so no system sound; it
follows the theme. From the top:

| Part | Content |
| --- | --- |
| Request box (the palette field's box, read-only) | the lead's icon, its title and a colon in semibold (13px, `text`), the request the ticket was minted for (`text-list`, elided) |
| Question (28px, 12px semibold) | "Run this command?" / "Run these {n} commands?" |
| Steps (28px each, 8 fixed slots) | number (11px `text-dim`), palette label (12px; `text` for a step that asks, `text-list` for one that does not), the plugin's shown name (11px `text-dim`), a reason (11px `text-dim`) |
| Note (11px `text-dim`, wraps), only when a step's plugin is not built in | "The player cannot tell what commands of plugins that are not built in will do." |
| Footer (the palette's separator; 44px, taller only when the checkbox's text wraps) | right: [Run `Enter`] [Don't Run `Esc`] (the library Button's plate with a `KeyHint`; confirm first, as `UrlInputRow`). Left, when offered (§2.2): the library `Check` "Don't ask again for {plugin}" / "… for {0} and {1}" / "… for these {n} plugins", unchecked each time the sheet shows. |

Reasons are facts only (H7; user ruling 2 of 2026-11-12): "Not built in" for a command of a plugin
that is not built in, "Closes the video" for the player's Close, "Quits the player" for its Quit;
none for a destructive command (no "Cannot be undone") nor for a step that does not ask.

Input (§4.6): it takes the keyboard focus; Enter runs (remembering when the box is checked), Ctrl+Enter
(⌘↵: Slint's `control` is Command on macOS) runs and remembers when the box is offered and is a plain
Run otherwise, Esc does not; a click toggles the box, Space does not (the focus stays on the sheet,
so a Space meant for pause cannot check it); held-down repeats count for
nothing (the Enter that sent the request); Run and Don't Run by pointer; a press outside it is Don't
Run. Nothing counts within 500 ms of it showing (`review_sheet::GUARD_MS`, Chromium's rule for
sensitive UI), each time it shows. Bindings pause while it shows (it is the menu layer), and the
palette and other menus do not open over it.

Edge cases (§4.7): while a native dialog is open (`dialog_busy`) it waits, and steps aside if one
opens over it; it shows again when something else closed the menu layer (a rebuilt shell, another
menu); it stays when the window is not active and takes no focus from another program. Only an
answer, the end of the ticket (120 s unused, `palette.lead.done`: answered Don't Run) or the end of
the caller (it stopped: the sheet just goes) removes it (`review_tick`, every window tick). The
answer goes the usual way, `Event::CommandRunAnswered` (caller, run, remember). stderr gets the number of steps asked
about, never the labels or the request.

Offline renders: `widgets_tests::render_review_sheet_states` (ignored; `VESTRA_SHEET_SHOTS=<dir>`,
`VESTRA_SHEET_LANG=en|zh-Hans`, `VESTRA_SHEET_BACKDROP=<png>`): a plan of three steps with one not
built in (and a destructive one: no checkbox), one destructive step, one step not built in (with the
checkbox), two steps with the checkbox checked (`r04-remember-one-checked`), three steps with two
plugins to remember (`r04-remember-two`), Close, eight steps; both palettes, 2×.

### 2.2 Don't ask again

Design: PROPOSAL §3.2 B3, §4.3, §4.5, §4.6, §4.7, §5.1 H4 (step 4; user rulings of 2026-11-12: per
plugin, kept until reset by a palette command; never offered with a destructive step).
`capabilities/commands_run.rs` (`Judged`, `trust_offer`, `trust_plugins`, `ask_again`,
`forget_trust`), `ui/ui_state.rs` (`trustedPlugins`), `host/review_sheet.rs`
(`remember_text`, `decide`), `host/actions.rs` (`ASK_AGAIN`), `packages/package_manager.rs`
(`uninstall`, `source_changes`, `trustable_plugin`).

- **What it trusts**: per caller (the `commands.run` caller's manifest id), the plugins (manifest
  ids) that are not built in whose steps on the sheet ask only for that reason, in plan order, each
  once. The host keeps that list with the waiting run (`ConfirmedRun::remember`) and owns the box
  (`Review::remember_checked`, unchecked for every sheet): a click asks it to toggle
  (`toggle-remember`) and, like every other input, counts for nothing within the 0.5 s guard
  (`review_sheet::decide`). The view cannot widen the list. At the answer the list is resolved
  again: only plugins still installed (not built in) or loaded for development are written, so one
  uninstalled while the sheet was open is not trusted again.
- **When it is offered**: when the list is not empty and no step of the plan always asks
  (`always_asks`: a command marked `destructive`, the player's Close and Quit, anything under the
  built-in `@settings` and `@reset` menus), trusted plugin or not (user ruling 3 of 2026-11-12). A
  trusted plugin's destructive command keeps asking, without the box.
- **What it changes**: `dangerous(item, builtin, trusted)` is false for a trusted plugin's ordinary
  command, so that caller's runs no longer ask and its `commands.list` rows say `dangerous: false`
  (a model sees no "asks the user first" for them). Built-in rules are unchanged; trust never applies
  to a built-in (nothing to trust).
- **Where it lives**: `ui-state.json` `trustedPlugins` (README §ui-state; headless runs keep it in
  memory only), written 1 s after Run and on exit, so it survives restarts; written at once when trust
  is removed (Ask Again, an uninstall, a replace), so a crash cannot bring it back. At most 256 plugins per
  caller (the oldest go first); an unreadable entry is dropped alone.
- **How it ends**: `Command Palette: Ask Again Before Plugin Commands` (`host/ask-again`, palette
  only, no menu path or default key; also runnable through `commands.run`, harmlessly) clears every
  caller's list and shows the OSD line "Plugin commands will ask again". Uninstalling a plugin
  (`packages.uninstall`, with or without its data) removes it from every list, and its own list as a
  caller. Installing it from another source (`packages.install` with `replace: true`, a new source
  identity: another plugin to the user) removes it too, at the request; an update from the same
  source keeps it (PROPOSAL §4.7), and so does disabling. Trust is by manifest id: a development
  load (`--plugin DIR`) of a trusted id inherits it.
- **Host log**: `commands.run: <caller> no longer asks before the commands of <id>, …` and
  `commands.run: plugin commands ask again` (manifest ids only).

This record exists to ask less; it is not a security boundary (plugin processes are not sandboxed,
rulings of 2026-11-03/04): the store review and uninstall remain the gate.

### 2.3 The main window's confirmations

Design: PROPOSAL §4.4, §5.1 H5, §5.4 step 5, §7 Q5/Q6 (user rulings of 2026-11-12: Ask Vestra's
first-use consent in the player; every other main-window confirmation too, as a separate step).
`capabilities/ui_dialog.rs` (`ask`), `host/review_sheet.rs` (`Asks::Confirm`, `confirm_sheet`,
`open_confirm`), `host/origins.rs`, `platform::tool_window_focused`. No API change:
`dialogs.confirm` keeps its parameters and its answer `{ confirmed, checked }`.

- **Which window's**: the window where the user last acted for the caller (`host/origins.rs`), not
  the focus when the request arrives: a confirmation reaches the host after the click has gone to
  the plugin's process and back, sometimes after a download or a check, and a click in the main
  window meanwhile, or the player in the background, sent the Plugin Store's Update to the main
  window's sheet, under the store window (tool windows stay above the main window), while the
  store's next click was refused `busy` (bug of 2026-11-12, player 0.8.1). What counts, newest
  first: a UI callback of the plugin's window, of its settings page (Options) or of its panel (the
  main window, or the floating sidebar's window); its command run from a menu, the command
  palette, a key, a bottom-bar button or a message's button (the window of that menu or key); a
  request typed in the palette's first row, or the status strip's Stop, for the lead's holder (the
  main window: Ask Vestra's first-use consent and the confirmations of the commands it runs go to
  the sheet with the request box, also after the Workshop's own window was shown); the plugin
  showing its own window (`ui.open`, `ui.show` visible): the user then looks there. A command
  another plugin runs (`commands.run`) takes the caller's origin, or none when the caller has
  none. Only a running or parked plugin's input counts (a stopped owner's, or the host's own
  commands', records nothing). A tool window still shown (a plugin's window, Options, the
  floating sidebar) gets a system dialog owned by it, brought to the front first (with the focus
  while the player is in the foreground, a taskbar flash or Dock bounce otherwise): Windows, the
  quiet task dialog (step 1); macOS, an `NSAlert` sheet on that window. The main window gets the
  sheet of §2.1 and comes forward the same way; a tool window over the sheet's place stays above
  it (Windows owned windows, the floating level on macOS), which is acceptable because the sheet
  is only for requests made in the main window, where the user is looking. An origin whose window
  closed falls back to the plugin's own window still shown, and with neither (a timer's request
  of a plugin without a window) to the earlier test: a tool window with the focus (Windows, a host
  window other than the main one with winit's `has_focus`; macOS, the visible key window without
  a sheet, one of winit's other than the main one) gets the system dialog, else the main window
  the sheet, also when the player is not in the foreground (it then takes no focus). Entries are
  keyed by owner generation: a parked plugin's moves to its woken generation, and it goes when
  the plugin stops. On other systems there are no system alerts, so every confirmation is drawn on
  the sheet (before: `unsupported`). Headless: still `unavailable`.
- **What it shows**: on the palette's plate at its place, the caller's message (12px semibold,
  28px, taller when it wraps) and detail (11px `text-list`, wraps), its checkbox at the footer's
  left, [confirm `Enter`] [cancel `Esc`] at its right. The request box (§2.1) only when the caller
  holds the lead, the user last acted for it in the palette's first row (`origins::Origin::Lead`:
  a request typed there or the strip's Stop; an action in its docked panel since then is plain
  `Main`) and that request's ticket is live (Ask Vestra's consent, asked while the request waits
  for it): its title and the request. `destructive` draws nothing: no reason label and no colour (user
  ruling 2 of 2026-11-12; the approval sheet marks no destructive step either); Enter confirms
  either way, as the system dialog's default button did. The panel ends 16px above the window's
  bottom at most: a longer detail (up to 2000 characters)
  scrolls in a std-widgets `ScrollView` (at least 48px of it shown), and the footer (checkbox and
  buttons) stays in view. When the window cannot keep the message and 48px of detail (the
  smallest window, 320×182, with a long message), the message scrolls with the detail; a plan's
  steps and note scroll the same way under its question. While the body scrolls its text is
  narrower by the scroll bar's 14px track, which never covers it (the decision is made on the
  texts measured at their full width). The buttons are the library `Button` with its `key` (a
  `KeyHint` after the text).
- **Into view**: the first time a sheet (a plan's or a confirmation's) shows, the main window
  comes into view (`view::show_again`): un-minimized and in front while the player is in the
  foreground, asking for attention otherwise (the taskbar button flashes, the Dock icon bounces),
  so a sheet never waits unseen in a minimized or covered window; not again when it shows again
  after another menu or a native dialog.
- **Input and lifetime**: as §2.1 (the focus, the 0.5 s guard, repeats, a press outside it is
  Cancel, the native-dialog wait, showing again after another menu). The checkbox is the caller's:
  a click toggles it (Space does not), its state is answered with Confirm and with Cancel, and
  Ctrl+Enter is a plain Enter. The end of the request's ticket (with a request box) answers Cancel;
  a caller that stops (`Capability::revoke`) leaves it unanswered and it goes.
- **One at a time**: while a sheet waits, a plan's or a confirmation's, `dialogs.confirm` (the
  main window's or a tool window's) and a `commands.run` that would ask are refused `busy`
  ("another confirmation is waiting for the user's answer"), as `dialogs.confirm` is while a system
  dialog is open. A refusal brings the confirmation to answer first into view, so a wait is never
  somewhere the user cannot see: the sheet's main window, or the open system dialog itself, whichever
  window owns it (Windows: the owner's last active popup, since the owner is disabled while it is
  open; macOS: the window with the sheet), to the front while the player is in the foreground,
  asking for attention otherwise. A waiting confirmation keeps plugins running as an open dialog
  does (`Busy::Dialog`).
- stderr gets the caller's id, never the texts (they may name the user's file).

Offline renders: `widgets_tests::render_confirm_sheet_states` (ignored; the same variables as
§2.1): Ask Vestra's consent (`r07-consent`), a plain confirmation (`h5-plain-replace-list`), a
destructive one with its checkbox unchecked and checked (`h5-destructive-checkbox`,
`h5-destructive-checked`); both palettes, 2×; and, English only, a 2000-character install detail
in a 640×360 window (`h5-long-detail-small`).

## 3. `palette.lead` (built-in only, residency recordable)

`capabilities/palette_lead.rs`, `host/palette.rs`, `ui/command_palette.rs`,
`ui/command-palette.slint`. Store label "Add the first row of the command palette".

| Method / event | Params | Notes |
| --- | --- | --- |
| `palette.lead.set` | `{ title (1–40), icon, placeholder? (≤60), empty? (≤80), note? (≤40), status?: { text (1–80; an end state's 1–160), note? (≤40), state?, strip? (1–80, working only) } | null }`\| null }` | Texts are strings (translated with the plugin's catalog) or text values; `icon` is a name of the icon set (`BUTTON_ICONS`) or a `plugin.ui.image`. One holder: another plugin's `set` fails `conflict` with `reason: "lead-taken"` (the slot refusals' form; PROPOSAL said `taken`). Recorded and replayed, less an end-state `status` (§3.1). |
| `palette.lead.clear` | `{}` | The row and its placeholder go. |
| `palette.lead.done` | `{ ticket }` | Ends the ticket. |
| event `palette.lead` | `{ query, keep, ticket, status }` | Sent to the holder on Enter, Shift+Enter (`keep`) or a click on the row; `status` when the row showed its status. A trigger (`TRIGGERS` in `host/supervisor/mod.rs`): it wakes a parked holder. |

The palette with a lead (nothing changes without one):

- Row 0 is the lead, drawn above the command rows and never scrolled away: the icon in the check
  column (accent), "title: query" in semibold and plain (the title alone and `empty` dimmed for an
  empty query), `note` on the right; while a working `status` is set and the query is empty, the
  status text and note instead (`command_palette::lead_shows_status`). The event's `status`
  follows the same rule. Shift+Enter on the row clears the field after sending (`run_lead`;
  PROPOSAL F1, Ask Vestra UX H6), so the row shows the status of the request just sent and a
  second Enter stops it instead of sending it again: the cursor stays on the lead row, and until
  that request's status has come and gone (or something is typed, or it is stopped) Enter there
  with nothing typed sends `status: true`, also before the status arrives (a parked holder takes
  a moment to wake; `PaletteState::sent`). Typing another query is another request. It
  takes one of the 12 slots; `Results: N` counts commands only; "No matching commands" moves to
  row 1; the field shows `placeholder`.
- The cursor (`command_palette::aim`): on the first command the query names (its title or full
  label equals the query, or its title continues the query from a word start for 3 or more
  characters, case ignored, category not part of the title: "pause" goes to Playback:
  Play/Pause), else on the lead. With an empty query it is on the first command (the most recent,
  so Enter repeats the last command as before), or on the lead while it shows a status. It is
  placed again after opening, typing, when the lead comes or goes, and when its status comes or
  goes while nothing is typed (the status arrives after Shift+Enter cleared the field); not by
  Shift+Enter itself, which leaves it on the lead row.
- `PaletteState::cursor` counts the lead as row 0; `Run(slot)` from the view uses the same slots
  (the lead's is 0), so the command rows' slot is `i + 1`.
- The bottom bar's palette cell shows `search-ai` (a speech bubble with the AI sparkle breaking out at its
  top right, design B) and the tip "Command Palette… and <title>" while a lead is set; `search` otherwise.
- Headless tests activate the row with `VESTRA_TEST_EXECUTE=lead:<query>@<ms>`
  (`host/supervisor/probe.rs`), as Enter would.

### 3.1 `status.state` and the status strip

Design: `C:\vestra\design\ask-vestra-ux\PROPOSAL.md` §4.2 (user rulings of 2026-11-12, step 2:
H1, H2, H6; SDK 1.7.15 `PaletteLeadStatus.state`). `capabilities/palette_lead.rs` (`LeadStatus`,
`StatusState`), `host/lead_strip.rs`, `ui/lead-strip.slint`, `ui/host.slint`.

`status.state` is `"working"` (absent or null: the default), `"done"`, `"failed"`, `"note"` or
`"stopped"` (SDK 1.8.0: the user stopped the work; before it Ask Vestra sent a `note`, which read
like an answer); any
other value fails `invalid` ("status.state is one of working, done, failed, note, stopped"), as the SDK's
TestHost does. A working status is the row's status (above) and the strip's text. An end state
(done, failed, note, stopped) is shown **once** in the strip and never on the row: `set` takes it off the
row and out of the recorded call (`Lead::sent`), so a parked or restarted holder is never shown it
again, and the row goes back to its usual form at once. The same end status sent again by the next
`set` (an update of other fields: the SDK's `update` sends them all) is not shown again
(`Lead::ended`); a new request (a new ticket) forgets it, so the same failure twice in a row shows
twice. The holder needs no timer and may call `palette.lead.done` right away.

**The texts' lengths and the strip's own text (SDK 1.8.0).** `TITLE_CHARS` … `STATUS_STRIP_CHARS`
in `capabilities/palette_lead.rs` are the SDK's `PALETTE_LEAD_LIMITS`
(`tests/sdk/lead-status-agreement.test.ts`); each text is counted as shown and is one line as
sent.

- `status.text` of a working status: 1–80 characters, one line of the row and of the strip.
- `status.text` of an end state: 1–160 characters. It shows in the strip alone, which wraps it
  to at most two lines and elides what does not fit (before 1.8.0 an end state's text was
  limited to 80 like a working one, although the strip already wrapped it).
- `status.strip`: optional, 1–80 characters, for a working status only (`LeadRow::strip`). The
  strip shows it in place of `text` (`LeadRow::working_strip`); the palette's row always shows
  `text`. So the row can read "Vestra is working · Planning…" and the strip "Planning…". With an
  end state it fails `invalid` ("status.strip is for a working status"): an end state's `text`
  is already the strip's. A null `strip` is none. It is part of the status: it goes with it
  when the holder is parked, from the row and from the recorded call.

`palette_lead::parse` checks in the order it had before 1.8.0, so a request with several faults
is answered as it was: unknown fields, `title`, the icon, then the status (its fields' names,
`text`, `note`, `state`), then `placeholder`, `empty`, `note`. The additions sit inside the
status: the longer limit applies to `text` when `state` names an end state (a state that is
refused keeps the 80), and `strip` is checked after `state`, first that the status is a working
one, then its length ("status.strip must be 1–80 characters on one line").


The strip is where the palette was: centred, y 16 (below the fullscreen title strip while it is
out), above the video, the OSD and the title strip, below the menu layer; the palette's plate
(`bg-panel`, 1px `line`, 8px corners, the menu shadow), 12px text, 32px high (an end state up to
two lines, 48px), 168–420px wide (an end state up to 520px), at most the window's width less
16px. It takes no keyboard focus. What it shows (`lead_strip::project`, first match wins):

| Source | Icon | Line (2px `accent-fill`, 8px in from each side) | Stop | Time |
| --- | --- | --- | --- | --- |
| An end state: done / failed / note / stopped | `menu-check` accent / `failed` danger / `ai` accent / `ai` dimmed (`text-dim`) | full / full `danger` / none / none | no | 2 s / 6 s / 4 s + 50 ms a character, at most 10 s / 2 s (`STOPPED_MS`: the user pressed Stop, it only confirms that); held while the active pointer is on it (the OSD's hold, at most 30 s); then a 300 ms fade |
| The step `commands.run` last ran for the holder with a live ticket | `ai` accent | filled to step/of, 150 ms per change | yes | until the ticket ends (done, a new request, 120 s idle, the holder stops or parks), an end state or `status: null` |
| The holder's working status (its `strip` text, else its `text`) | `ai` accent | a 36% segment sweeps left to right, 1.4 s a pass, easing in and out, its ends fading into the plate | yes | while it is set |

From the first step on, a working text the holder sends changes only the palette's status row,
not the strip. A working status that arrives while an end state shows, with no step, replaces it
(a new request). The strip yields to any open menu or the palette (the palette's first row shows
the working status then) and comes back when it closes, its time running on. Presses: Stop sends
the `palette.lead` event with `status: true` and an empty query, as Enter on the status row does
(a new ticket; the holder stops its request); a press elsewhere opens the palette, or on an end
state removes it at once; nothing while it fades out. Every element stays (`visible` and widths
change); the host writes the `lead-strip` struct only when it changes, and again into a rebuilt
shell (`write_strip`). While it shows, the OSD message's text ends 8px left of it.

**Motion and frame rate.** The sweep is the only lasting animation: the segment's `x` reads
`animation-tick()` only while the strip shows and works, so nothing else asks for frames (a step,
an end state, a hidden strip: none; `widgets_tests::the_lead_strip_animates_only_while_it_sweeps`).
`widgets_tests::lead_strip_sweep_frame_cost` (ignored, `VESTRA_STRIP_BENCH=1`) draws the window
over a video still with Slint's software renderer at 60 Hz pacing and prints the draw times, whole
frame and changed region. With the system's animations off (Windows' Animation effects, macOS'
Reduce motion: `platform::system_animations`, read at most once a second while the strip would
sweep) the segment rests in the middle (`LeadStrip.still`). A holder parked (or restarting) while
its status says it works keeps its row, but the working status goes from the row, the strip and the
recorded call (`Lead::park`): its tickets ended with its process, so nothing is working.

Offline renders: `widgets_tests::render_lead_strip_states` (ignored; `VESTRA_STRIP_SHOTS=<dir>`,
`VESTRA_STRIP_LANG=en|zh-Hans`, `VESTRA_STRIP_BACKDROP=<png>`) draws each state at 2×, both
palettes, as the spec's `proposed/detail-*.png` do.


Why both stay built-in only for now (PROPOSAL §10): one lead slot, the palette's most valuable
row, and a third-party lead would see every query; running any plugin's command is a wider
boundary than any other request, and plugin processes are not sandboxed, so the boundary is the
player's (tickets, rates, confirmation, the line naming each run). Opening either needs the user's ruling.

## 4. Tests

- Unit: `capabilities/commands_run.rs` (tickets, idle end, rates, revoke and rekey, which commands
  ask first, trusted plugins, what "Don't ask again" offers, `step`/`of`, `then`, plan approvals,
  the sheet's reasons), `host/review_sheet.rs` (the guard, the question, the sheet's value, the
  checkbox's text; a confirmation's value and its checkbox answered with either button), `ui/ui_state.rs` (`trustedPlugins`: per caller, bounded, uninstall, reset, lenient
  reading, round trip), `capabilities/palette_lead.rs` (parameters, built-in only),
  `capabilities/mod.rs` (built-in-only refusal for live requests and replays),
  `capabilities/palette_lead.rs` (`status.state`), `host/lead_strip.rs` (what the strip shows, end
  states' times, the shell value), `ui/command_palette.rs` (`names`, `aim`, `lead_shows_status`, no
  change without a lead), `ui/widgets_tests.rs` (the strip's sizes, presses, fade and animation; the sheet's size, keys,
  repeats and buttons; the checkbox: click, Run, Enter, Ctrl+Enter, Space, a wrapping text; a
  confirmation's size with and without the request box and detail, its keys and buttons; a
  2000-character detail in a small window scrolls and keeps the buttons in it; the smallest window
  scrolls a 200-character message with its detail and a plan's eight steps, the scroll bar never
  covers the text; `Button`'s key cap),
  `host/supervisor/probe.rs`. `src/tests/sdk/host-agreement.test.ts`: TestHost accepts the
  player's `StatusState::NAMES` and refuses another with the player's code and message (skipped
  before SDK 1.7.15).
- Integration, ignored by default (`npm run ci:local`): `src/host/tests/palette_lead.rs`, the
  vertical slice with the real host headless: a built-in fixture sets the lead, gets the query and
  a ticket, runs its own command (the strip's line), a destructive one after the confirmation, hits
  the rate limit, an unknown `status.state` and an end state, and the ended ticket; a development load is refused both capabilities; a parked
  holder is woken by the row and its ticket works in the woken run; Don't Run declines; a plan
  (`then`) is refused when malformed, asked once before its first step, its approved steps run
  without asking and a step outside it asks again; Don't Run on a plan runs nothing; "Don't ask
  again" on a development load's plain command (it stops asking and listing as `dangerous`, its
  destructive command still asks), then Ask Again Before Plugin Commands (both in the host log).
- `package_manager.rs`: a development load of a built-in's id (even of the installation's
  directory) is not built-in (`builtin_provenance`: the run must use the built-in instance's data
  key).
