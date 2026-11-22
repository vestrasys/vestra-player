# SDK refactor: three layers, three plugin styles, generated protocol docs

Design specification for the next refactor of `@vestra-player/sdk` (repository [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk), checkout `C:\vestra\sdk`). Written for the API maintainer who carries it out and for the coordinator who sequences it; plugin authors see only the results (new optional APIs, new documentation pages). The design decisions below follow the user's rulings of 2026-11-08 (§1.7); the schedule is **not** decided and this document does not propose one.

Status: 2026-11-08, approved design; Step 0 (the characterisation tests of §4.0) in progress; the refactor is the project's top priority with three dedicated agents. Sources: the researcher's inventory (`INVENTORY.md`) and layering investigation (`LAYERS.md`, both in `C:\vestra\design\sdk-refactor\`, written against SDK 1.6.12), re-verified against the SDK checkout at `package.json` 1.6.17 (HEAD `fb5bfdb`) and the player working tree (`6b142feb`). File:line references in this document are to those two trees unless marked "(report)", which means the number comes from a report and was not re-verified here. SDK paths are relative to the SDK repository (`src/plugin/ui.ts`), player paths to this repository (`src/host/src/…`).

## 1. Motivation and evidence

### 1.1 Release churn

82 tags between 2026-10-21 (`v1.0.0`) and 2026-11-08 (`v1.6.16`), 81 CHANGELOG entries, i.e. a release every working half-day for three weeks. Which files move with every release (INVENTORY §3, releases touched / lines changed):

| File | Releases | Lines changed | Why it moves |
| --- | --- | --- | --- |
| `src/plugin/types.ts` | 62 of 77 intervals | +11,210 / −1,175 | every API addition is types + doc comment + TestHost fake, in one file |
| `src/plugin/testing.ts` | 54 | +5,865 / −611 | the fake player mirrors every host rule by hand |
| `src/wire.ts` | 30 | +442 / −92 | every new host method |
| `src/commands.ts` | 22 | +843 / −55 | the built-in command table is a copy of the player's menu; 14 releases were "align with the host" only |

About 39 releases paired a new API with a player version, about 14 only re-aligned tables with the host, about 16 were documentation. The refactor does not aim to stop API growth; it aims to stop a one-line host change from needing a hand edit in three SDK files.

### 1.2 The framework is welded into the primitives

The SDK has three kinds of code, but the middle one does not exist as a layer (LAYERS §1, §8a):

- **L0, protocol**: `Transport` (`src/wire.ts:416-426`) and `createWire` (`src/wire.ts:441`) plus the per-domain clients (`src/source.ts`, `src/commands.ts`, `src/processing.ts`, `src/media.ts`, `src/producer.ts`): about 120 `transport.request` call sites, no state apart from subscription counting (`src/wire.ts:780-845`). Clean, complete, private: `@vestra-player/sdk/runtime` is "not part of the plugin API and not covered by semver" (`src/player-runtime.ts:1-8`).
- **L1, primitives**: a window or panel write, a command registration, a player event. These exist only as `protected` or private pieces of L2 classes.
- **L2, framework**: the update pass, dependency tracking, the row diff engine, the state mirrors.

The welds, each one line of evidence:

| Primitive | Where L2 is switched on unconditionally | Effect on a plugin |
| --- | --- | --- |
| View (window, panel, settings page) | `ViewBase` constructor: `scope.runtime.bind(() => this.evaluate())` (`src/plugin/ui.ts:456`) | every view joins every update pass, bound or not |
| View write | `set()` (`src/plugin/ui.ts:523-530`) shares `#values → flush → #encode → #send` with `bind()`; arrays are diffed into row patches (`src/plugin/ui.ts:663-667`); the raw `write(property, PropertyWrite)` is `protected abstract` (`src/plugin/ui.ts:459`) and only `WindowHandle.write` (`:891`) / `PanelHandle.write` (`:1032`) reach `host.ui.set` / `host.ui.setMount` | no way to push one value without the diff engine; a plugin that reached `host.ui.set` behind the SDK's back would desynchronise `#sent`/`#bases` and get `stale` fallbacks (`src/plugin/ui.ts:689-693`) |
| Command | `CommandHandle` constructor: `runtime.bind(() => this.#sync())` (`src/plugin/commands.ts:242`); `set()` calls `scope.update()` (`:293-296`), a full pass over every binding and every command | a command whose four state fields are constants still re-evaluates on every pass, and changing it re-runs everything else |
| Player state | `PlayerService` constructor subscribes `source.state` as a mirror (`src/plugin/player.ts:229-234`) and `createPlugin` forces the init of player, window and timeline mirrors (`src/plugin/index.ts:56-61`); the only listener API, `player.onChange`, runs through `scope.runOn` and dependency observation (`src/plugin/player.ts:564-572`) | a plugin cannot subscribe to one event without the mirror, nor get the raw payload |
| Runtime | one class `Runtime` (`src/plugin/runtime.ts:252`) is transport owner (`host`, `:253`), subscription counter (`on/hold/raw`, `:693/:752/:818`), pass scheduler (`bind/part/observe/invalidate/changed`, `:424-677`) and activation state machine (`starting/declaring`, `:372/:387`); the helpers `failure/Latest/quiet/uncertain/sleep` live in the same file, so every `src/plugin/*.ts` value-imports the scheduler (`src/plugin/stream.ts:6`, `src/plugin/settings.ts:2`) | the dependency graph shows no layers at all |

19 plugins (16 built-in, 3 third-party) never reach past the `plugin` object (LAYERS §6: zero hits for `.runtime` or `as any`), so the welds are not holding anything up today; they are what the refactor removes.

### 1.3 Manual `plugin.update()`

The framework's precise invalidation (`src/plugin/runtime.ts:442-677`, SDK 1.5.45) covers reads of the mirrored state (`plugin.player.state` and friends). It does not cover a plugin's own asynchronous or timer-driven state, so built-ins patch the gap with a full pass by hand. Count of `plugin.update()` / `scope.update()` in the built-ins (player working tree, `src/plugins/*`, tests and `workshop/vendor/` excluded):

| Plugin | Calls | Plugin | Calls | Plugin | Calls |
| --- | --- | --- | --- | --- | --- |
| subtitle-online | 18 | video | 7 | capture | 4 |
| workshop | 10 | info | 7 | hotkeys | 2 |
| playback | 9 | playlist | 6 | history, marks, store | 1 each |
| subtitles | 8 | visualizer | 5 | nes, preferences | 0 |
| window | 8 | audio | 4 | **total** | **91** |

(LAYERS §6 counted 84–91 on `origin/main`; the pattern is the same.) Typical shapes: `new Playback(player, plugin.dialogs, () => plugin.update(), …)` and `new Keyframes(plugin, playback, () => plugin.update())` inject the full pass as a callback (`src/plugins/playback/main.ts:33,62`); `setInterval(() => void readStats().then(() => plugin.update()), 1000)` (`src/plugins/info/dialog.ts:199`). Each call re-runs every binder and every command's `#state()` in that plugin. Third-party plugins have 0 calls: they lean on L1 services (`openStream`, `resolvers.add`, `player.open`) and keep UI state in a few variables.

### 1.4 Rules implemented twice

INVENTORY §2 lists 22 numeric rule sets (limits, id lists, icons, wake events, core actions…) and 10 behavioural rules (layout toggle, keepsRunning, key normalisation, command declaration checks, `plugin.when`, job states, processing validation, row patch application) that exist once in Rust and once in TypeScript (runtime, TestHost or both). All numeric rules agree today, held together by 42 hand-written tests in `src/tests/sdk/host-agreement.test.ts`, most of which read Rust source with regular expressions (for example `:54` matches `const MAX_PER_OWNER: usize = (\d+)` (report)). Only the capability table has a generated path (`scripts/capabilities-sync.mjs` → `src/host-capabilities.ts`, from `src/host/capabilities.json`). The behavioural rules have no cross-implementation tests; SDK 1.6.12 fixed a drift in exactly one of them (panel toggle on a collapsed column).

### 1.5 Three files nobody can read whole

`src/plugin/types.ts` 11,427 lines (88 % doc comments, 114 interfaces), `src/plugin/testing.ts` 7,645 lines (one class, about 54 public methods, 91 public fields, 117 wire handlers from `:5374`), `src/plugin/ui.ts` 1,433 lines (writer, binder, diff engine, windows, panels, layout service, dialogs in one module). Together with `runtime.ts` they are where every release lands (§1.1).

### 1.6 No documented events or messages

`docs/explanation/host-and-node.md` describes framing and the three message shapes (`:134-192`) and states the intent: "Plugin code never sees a method name" (`:231`). No page lists the ten subscribable events (`HostEvent`, `src/wire.ts:429-439`) or the roughly 90–97 wire methods (`src/host/capabilities.json`: 27 capabilities, 87 methods in this working tree; INVENTORY counted 97 on `origin/main`). Once a plugin may subscribe to events by name (style B) or send messages (style C), both become plugin-author documentation and must be generated, not written (§5.1).

### 1.7 Rulings (2026-11-08, binding)

Recorded in LAYERS §9 and kept here verbatim in substance:

1. Three-layer model adopted: **L0 protocol / L1 primitives / L2 framework**. L2 works only through L1's public surface; primitives never import the pass scheduler.
2. Three plugin styles are supported and documented: **A** declarative `bind()` (recommended default); **B** subscribe and write directly (`player.on(event)` + `view.set`/`write`, event names as literal types with IDE completion); **C** direct messages through `plugin.wire` (labelled "changes with versions"). Exposing the lower entry points is approved.
3. New `plugin.state({...})`: plugin-owned state whose reads register dependencies and whose writes mark them stale, to remove manual `plugin.update()`.
4. SDK documentation is refactored with it: event table and message table generated from code; a "three ways to write a plugin" chapter with 3–4 representative scenarios written side by side; each example keeps one main version, only examples whose styles genuinely differ get three.
5. This plan lives at `doc/specs/sdk-refactor.md`. Timing is decided separately. Existing plugin code runs unchanged.

Rulings of the same day, afternoon and evening (they close the open questions of the first draft):

6. The direct-message entry is named **`plugin.wire`** (not `plugin.experimental`); its source file is `src/plugin/wire-facade.ts`, distinct from the L0 directory `src/wire/`.
7. `plugin.state` tracks **one level** only: a nested object is replaced, not mutated.
8. `player.on` covers **all ten** `HostEvent` names, mirrored or not.
9. INVENTORY batches H and I are not batches: H is a measurement after Step 2 (the pass counter and a benchmark decide whether to simplify); I is a usage recount after the generated tables, ending in a short delete list for the user. The pass counter and the benchmark are added in Step 1 (§4.0).
10. Versioning: Step 1 ships as **1.6.x patch releases** ("Who must change: nobody"); Steps 2 and 3 together with the documentation refactor ship as **1.7.0**, the one approved minor bump, for the new API (`plugin.state`, `player.on`, `plugin.wire`).
11. The built-in sweep (manual `update()` → `plugin.state`) starts right after the player upgrades to SDK 1.7.0 with zero plugin edits, then plugin by plugin: `playback` and `info` as pilots, then the rest.

Standing rulings that bound the design: logic stays in TypeScript and nothing moves into the Rust host (the host keeps only shared facilities, `doc/architecture/README.md` §1); the boundary is fixed first and big moves come only from a researcher inventory (memory: SDK refactor direction).

## 2. Target architecture

### 2.1 Layers

| Layer | Contents | Public to plugins as | Stability label |
| --- | --- | --- | --- |
| **L0 protocol** | `Transport`, `createWire`, typed per-domain clients, wire types (`PropertyWrite`, `RowOp`, `HostEvent`, request/reply shapes), `API_VERSION` | `plugin.wire.request(method, params)`, `plugin.wire.on(event)`, exported **types** `Wire`, `PropertyWrite`, `RowOp`, `HostEvent` | "changes with versions"; each message carries a protocol label (§4, Step 3 preconditions) |
| **L1 primitives** | connection (subscribe/hold/raw, activation), view writer (`set`, `write`, limits, `Latest` single-flight, chunked `more`), command registry (register/update/unregister), player events (`player.on`), services (storage, secrets, dialogs, osd, clipboard, system, host, power, bindings, packages, i18n, `openStream`, player actions) | `plugin.*` members that exist today plus `player.on`, `view.write` (under `plugin.wire` until Step 3 ends) | semver as today for existing members; new members `@since` and documented |
| **L2 framework** | pass scheduler (`bind/part/observe/needsFull/invalidate/changed`, safety pass), row diff engine (`patch.ts`), state mirrors (`PlayerState`, window, timeline, appearance, subtitles…), `CommandHandle` state evaluation, `plugin.when`, `plugin.state` | `view.bind`, `commands.add` with function fields, `plugin.player.state`, `plugin.update`, `plugin.state` | semver as today |

Rule (ruling 1): **L2 uses only L1's public surface; primitives never import the pass scheduler.** Enforced by a lint in `npm run check` (an ESLint `no-restricted-imports` rule or a 40-line script over `tsc`'s import graph): no file under `src/plugin/services/`, `src/plugin/view/writer.ts`, `src/plugin/commands/registry.ts`, `src/plugin/player/events.ts`, `src/connection.ts`, `src/wire/**` may import `src/plugin/passes.ts`, `src/plugin/state.ts`, `src/plugin/view/binder.ts`, `src/plugin/commands/tracked.ts` or `src/plugin/player/mirror.ts`; and nothing under `src/plugin/**` may import `src/wire/**` except through `connection.ts`. The lint lands in Step 1 together with the file moves, so the structure cannot regress before Step 2 relies on it.

