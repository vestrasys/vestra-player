# Palette lead, `commands.run` and built-in-only capabilities (maintainers)

Host side of Ask Vestra (design: `C:\vestra\design\ask-astra\PROPOSAL.md`, user decisions of
2026-11-11). Ask Vestra itself is the built-in Vestra Player AI plugin's (`src/plugins/workshop`);
the host adds three general pieces that plugin is built from. Names are by what they do, not by
Ask Vestra, so a calculator, a library search or a jump-to-time row can use them later (PROPOSAL
§10).

## 1. Built-in-only capabilities

`Info::builtin_only` (`capabilities/mod.rs`, exported as `builtinOnly` in
`src/host/capabilities.json` and the SDK's `CAPABILITY_INFO`) marks a capability only built-in
plugins may use. Built-in means **by provenance**: `Host::is_builtin` (listed in `builtin.json`,
running from that directory, recorded built-in in the store). A manifest field or a load mode never
makes a plugin built-in, so a development load of a built-in's id is refused too (PROPOSAL Q10 A).

Where it is checked:

```mermaid
flowchart TB
    R["request from a plugin"] --> D["Host::dispatch: route, declared check"]
    D --> B{"needed capability builtin_only?"}
    B -- "no" --> H["capability handle"]
    B -- "yes, Host::is_builtin" --> H
    B -- "yes, not built-in" --> E["error capability: for built-in plugins only"]
    P["recorded call at start-up"] --> RP["capabilities::replay: declared check"]
    RP --> RB{"builtin_only and not is_builtin_package(origin)?"}
    RB -- "yes" --> E
    RB -- "no" --> CR["capability replay"]
    V["packages.validate / validateArchive"] --> VB{"manifest declares a builtin_only capability?"}
    VB -- "yes" --> E
```

`packages.validate` (`package_validate.rs`) validates only third-party packages (built-in ids are
refused before), so a store or file install of a package declaring one fails there with
`capability`. `vestra-plugin check` gets the same rule from the SDK's `CAPABILITY_INFO`.

## 2. `commands.run` (risky, built-in only, residency none)

`capabilities/commands_run.rs`. Store label "Run any player or plugin command".

| Method | Params | Result | Errors |
| --- | --- | --- | --- |
| `commands.list` | `{}` | `{ revision, commands: [{ id, label, labelSource, title, category, path, place, keys, plugin, builtin, enabled, reason?, checkable, checked?, radio, group, busy, dangerous }] }` | — |
| `commands.run` | `{ id, ticket, step?, of? }` | `{ ran: true }` or `{ ran: false, declined: true }` | `missing`, `disabled` (with `reason`), `busy`, `invalid` (the palette's own command, bad `step`/`of`), `ticket`, `limit` |

- The list is the palette's (`Host::palette_items`, `ui/command_palette.rs` `items`): the same
  commands, labels and places. `id` is the global id `<manifest>/<local>` (`host/<id>` for the
  player's). `revision` is the registry revision. `keys` are every active key label
  (`bindings::Table::labels`). `builtin` says whether the command's plugin is built-in.
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
- **Confirmation** (`dangerous`): the host's own alert (`platform::confirm`, not destructive
  style: no icon on Windows, so no system sound): the question "Run “<command>”?" as its title, then
  "<caller> asks, for your request “<request>”." and, for a plugin that is not built in, that the
  player cannot tell what its command does;
  "Run" / "Don't Run". It comes before a command
  marked `CommandOptions.destructive`, the player's own Close (`host/stop`) and Quit (the host's
  list names only the player's commands), anything under the built-in `@settings` and `@reset`
  menus, and every command of a plugin that is not built-in (PROPOSAL Q3 A). The core never lists
  plugin commands: **plugins mark their destructive commands with `CommandOptions.destructive`**, and
  the built-in plugins mark theirs (Remove All, Clear History, Delete, …) before any caller uses
  `commands.run` (a gate the coordinator records).
  The caller cannot answer or skip it. "Don't Run" answers `{ ran: false, declined: true }`. The
  ticket and the command are checked again after the answer. Headless there is no alert:
  `VESTRA_TEST_CONFIRM=run` answers Run (tests), anything else Don't Run.
- **Where the run is named**: when the caller holds the palette lead (its ticket is the lead's),
  the step goes to the status strip (§3.1): "i/n · <command label>" with `step`/`of`, the label
  alone without, the line filled to i/n (full for a lone command). Any other caller (none yet: both
  capabilities are built-in only and only the lead mints tickets) gets the OSD line
  "<plugin> · <command label>" (or "<plugin> · i/n · <label>"). Either way the line shows before
  the command runs, and `commands.run: <id> ran <global> (strip: …)` (or `(osd: …)`) goes to
  stderr and `commands.run: <id> ran <global>` (with ` (step i/n)`) to the host log: ids only,
  never the label or the line, which may name the user's file (README §4.3, Host log).

`CommandOptions.destructive` (`commands::Declaration::destructive`, optional, absent = false) is
the general part: any plugin may mark a command, and other places (the bar's `»` menu) may ask
before running such a command later.

## 3. `palette.lead` (built-in only, residency recordable)

`capabilities/palette_lead.rs`, `host/palette.rs`, `ui/command_palette.rs`,
`ui/command-palette.slint`. Store label "Add the first row of the command palette".

| Method / event | Params | Notes |
| --- | --- | --- |
| `palette.lead.set` | `{ title (1–40), icon, placeholder? (≤60), empty? (≤80), note? (≤40), status?: { text (1–80), note? (≤40), state? } \| null }` | Texts are strings (translated with the plugin's catalog) or text values; `icon` is a name of the icon set (`BUTTON_ICONS`) or a `plugin.ui.image`. One holder: another plugin's `set` fails `conflict` with `reason: "lead-taken"` (the slot refusals' form; PROPOSAL said `taken`). Recorded and replayed, less an end-state `status` (§3.1). |
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

`status.state` is `"working"` (absent or null: the default), `"done"`, `"failed"` or `"note"`; any
other value fails `invalid` ("status.state is one of working, done, failed, note"), as the SDK's
TestHost does. A working status is the row's status (above) and the strip's text. An end state
(done, failed, note) is shown **once** in the strip and never on the row: `set` takes it off the
row and out of the recorded call (`Lead::sent`), so a parked or restarted holder is never shown it
again, and the row goes back to its usual form at once. The same end status sent again by the next
`set` (an update of other fields: the SDK's `update` sends them all) is not shown again
(`Lead::ended`); a new request (a new ticket) forgets it, so the same failure twice in a row shows
twice. The holder needs no timer and may call `palette.lead.done` right away.

The strip is where the palette was: centred, y 16 (below the fullscreen title strip while it is
out), above the video, the OSD and the title strip, below the menu layer; the palette's plate
(`bg-panel`, 1px `line`, 8px corners, the menu shadow), 12px text, 32px high (an end state up to
two lines, 48px), 168–420px wide (an end state up to 520px), at most the window's width less
16px. It takes no keyboard focus. What it shows (`lead_strip::project`, first match wins):

| Source | Icon | Line (2px `accent-fill`, 8px in from each side) | Stop | Time |
| --- | --- | --- | --- | --- |
| An end state: done / failed / note | `menu-check` accent / `failed` danger / `ai` accent | full / full `danger` / none | no | 2 s / 6 s / 4 s + 50 ms a character, at most 10 s; held while the active pointer is on it (the OSD's hold, at most 30 s); then a 300 ms fade |
| The step `commands.run` last ran for the holder with a live ticket | `ai` accent | filled to step/of, 150 ms per change | yes | until the ticket ends (done, a new request, 120 s idle, the holder stops or parks), an end state or `status: null` |
| The holder's working status | `ai` accent | a 36% segment sweeps left to right, 1.4 s a pass, easing in and out, its ends fading into the plate | yes | while it is set |

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
frame and changed region. Not done yet: a system setting without animations (Windows "Show
animations") does not stop the sweep (`LeadStripView.still` is there for it).

Offline renders: `widgets_tests::render_lead_strip_states` (ignored; `VESTRA_STRIP_SHOTS=<dir>`,
`VESTRA_STRIP_LANG=en|zh-Hans`, `VESTRA_STRIP_BACKDROP=<png>`) draws each state at 2×, both
palettes, as the spec's `proposed/detail-*.png` do.


Why both stay built-in only for now (PROPOSAL §10): one lead slot, the palette's most valuable
row, and a third-party lead would see every query; running any plugin's command is a wider
boundary than any other capability, and plugin processes are not sandboxed, so the boundary is the
player's (tickets, rates, confirmation, the line naming each run). Opening either needs the user's ruling.

## 4. Tests

- Unit: `capabilities/commands_run.rs` (tickets, idle end, rates, revoke and rekey, which commands
  ask first, `step`/`of`), `capabilities/palette_lead.rs` (parameters, the capability's flags),
  `capabilities/mod.rs` (built-in-only refusal for live requests and replays, the JSON field),
  `capabilities/palette_lead.rs` (`status.state`), `host/lead_strip.rs` (what the strip shows, end
  states' times, the shell value), `ui/command_palette.rs` (`names`, `aim`, `lead_shows_status`, no
  change without a lead), `ui/widgets_tests.rs` (the strip's sizes, presses, fade and animation),
  `host/supervisor/probe.rs`. `src/tests/sdk/host-agreement.test.ts`: TestHost accepts the
  player's `StatusState::NAMES` and refuses another with the player's code and message (skipped
  before SDK 1.7.15).
- Integration, ignored by default (`npm run ci:local`): `src/host/tests/palette_lead.rs`, the
  vertical slice with the real host headless: a built-in fixture sets the lead, gets the query and
  a ticket, runs its own command (the strip's line), a destructive one after the confirmation, hits
  the rate limit, an unknown `status.state` and an end state, and the ended ticket; a development load is refused both capabilities; a parked
  holder is woken by the row and its ticket works in the woken run; Don't Run declines.
- `package_manager.rs`: a development load of a built-in's id (even of the installation's
  directory) is not built-in (`builtin_provenance`: the run must use the built-in instance's data
  key).