### 2.2 Directory layout

```text
src/
  wire/                      L0: transport.ts (Transport, Json, HostError), events.ts (HostEvent, subscribe counting),
                             clients/*.ts (the per-domain typed clients, moved from wire.ts, source.ts, …), index.ts (createWire)
  connection.ts              L1: owns the Wire; on/hold/raw with why/mirror marks; starting/declaring/activated; features
  latest.ts                  L1 helper: the Latest<T> single-flight queue (from runtime.ts:182)
  errors.ts                  L1 helper: failure/quiet/uncertain/toError/sleep (from runtime.ts:68-100 (report))
  plugin/
    index.ts                 createPlugin (unchanged signature; mirrors no longer force-initialised, see Step 2)
    scope.ts                 ScopeImpl: the Plugin object; wires services, view, commands, player, state, passes, plugin.wire
    types.ts                 re-exports types/*.ts (export type *); the public type surface is unchanged
    types/*.ts               by domain: scope, ui, commands, player, media, processing, services, …  (INVENTORY batch G)
    services/*.ts            L1: storage, secrets, dialogs, osd, clipboard, system, host, power, bindings, packages, language, local, drops
    view/
      writer.ts              L1: set/write/flush/#encode/#send/#parts, limits (values.ts), Pending/Settled/Sent, stale fallback
      binder.ts              L2: bind/snapshot/evaluate; joins the passes on the first bind()
      window.ts, panel.ts, settings.ts, layout.ts, markdown.ts, image.ts
    commands/
      registry.ts            L1: register/update/unregister with revision and Latest; declaration check call
      tracked.ts             L2: function fields, reader(), when-conditions, re-evaluation; binds only when needed
      keys.ts, when.ts
    player/
      events.ts              L1: player.on(event, listener) = connection.on(mirror=false) with a typed payload
      mirror.ts              L2: PlayerService state copy, accept/changed, optimistic submit/control
      stream.ts              L1: openStream (no import from mirror.ts; abortError moves to errors.ts)
      timeline.ts, window.ts, appearance.ts, subtitles.ts, filters.ts, shaders.ts, media.ts, resolvers.ts (mirror pattern, L2)
    state.ts                 L2: plugin.state({...}) tracked store
    passes.ts                L2: bind/depends/part/observe/needsFull/invalidate/changed/#schedule/#run, SAFETY_PASS_MS
    wire-facade.ts           L0/L1 facade behind `plugin.wire`: request, on, view(view).write/writeRows, commands.register
    testing/
      index.ts               TestHost facade class: same public members, same `@vestra-player/sdk/testing` entry
      transport.ts           request/on/calls/sent/settle/handle (the L0 fake)
      layout.ts, commands.ts, core-actions.ts, osd.ts, jobs.ts, processing.ts, i18n.ts, declarations.ts
  commands.ts, processing.ts, capabilities.ts, host-capabilities.ts, i18n*.ts, manifest.ts, palettes.ts  (rule tables; unchanged, later generated: batch E)
  player-runtime.ts, docs.ts, ts-plugin.ts, capability-check.ts  (unchanged entry points)
```

Two things are called "wire" and must not be confused: `src/wire/` is the L0 directory (transport, clients, types; private to the SDK and the runner), `plugin.wire` is the plugin-facing facade implemented in `src/plugin/wire-facade.ts`.

Module paths that matter to consumers do not change: `package.json` `exports` keeps `.`, `./testing`, `./runtime`, `./docs`, `./ts-plugin` (`package.json:13-36`); `src/index.ts` keeps `export type *`; `dist/index.d.ts` and `dist/plugin/testing.d.ts` must snapshot-equal before and after Step 1 (§4.0).

### 2.3 Every entry point hangs off `plugin`

A plugin **cannot import the SDK at run time**: `vestra-plugin build` fails on any value import of `@vestra-player/sdk` or a subpath (`bin/vestra-plugin.mjs:656-660`: "runtime import of …; use `import type`"), and `src/index.ts:1-8` documents it. Therefore there are **no subpath exports for L1 or L0**. Everything a plugin can call is a member of the `plugin` object it receives in `activate`, including the wire facade:

```ts
import type { Plugin, PropertyWrite, RowOp, HostEvent } from "@vestra-player/sdk"; // types only, as today

export default async function activate(plugin: Plugin) {
  plugin.player.on(...);      // L1, style B
  plugin.wire.request(...);   // L0, style C
}
```

The exported types grow (`Wire`, `PropertyWrite`, `RowOp`, `HostEvent`, `EventPayload<E>`); the exported values stay zero.

### 2.4 What does not change

`Plugin`/`Scope` and every existing member keep their signatures and behaviour; `TestHost` keeps its public members and stays an L0 fake (`class TestHost implements Transport`, `src/plugin/testing.ts:1040`; `request` at `:3755`, `on` at `:3785`, `calls` at `:1045`, `#defaults` at `:5374`); `createPlugin`'s signature (`src/plugin/index.ts:40-49`) and the player's runner are untouched; `apiVersion` stays `vestra/1`.

### 2.5 As built in Step 1 (1.6.18–1.6.20)

Where Step 1 landed differently from the layout above (the layout above stays the Step 2 target):

- `connection.ts`, `passes.ts`, `errors.ts` and `latest.ts` live in `src/plugin/`, not `src/`: `callSite` locates the SDK with `new URL("..", import.meta.url)`, and every user is under `src/plugin/`. `Runtime` is a thin class (`extends Connection`, a `passes` field, one-line delegations).
- The view code is `src/plugin/view/{writer,binder,base,window,panel,layout,image}.ts`; `base.ts` (`ViewBase` composing writer and binder) is new and belongs to neither layer until Step 2 changes its constructor. Dialogs are `src/plugin/services/dialogs.ts`. `view/writer.ts` (L1) uses the row diff engine of `patch.ts`; Step 2 decides whether patch planning stays in the writer.
- The types are `src/plugin/types/{base,commands,ui-content,ui-layout,ui-locale,ui-shell,playback,processing,subtitles,snapshots,sources,packages,services}.ts`, one documentation category per file (UI in four); the names follow TypeDoc's source order so the reference keeps its order.
- The import lint is `scripts/check-layers.mjs` in `npm run check`: L0/L1 files may not import or load any framework file (passes, runtime, binder), no value-import cycles (one known function-only cycle in `testing/`), every relative import resolves, types files are type-only. Three known L1→L2 edges (services → player mirror, stream → player mirror, packages → `scope.update()`) are listed as pending until `player.ts` is split. The rule "only `connection.ts` reaches `src/wire/`" waits for `src/wire/` to exist.
- The `TestHost` facade (`testing/index.ts`) is limited on non-comment code lines (≤ 1,500), not total lines: its doc comments must stay on the declarations.

## 3. The three plugin styles

Scenario used throughout: **the Playback panel shows the playback speed; pressing C cycles it.** The panel is `plugin.panels.mount("bottom", "ui/bar.slint")` as in `src/plugins/playback/main.ts:28`, with a Slint `in property <string> rate`.

### 3.1 Style A: declarative `bind()` (default, recommended)

```ts
const bar = plugin.panels.mount("bottom", "ui/bar.slint");
bar.bind(() => ({ rate: `${plugin.player.state.rate.toFixed(2)}×` }));
plugin.commands.add({
  id: "rate.cycle", path: ["Playback", "Cycle Speed"], bindings: ["C"],
  enabled: () => plugin.player.state.can.rate,
  run: () => plugin.player.setRate(nextRate(plugin.player.state.rate)),
});
```

The plugin states what the panel shows; the SDK decides when to send. Reads of `state.rate` are recorded (`src/plugin/player.ts:559-562` → `runtime.reading/depends`), so after Step 2 only this binder re-runs when `rate` changes, arrays are diffed into row patches, and values are coalesced one pass per microtask. Nothing in this code changes with the refactor.

### 3.2 Style B: subscribe and write

```ts
const bar = plugin.panels.mount("bottom", "ui/bar.slint");
plugin.player.on("source.state", (s) => bar.set({ rate: `${s.rate.toFixed(2)}×` }));
plugin.commands.add({
  id: "rate.cycle", path: ["Playback", "Cycle Speed"], bindings: ["C"],
  run: async () => { const s = await plugin.player.refresh(); await plugin.player.setRate(nextRate(s.rate)); },
});
```

`plugin.player.on(event, listener)` is a plain subscription: `event` is the literal union `HostEvent` (`"source.state" | "window.state" | "timeline.changed" | …`, `src/wire.ts:429-439`), the listener's parameter type is `EventPayload<typeof event>` (the wire payload, `SourceState` here), so the IDE completes both. No mirror is created for it (Step 2: the mirror subscribes on first read of `plugin.player.state`, not in the constructor), and the subscription is sent with `why` so the player's lazy-activation log names the listener (`src/wire.ts:790-800`; host `src/host/src/api/events.rs:66-71`). `bar.set({...})` after Step 2 goes to the L1 writer: value limits checked, one `Latest` write in flight per property, `more` chunking for large arrays, but **no diff against the last sent value and no pass registry**: the panel joins the passes only if something calls `bind()` on it. A command with no function fields (`enabled` omitted) is registered once and never re-evaluated.

### 3.3 Style C: direct messages through `plugin.wire` (unstable)

```ts
const bar = plugin.panels.mount("bottom", "ui/bar.slint");
const view = plugin.wire.view(bar);                               // raw writer for one view
plugin.wire.on("source.state", (data) => {                        // raw event, payload `unknown`
  const s = data as { rate: number };                             // the plugin types the payload itself
  void view.write("rate", { value: `${s.rate.toFixed(2)}×` });   // a PropertyWrite; limits are the host's only
});
// or, without any SDK help:
void plugin.wire.request("ui.setMount", { property: "rate", value: "1.00×" });
```

`plugin.wire.*` is the L0/L1 pass-through: `request(method, params)` is `Transport.request` with the capability check the host already performs (`src/host/src/host/dispatch.rs:150-175`) and nothing else; `view(bar).write(property, PropertyWrite)` and `.writeRows(property, RowOp[])` reach the writer below the diff engine; `on(event)` is `connection.raw`. Every member is documented with the stability label "changes with versions" and `@experimental`, its types are exported (`Wire`, `PropertyWrite`, `RowOp`), and the preconditions in §4 Step 3 apply (protocol label, `why`/`mirror` marking, one track per property).

### 3.4 Call chain per style

Pressing **C**, up to the panel showing the new speed. "Skipped" marks a step a style does not take. Wire lines are JSON lines over stdio (`host-and-node.md:134-192`). The host handler for a **panel** write is `ui.setMount` → `src/host/src/api/ui_mount.rs:61` → `set_mount` (`:182`); for a **tool window** the same chain ends in `ui.set` → `src/host/src/api/ui_window.rs:24` (method table) → `change` (`:116-137`, `prop_value` then `ui_values::reconcile_in`/`set`).

| # | Step | A `bind()` | B `player.on` + `set` | C `plugin.wire` |
| --- | --- | --- | --- | --- |
| 1 | host: key C matched to `rate.cycle` → `{"event":"commands.execute","data":{"id":"rate.cycle","revision":n}}` (`src/host/src/host/actions.rs:18`) | ✓ | ✓ | ✓ |
| 2 | SDK `connection` raw listener for `commands.execute` (`src/plugin/runtime.ts:356`) → `CommandHandle.#run` → `scope.run(options.run)` (`src/plugin/commands.ts:238-241`) | ✓ | ✓ | ✓ (or the plugin's own `wire.on("commands.execute")`) |
| 3 | plugin `run` → `plugin.player.setRate(next)` → `PlayerService.control` → `host.source.submit` (`src/plugin/player.ts:621-625, 279-289`) → wire `{"method":"source.submit","params":{"intent":{"kind":"control","control":{"kind":"rate","rate":1.5}},…}}` (`src/source.ts:129`) | ✓ (with `expected` stamp, fails `stale` on a changed source) | ✓ | ✓ or raw `request("source.submit", …)` |
| 4 | host applies the rate, emits `{"event":"source.state","data":{…,"rate":1.5,…}}` to subscribers | ✓ (mirror subscription) | ✓ (listener subscription, `why`) | ✓ (raw subscription; must mark `why` itself) |
| 5 | SDK `PlayerService.accept` → new `PlayerState` view → `runtime.changed("player", ["rate"])` (`src/plugin/player.ts:246-258`) | ✓ | skipped (no mirror unless `state` was read) | skipped |
| 6 | SDK pass: `invalidate` → microtask → `#run`; `ViewBase.evaluate` → `snapshot` → `part(token, binder)` re-runs only binders that read `rate` (`src/plugin/ui.ts:566, 546-563`; `src/plugin/runtime.ts:476-536`); commands with function fields re-evaluate (`src/plugin/commands.ts:251-283`) | ✓ | skipped | skipped |
| 7 | plugin code runs | binder returns `{rate: "1.50×"}` | listener calls `bar.set({rate: "1.50×"})` | listener calls `view.write("rate", {value: "1.50×"})` or `request("ui.setMount", …)` |
| 8 | SDK writer: `encode` (limits, `src/plugin/values.ts:42`), `#send`: row diff → patch or whole value, `stale` fallback, `more` chunks (`src/plugin/ui.ts:586, 651-742`) | ✓ full | `encode` + `Latest` + chunks; **no diff, no `#bases`** | `write`: `Latest` + chunks only; `request`: nothing |
| 9 | wire `{"method":"ui.setMount","params":{"property":"rate","value":"1.50×","slot":"bottom"}}` (`src/wire.ts:869`; `ui.set` for a window, `:908`) | ✓ | ✓ | ✓ |
| 10 | host `dispatch.rs`: capability declared? text values resolved (`:150-175`) → `ui_mount.rs:61 set_mount` (window: `ui_window.rs:116 change`) → Slint property set | ✓ | ✓ | ✓ |

Passes per key press in this scenario: A one pass; B none; C none. Wire lines: identical in all three (one `source.submit`, one `source.state` event, one `ui.setMount`); only the subscription line differs (`{"event":"source.state","mirror":true}` for A versus `{"event":"source.state","why":[…]}` for B/C).

### 3.5 `plugin.state({...})`: the replacement for manual `plugin.update()`

```ts
const st = plugin.state({ video: null as boolean | null, busy: false });
st.video = true;        // marks "video" stale → one coalesced pass, binders/commands that read st.video re-run
bar.bind(() => ({ keyframes: st.video === true }));   // the read registers a dependency on st.video
```

Semantics: `plugin.state(initial)` returns a tracked object of the same shape (a `Proxy` over a plain object, scoped to the calling `Scope`). Reading a property inside a binder, a command's function field or an `onChange` listener records `(storeId, key)` through the same `depends` mechanism the mirrors use (`src/plugin/runtime.ts:463`), so precise passes (`part`/`needsFull`) treat it like `plugin.player.state`. Writing a property calls `changed(storeId, [key])`; equal values (`Object.is`) are no-ops; tracking is **one level** (ruling 7): a nested object is not tracked, replace it to change it. Reading outside a tracked context is a plain read. `plugin.update()` stays, as the escape hatch for state the SDK cannot see.

Before (`src/plugins/playback/keyframes.ts:15-46`, abridged):

```ts
export class Keyframes {
  #video: { sourceId: number; video: boolean } | null = null;
  readonly #update: () => void;
  constructor(scope: Scope, playback: Playback, update: () => void) { …; this.#update = update; }
  #answer(sourceId: number, video: boolean) {
    …
    this.#video = { sourceId, video };
    this.#update();                      // full pass: every binder and every command of the plugin
  }
  state(): State { const s = this.#scope.player.state; …this.#video…; return { enabled, checked: null, busy: this.busy, reason }; }
}
// main.ts:62  new Keyframes(plugin, playback, () => plugin.update())
```

After:

```ts
export class Keyframes {
  readonly #st;
  constructor(scope: Scope, playback: Playback) {
    this.#st = scope.state({ video: null as { sourceId: number; video: boolean } | null, busy: false });
  }
  #answer(sourceId: number, video: boolean) {
    …
    this.#st.video = { sourceId, video };   // only readers of st.video re-run: the two keyframe commands
  }
  state(): State { const s = this.#scope.player.state; …this.#st.video…; return { enabled, checked: null, busy: this.#st.busy, reason }; }
}
// main.ts  new Keyframes(plugin, playback)
```

The injected `update` callback disappears (`main.ts:33` for `Playback` and `:62` for `Keyframes` are the same pattern), `busy` becomes tracked too, and `src/plugins/info/dialog.ts:199` becomes `timer = setInterval(() => void readStats().then((s) => (st.stats = s)), 1000)`.

### 3.6 Choosing

A for anything whose value is a function of state the SDK can see (player, window, timeline, `plugin.state`): least code, least traffic, precise passes. B when the source of truth is outside the SDK (a timer, a socket, a frame counter, an emulator) and the plugin wants to push exactly when it knows something changed, or when it wants raw event payloads; VestraNES (`C:\vestra\plugins-main\plugins\vestranes\main.ts:26` binds four fields from a `status` variable and otherwise only calls L1 `openStream`, `dialogs`, `commands.add`) is the L1-style plugin that gains most from B. C only to try a host method the SDK does not wrap yet; the documentation says so on every member.

## 4. Batches and acceptance criteria

Versioning (ruling 10): the Step 1 batches ship as **1.6.x patch releases**, each with the CHANGELOG line **"Who must change: nobody."**; Steps 2 and 3 ship together with the documentation refactor (§5) as **1.7.0**, the one approved minor bump, because they add API (`plugin.state`, `player.on`, `plugin.wire`); the later batches D, E and F are patches again. Every release passes the characterisation tests of §4.0 unchanged unless the batch says which test changes and why. Rough sizes are the inventory's estimates, for ordering only; **timing is not decided and this document does not schedule the batches.**

### 4.0 Characterisation tests, added first

Before any file moves (a release of its own, or the first commit of Step 1):

| Test | What it pins | Mechanism |
| --- | --- | --- |
| Golden wire traces | the exact `(method, params)` sequence and the events subscribed, per scenario | `TestHost.calls` (`src/plugin/testing.ts:1045`) recorded for the 10 examples and 6 built-ins (playback, playlist, info, subtitles, video, workshop) under fixed scripts (start, open a source, three state changes, one command, dispose); compared to `test/fixtures/traces/*.json` |
| API surface snapshot | every exported name and member of `dist/index.d.ts` and `dist/plugin/testing.d.ts` | a script that lists declarations by name and signature (the generated `docs/reference/testing.md` already derives from the declaration); diff must be empty in Step 1 and additive afterwards |
| Pass counter | number of `Runtime.#run` passes and of binder evaluations per scenario | a hook on `passes.ts` behind `RuntimeOptions`; asserted per golden scenario |
| Manual update baseline | `plugin.update()`/`scope.update()` count per built-in (table §1.3) | a script in the player repository; Step 2's sweep reduces it, Step 1 must not change it |
| Benchmark | wall time of the large-list scenarios (20,000 rows: set one row, move 50 rows, 16-chunk `more` write) and of one pass over the playback plugin's bindings | `node --test` benchmark under `test/bench/`, reporting p50/p95 to a JSON file that is compared, not asserted, in CI; it is the measurement ruling 9 wants before any simplification of the diff engine or of field-level dependencies |

### Step 1: split files, zero behaviour change (1.6.x patches)

Scope: INVENTORY batches **A** (`testing.ts` → `testing/*`, `TestHost` stays a facade class), **B** (`runtime.ts` → `connection.ts`, `passes.ts`, `latest.ts`, `errors.ts`; `Runtime` becomes a thin aggregate or disappears behind `ScopeImpl`), **C** (`ui.ts` → `view/{writer,binder,window,panel,layout,image}.ts`, dialogs out), **G** (`types.ts` → `types/*.ts` with `export type *`); the import lint (§2.1); the value-import cycle `src/plugin/player.ts:17` ↔ `src/plugin/stream.ts:7` removed (`abortError` → `errors.ts`); the lateral import `src/plugin/commands.ts:5` → `ui.ts` (`ImageHandle`) redirected to `view/image.ts`.

Compatibility: pure moves; no runtime behaviour, no public type, no wire line changes.

Acceptance: golden wire traces identical; API snapshot identical; pass counter identical; `docs-tree.test.ts` (`test/docs-tree.test.ts:419` "every page and declaration of the documentation site is in the tree"), `doc-lint` and `npm run docs:check` green with `STRICT_CATEGORIES` and `REDIRECTS` unchanged (typedoc works by symbol, not file); lint reports zero forbidden imports; no file under `src/plugin/` above 1,500 lines, `testing/*.ts` each below 1,500, `types/*.ts` each below 2,000. Size: A 2–3 days, B 2–3 days, C 3 days, G 2 days (inventory).

### Step 2: framework optional (part of 1.7.0)

Scope, each a separately revertible commit; released together with Step 3 and the documentation refactor as 1.7.0:

1. **View joins the passes on first `bind()`**: `ViewBase` constructor no longer calls `runtime.bind` (`src/plugin/ui.ts:456`); `binder.ts` registers on the first `bind()`. `set()` on a never-bound view goes straight to the writer (limits, `Latest`, chunks) and keeps no `#bases`; on a bound view `set()` behaves as today (forced send through the diff engine, `src/plugin/ui.ts:526`) so mixed use stays correct.
2. **Commands bind only when needed**: `CommandHandle` calls `runtime.bind` only if `enabled`, `checked`, `busy` or `reason` is a function (`src/plugin/commands.ts:242`); `set()` pushes this command's state through its own `Latest` (`:293-296`) and invalidates only bindings that read this command (none, normally) instead of `scope.update()`.
3. **Mirror on first read**: `PlayerService` (and window, timeline) subscribe `source.state` as a mirror on the first read of `.state` or the first `onChange`, not in the constructor (`src/plugin/player.ts:229-234`); `createPlugin` still awaits the initial `source.state` fetch when the manifest declares `playback.read` so the first read is synchronous (`src/plugin/index.ts:56-61`), but sends no `events.subscribe` until needed. Plugins that never read the state stop costing a mirror subscription.
4. **`plugin.player.on(event, listener)`** for all ten `HostEvent` names (ruling 8), mirrored or not: `HostEvent` literal types, `EventPayload<E>`, `why` from `callSite()`; `Disposable`, scoped. The same shape on `plugin.window`/`plugin.timeline` is sugar for the same call.
5. **`plugin.state(initial)`** (§3.5), in `state.ts`, one level.
6. **Built-in sweep** (player repository, separate PRs per plugin, ruling 11): first the player upgrades to SDK 1.7.0 with **zero** plugin edits (a release of its own, proving compatibility); then manual `update()` is replaced with `plugin.state` plugin by plugin, `playback` and `info` as pilots, then `subtitle-online`, `workshop`, `subtitles`, `window` and the rest in descending count (§1.3).

Compatibility: items 1–3 change nothing a plugin can observe except traffic (fewer `events.subscribe` lines, fewer `commands.update` lines, fewer passes); where a golden trace changes, the change is "a line disappears" and the test is updated with the CHANGELOG line that explains it. Items 4–5 are additive API (`@since 1.7.0`). Item 6 touches only built-ins and needs no SDK change after 1.7.0.

Tests added first: `events-capability`, `mirror-subscriptions`, `precise-invalidation` already exist (580/130/82 lines (report)); add "a view that is only `set()` sends whole values and no patch", "a command with constant fields sends one `commands.register` and no `commands.update` across 20 passes", "a plugin that never reads `player.state` subscribes to nothing", "`player.on` payload equals the wire event data", "`plugin.state` write re-runs exactly the readers".

Acceptance: pass counter per scenario ≤ today's, and 0 for the style-B scenario; manual `update()` count per built-in from the §1.3 table to **0** for the swept plugins and the baseline for the rest; `host-agreement.test.ts` green; the lazy-activation log (`why`) names `player.on` listeners by call site; mixed `set()`+`bind()` on one view stays patch-correct (row-patches tests). Size: SDK 1–2 weeks (LAYERS §8c), sweep 1 day per large built-in.

### Step 3: expose L1/L0 through `plugin.wire` (part of 1.7.0)

Scope: `plugin.wire` (`request`, `on`, `view(v).write/writeRows`, `commands.register(declaration)`) in `src/plugin/wire-facade.ts`; exported types `Wire`, `PropertyWrite`, `RowOp`, `HostEvent`, `EventPayload`; `TestHost` accepts raw requests exactly as the player does (it already does, being an L0 fake; `strictDeclarations` must apply to `commands.register` through `plugin.wire` too, `src/plugin/testing.ts:7490`).

Preconditions, each a small change released before or with Step 3:

- **Protocol versioning label**: the message table (§5.1) and every `plugin.wire` doc comment carry the player version the method appeared in (from `docs/reference/manifest.md`'s player-version table) and the sentence "the wire protocol is not semver; this member changes with versions" (`src/player-runtime.ts:1-8` already says this for the runner).
- **Capability check for raw calls**: `vestra-plugin check` and the editor plugin map member names to capabilities (`src/capability-rules.ts`, `src/api-requests.ts`); a raw `request("ui.setMount", …)` with a string literal method is mapped through `HOST_CAPABILITIES` (`src/host-capabilities.ts`, generated) so the check still fires; a non-literal method is reported as "unchecked".
- **Lazy-activation marking**: `plugin.wire.on` always sends `why` (call site) and never `mirror: true`; a raw `request("events.subscribe", …)` without `why`/`mirror` is refused by the SDK with code `invalid` so the player's residency classification (`events.rs:50-56`, `host/supervisor/mod.rs:929 classify` (report)) is not fooled.
- **One track per property**: a property written through `plugin.wire.view(v).write` is marked; a later `bind()`/`set()` that produces the same property throws `invalid` ("property `rows` is written directly; bind it or write it, not both"), and vice versa, so the diff engine's `#sent`/`#bases` cannot go stale (`src/plugin/ui.ts:689-693`).

Compatibility: additive; nothing existing changes. Ships inside 1.7.0 with Step 2.

Acceptance: golden traces unchanged for all existing scenarios; a new style-C scenario trace equals the style-B trace except the subscription line; `capability-check.test.ts` covers a literal raw method; doc-lint `@capability` rule accepts "That of the method" wording for `plugin.wire.request`. Size: 1 day for the pass-through (LAYERS §8c), 3–4 days with the preconditions.

### Then: INVENTORY batches D, E, F (single source of rules), unchanged (1.7.x patches)

| Batch | Scope | Compatibility | Tests first | Acceptance | Size |
| --- | --- | --- | --- | --- | --- |
| **D** rules module | the checks TestHost performs as "the player's" (`checkDeclaration`, `layout.set`, osd, job refusals, `patchOps`) move to `src/rules/` shared by runtime and TestHost, as `when.ts`/`processing.ts` already are | error codes and messages are the contract, unchanged | table-driven fixtures recording today's TestHost (request → code/message) | fixtures pass against both; no duplicate implementation left in `testing/*` | 1 week |
| **E** host-generated rule file | `capabilities-sync.mjs` grows to `rules.json`: every `*_LIMITS`, `MENU_IDS`, `BUTTON_ICONS`, `WAKE_EVENTS`, `ACTIONS`, `PHASES`, error and reason codes, `BUILTIN_COMMANDS` from the player's `menus-snapshot`; SDK constants become generated; `host-agreement.test.ts` compares files instead of reading Rust with regexes | export names and values unchanged | `test/constants.test.ts` plus one release with a generated-vs-handwritten diff test | the 42 regex tests replaced by one file comparison; a host table change is one SDK release with no hand edit | 1 week, SDK first |
| **F** behaviour fixtures | shared JSON fixtures run by `cargo test` and `node --test`: condition evaluation, key normalisation, patch application, `layout.set` transitions, job states; start with the toggle rule fixed in 1.6.12 | no API change | the fixtures are the tests | every behavioural rule in INVENTORY §2.2 has a fixture both sides run | 1 week |

Two items of the inventory are **not** batches (ruling 9):

- **Measurement after Step 2** (inventory H): the pass counter and the benchmark of §4.0, run on 1.7.0, say whether the row-move planning in `patch.ts` and the field-level dependencies in `passes.ts` still earn their complexity now that passes are rarer. A simplification is proposed to the user only with those numbers; none is planned here.
- **Usage recount after the generated tables** (inventory I): once the event and message tables exist (§5.1) and `plugin.wire` has shipped, the zero-use members of INVENTORY §1.2 are recounted over built-ins, third-party plugins and examples, and a short delete list goes to the user; nothing is removed without that ruling.

## 5. SDK documentation refactor

> **As built (1.7.2, 2026-11-08, user ruling).** After 1.7.0 the user found the seven-group tree too large and wordy. SDK 1.7.2 reduced it to four groups — Tutorial (one page), Guides (how-to + task tutorials + a design-rules page), Reference (generated tables kept), Examples — deleted `explanation/` and `upgrading/`, removed every player/SDK version-history sentence from docs and published TSDoc (`@since` tags stay), and added site redirects plus doc-tree aliases for every old title and file path. The generated-block mechanism and the two generated tables of §5.1 are unchanged. The group list in the next paragraph describes 1.7.0.

Ruling 4 applies. The documentation tree keeps its Diátaxis groups (`scripts/doc-structure.mjs` `GROUPS`: tutorials, how-to, reference, explanation, guidelines, examples, upgrading) and its generated-block mechanism (`scripts/doc-tables.mjs`: `<!-- generated:<name> -->` blocks, one generator per `scripts/doc-tables/<name>.mjs`, `npm run docs:check` fails on drift).

### 5.1 Generated reference pages

Two new generators, never hand-written; both ship in 1.7.0 with Steps 2 and 3, since that is when the names become plugin-facing.

**Event table** (`docs/reference/events.md`, generator `scripts/doc-tables/events.mjs`):

| Column | Source |
| --- | --- |
| event name | the `HostEvent` union in `src/wire/events.ts` (today `src/wire.ts:429-439`), read with `scripts/doc-tables/lib/source.mjs` |
| payload type | the `EventPayload<E>` mapping (new in Step 2), rendered as a link into the API reference |
| what it feeds | the `plugin.player.*`/`plugin.window.*`/… mirror or `onChange` that subscribes to it, from a `@event` tag on the mirror's doc comment (doc-lint enforces that every `HostEvent` has exactly one or is marked "no mirror") |
| wake event | membership in `WAKE_EVENTS` (`src/plugin/runtime.ts:37`, host `events.rs`) |
| capability | from `HOST_CAPABILITIES` (`events` or the specific read capability) |
| player version | first version in the manifest reference's player-version table |

**Message table** (`docs/reference/messages.md`, generator `scripts/doc-tables/messages.mjs`):

| Column | Source |
| --- | --- |
| method | `src/host/capabilities.json` via `scripts/capabilities-sync.mjs` (27 capabilities, 87–97 methods) |
| params / result | the typed client's `transport.request<Result>(method, params)` signature in `src/wire/clients/*.ts`, read with the TypeScript compiler; a method with no client is marked "no SDK wrapper" |
| error codes | the `PluginError` table (`src/plugin/types.ts`, already generated into `docs/reference/errors.md` by `error-codes.mjs`) joined with the codes each handler can return, from batch E's `rules.json` (until E lands: from `@throws` on the client) |
| host rule it enforces | the `needs` field of `capabilities.json` (capability, by-parameter cases) plus the residency the capability records (`live`/`recordable`/none) |
| stability | "SDK member" when wrapped by a public L1/L2 member (named), "`plugin.wire` only, changes with versions" otherwise |

Both pages join `docs/reference/index.md` and `GROUPS`; `docs-tree.test.ts` picks them up through the tree.

### 5.2 Chapter "Three ways to write a plugin"

A new explanation page `docs/explanation/three-ways.md` (and the matching how-to pages where a scenario needs steps), with 3–4 scenarios each written in all three styles side by side, each followed by the call-chain table of §3.4 reduced to the rows that differ and a "when to choose" paragraph:

1. **Speed display** (§3): state the SDK mirrors → A wins; B and C shown for contrast.
2. **Per-frame push** (VestraNES): the emulator produces a status line and a frame counter at 60 Hz; A would re-run a binder per frame, B pushes `set()` from the frame callback with `Latest` coalescing, C writes raw `PropertyWrite`. Uses `openStream` as the L1 service it already is.
3. **Long list with row patches** (playlist-like, 20,000 rows): A `bind()` with keyed rows gets the diff engine (`doc/architecture/large-lists.md` numbers (report; file not present in this working tree)); B `set()` sends whole arrays and chunks; C `writeRows` sends the plugin's own `RowOp[]`.
4. **Plugin-own state** (Keyframes, §3.5): `plugin.state` versus manual `update()` versus B's explicit `set()` after the async answer.

Each code block is a tested snippet (doc-lint `example-lead` and `stale-refs` rules apply; the snippets live under `examples/three-ways/` as one example plugin with three entry files, so `check-examples.mjs` compiles them).

### 5.3 Examples policy

The ten examples under `examples/` (`hello`, `blur`, `signal-generator`, `bookmark`, `panel-menu`, `adjust-panel`, `menus`, `markdown`, `i18n`, `theme-widgets`; `docs/examples/index.md`) each keep **one** main version, in style A unless the example is about something else (`hello`, `menus`, `i18n` have no view binding to speak of). Only examples whose three versions genuinely differ get three entry files and a short "in style B / C" section in their walkthrough: `signal-generator` (per-frame push, B is natural) and `panel-menu` (list rows, C's `writeRows` is instructive). `bookmark` stays A only (B would be the same code with `set` instead of `bind`). New examples default to A.

### 5.4 `host-and-node.md` and stability labels

`docs/explanation/host-and-node.md` keeps the framing and message-shape sections and changes "Plugin code never sees a method name" (`:231`) to: plugin code in styles A and B never sees a method name; style C does, and the names are in the message table. The page gains a short "Layers and stability" section: L2/L1 members are semver (`@since`, deprecation by CHANGELOG); `plugin.wire` and the two tables are "changes with versions", with the player version column as the only promise. `docs/explanation/architecture.md` and `plugins.md` get the layer names where they describe the update pass; `doc/architecture/sdk-design.md` in this repository gets a pointer (one paragraph) to this spec.

### 5.5 Tests to update

`test/docs-tree.test.ts` (new pages in the tree, `:66-124` group listings); `scripts/doc-structure.mjs` (`GROUPS` entries with `why`; size limits for the two generated pages are exempt as generated blocks); `scripts/doc-lint.mjs` (new rule: every `HostEvent` has an `@event` owner; members of `plugin.wire` carry `@experimental` and the stability sentence); `scripts/check-docs.mjs` (page list); `npm run docs:check` (new generators); `llms.txt` regenerated (`scripts/llms.mjs`).

### 5.6 What a plugin author needs to know, per style

| Need | A `bind()` | B `on` + `set` | C `plugin.wire` |
| --- | --- | --- | --- |
| Which state exists and how to read it | `PlayerState` & friends (API reference), `plugin.state` | event table: names, payload types | event table + message table |
| When the player is told | "the update pass" (`plugins.md`): coalesced, precise, safety pass | "on your call": `Latest`, chunks, limits (`limits.md`) | "on your call", no coalescing beyond `Latest` in `write`; nothing in `request` |
| Lazy activation | automatic (`mirror`/`why` marks) | automatic `why` | must send `why`; residency column of the message table |
| Capability declaration | `@capability` on each member | same | capability column of the message table; `vestra-plugin check` for literal methods only |
| Errors | `PluginError` codes, `onError` | same | error-code column; raw `HostError` |
| Stability | semver | semver | changes with versions; player version column |
| Testing | `TestHost` as today (`mounted()`, `window()`, `command()`) | same, plus `sent("ui.setMount")` | same (`calls`, `sent`, `handle`) |

## 6. Risks and compatibility rules

From LAYERS §8(d), with the rule each batch follows:

| Risk | Rule |
| --- | --- |
| The wire protocol is not semver (`src/player-runtime.ts:4-6`); exposing it makes 90+ methods a public contract | `plugin.wire` and the two tables carry "changes with versions" and a player-version column; the SDK promises only that a listed method matches that player version; no deprecation period for `plugin.wire` members |
| Capability checks are bypassed: `capability-rules.ts`/`api-requests.ts`/`ts-plugin.ts` map **member names** to capabilities; raw `host.*` calls are not in the table | Step 3 precondition: literal-method mapping through `HOST_CAPABILITIES`; non-literal methods reported "unchecked"; the host still refuses undeclared capabilities (`dispatch.rs:150-152`) |
| Lazy-activation misclassification: `events.subscribe` without `why`/`mirror` (`src/wire.ts:790-845`) makes the player mark the plugin resident or not wrongly (0.5.18/0.5.21 mechanism; `events.rs:66-71`) | `plugin.wire.on` always sends `why`; raw `events.subscribe` without marks is refused by the SDK |
| Two write tracks on one property desynchronise `#sent`/`#bases` → `stale` fallbacks (`src/plugin/ui.ts:689-693`) | one track per property, enforced with code `invalid` at the first conflicting call |
| `Latest` semantics (single in-flight, newest wins, `src/plugin/runtime.ts:182`) and the value/row/byte limits (`src/plugin/values.ts:42`, `CHUNK` in `patch.ts`) are either replicated in L1 or left to the plugin | the L1 writer keeps `Latest`, limits and chunking (they are primitives, not framework); only `plugin.wire.request` leaves them to the plugin, and its documentation says so |
| TestHost presets target SDK behaviour (`strictDeclarations`, `features: [PATCH_FEATURE]`, `src/plugin/testing.ts:7487-7490`) | they apply to raw registration and raw writes too; TestHost stays an L0 fake so every style is testable without changes to a plugin's tests |
| Moving files breaks documentation anchors and the `llms.txt` tree | typedoc, `docs.ts` and `doc-standard.mjs` work by symbol name; `REDIRECTS` checked in Step 1 acceptance |
| A split that changes behaviour by accident | golden wire traces, API snapshot and pass counter are added **before** Step 1 and must be unchanged by it |

Compatibility rules for every release: existing plugins run unchanged (zero source edits, same `minPlayerVersion`); Step 1 and the D/E/F batches are patch releases with "Who must change: nobody", and 1.7.0 (Steps 2 and 3 with the documentation) is the only minor bump, additive only, with the same line; `apiVersion` stays `vestra/1`; new members are additive with `@since`; TestHost stays an L0 fake; no logic moves into the host (the host gains at most the `rules.json` export of batch E, which is data it already owns).

## 7. Open questions for the user

None open at the time of writing: the five questions of the first draft (one-level `plugin.state`, `player.on` for all events, the name `plugin.wire`, the timing of the built-in sweep, the benchmark in Step 1) were ruled on 2026-11-08 and are recorded as rulings 6–11 in §1.7. Questions that come up during implementation are added here and taken to the user; the two deferred decisions (simplify the diff engine or the dependencies; delete zero-use members) return with the numbers described after the D/E/F table in §4.

## 8. Non-goals

- No change to the wire protocol, to `apiVersion`, or to the host's capability model; the host gains no plugin logic.
- No deletion or deprecation of existing members (INVENTORY §1.2's zero-use members stay until the recount and the user's ruling).
- No rewrite of the diff engine or of precise invalidation (a simplification waits for the measurement after Step 2), and no performance work beyond what fewer passes bring for free.
- No second minor bump: everything after 1.7.0 is a patch again.
- No new subpath exports: plugins keep importing types only.
- No breaking change to `TestHost`; no second test host.
- No schedule in this document.
