#!/usr/bin/env node
/**
 * Wire trace differential (core thread plan S1.1): compares two runs' wire
 * traces plugin by plugin and prints the first difference of each plugin. A
 * trace is what the host's test switch `VESTRA_TEST_WIRE_TRACE=<file>`
 * writes (src/host/src/process/wire_trace.rs): one JSON line per protocol line
 * between the host and a plugin, `{plugin, seq, dir, owner, ms, line}`.
 *
 *   node src/scripts/verify-wire.mjs diff <A> <B> [--allow-reorders]
 *        [--report <file>] [--root <path>=<token>]...
 *     <A>, <B>: two trace files, or two V2 run directories
 *     (verify-runtime.mjs run --wire-trace: raw/<scenario>/<step>.wire.jsonl,
 *     compared step by step; their run.json gives the roots), or two
 *     directories of trace files (paired by relative path).
 *     --allow-reorders  rule R4 and the S3a row's three classes: a
 *                       plugin's lines read may interleave its
 *                       UI-independent and UI-bound streams differently,
 *                       each in order; of its lines written, (1) core
 *                       lines keep their order, (2) UI-bound
 *                       replies and (3) lines an interface report caused
 *                       (`lineClass`) keep their order and their places
 *                       among each other and may move against the core
 *                       lines. Each reorder is listed, each moved line of
 *                       class 2 or 3 counted per class; anything else is
 *                       a difference.
 *     --root <path>=<token>  a path replaced by <token> (repeatable)
 *     --noise <file>    report only differences outside the noise set that
 *                       `noise` recorded (below), and how many were inside it
 *   node src/scripts/verify-wire.mjs noise <base A> <base B> --out <file>
 *     Two runs of the same build (V2 run directories, or trace files): their
 *     differences per plugin and direction are the noise set, written to
 *     <file> with its size printed. Recorded by position class (a request's
 *     method, `reply:<method>`, `event:<name>`): a class whose own sequence
 *     differs is noisy with every line either run had in it, and a direction
 *     whose order of the other classes differs has a noisy order. A
 *     candidate is then reported for a difference in a class that was not
 *     noisy, a line no base run had in a noisy class, a changed order where
 *     the order was not noisy, or a plugin missing that both bases had.
 *   node src/scripts/verify-wire.mjs check <trace file or directory>
 *     The rules one trace shows without a second run (`checkTrace`):
 *     request and reply pairing, R4's order in the UI-bound stream, R5's
 *     `ui.preparing` before the `ui.mount` reply, and R13 as the wire shows
 *     it (no `window.state` broadcast of a change before the reply that
 *     made it; the plugin's next state read sees the change).
 *
 * Which comparison proves what (coordinator ruling, 2026-11-15): controlled
 * plugins (src/host/tests/wire_trace.rs) must compare exactly equal: the
 * gate for S2 and, with --allow-reorders, S3a. V2's scenarios start 16
 * built-ins concurrently and differ run to run, so they are compared with
 * --noise against two base runs. `check` runs on any trace.
 *
 * The stream map (which methods are UI-bound, rule R4) is wire-streams.json
 * beside this script, generated once from the core thread design's method
 * table (plan S0.2) and kept by hand since.
 *
 * Normalised (each side alike) before the comparison:
 *
 *   owner numbers, `seq`, `ms`     not compared (`g`, `owner`, `pid` and
 *                                  `generation` fields inside lines too)
 *   request ids                    renumbered per plugin in order of first
 *                                  use: the plugin's requests `#1, #2, …`
 *                                  (its replies by the request's number),
 *                                  the host's requests to it `h#1, …`
 *   times                          only what is measured or read from the
 *                                  clock, each found varying run to run on
 *                                  the controlled plugins: `elapsedMs`
 *                                  (`ui.preparing`: how long a mount took
 *                                  to prepare), `prepareMs` (the same in
 *                                  the `ui.mount` reply), and any number
 *                                  from 10^12 up (a wall clock in ms, as a
 *                                  `time` trigger's bounds). Media
 *                                  positions, requested delays, patch
 *                                  positions (`at`) and every other number
 *                                  are compared; in V2's scenarios their
 *                                  run-to-run variation (positions while
 *                                  playing, `playback.stats`) is what the
 *                                  noise baseline holds
 *   paths, GUIDs, digests, versions  as V2 does (verify-runtime.mjs
 *                                  `normaliser`): the roots of run.json and
 *                                  --root, instance GUIDs, package digests
 *   heartbeat                      the shared runner's `runner.ping` and
 *                                  `runner.pong` are dropped (one every 2 s:
 *                                  their count is the run's length)
 *   repeated state broadcasts      an outgoing event whose name ends in
 *                                  `.state`, equal to the plugin's line
 *                                  written before it, is dropped (a state
 *                                  broadcast while playing comes about 10
 *                                  times a second; how many is timing).
 *                                  Every other line counts each time, a
 *                                  one-shot event (`triggers.fired`,
 *                                  `ui.preparing`) included
 *   failed writes                  a line whose write failed is compared
 *                                  with ` [write failed]` after it
 *   directions                     a plugin's lines read and its lines
 *                                  written are compared as two sequences:
 *                                  their interleaving is the pipes' timing
 *                                  (two threads), not the host's order
 *
 * Exit code: 0 no difference (allowed reorders listed), 1 differences,
 * 2 usage error. No dependencies.
 */
import {
  existsSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { normaliser } from "./verify-runtime.mjs";

class UsageError extends Error {}

const HERE = dirname(fileURLToPath(import.meta.url));
export const STREAMS_FILE = join(HERE, "wire-streams.json");

/** The `plugin` of the shared runner's own lines (wire_trace.rs `RUNNER`). */
export const RUNNER = "(shared runner)";
const HEARTBEAT = new Set(["runner.ping", "runner.pong"]);
/** Keys of measured durations (see the header, "times"). */
const TIME_KEY = /^(elapsedMs|prepareMs)$/;
const EPOCH_MS = 1_000_000_000_000;
const OWNER_KEY = /^(g|owner|pid|generation)$/;

// ---------------------------------------------------------------- streams

/** The stream of a request's method: `B` (UI-bound) or `I`; `known`. */
export function streamOf(streams, method) {
  if (streams.uiBound.includes(method)) return { stream: "B", known: true };
  const known = streams.uiIndependent.some((entry) =>
    entry.endsWith("*")
      ? method.startsWith(entry.slice(0, -1))
      : entry === method,
  );
  return { stream: "I", known };
}

// ------------------------------------------------------------- normalise

/** A trace file's records (the `start` lines left out). */
export function readTrace(text) {
  return text
    .split(/\r?\n/)
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line))
    .filter((record) => record.plugin !== undefined);
}

const isReply = (line) =>
  line &&
  typeof line === "object" &&
  "id" in line &&
  !("method" in line) &&
  ("result" in line || "error" in line);
const isRequest = (line) =>
  line && typeof line === "object" && "id" in line && "method" in line;

/**
 * One trace, normalised: `{ [plugin]: { in: [entry], out: [entry] } }`,
 * each entry `{ text, stream, kind, seq, ms }` (`text` is what is
 * compared; `stream` is `I` or `B` by rule R4; `kind`, its position
 * class for the noise baseline: the request method, `reply:<method>` or
 * `event:<name>`; `seq` and `ms` locate it).
 * `context`: `{ roots: [[path, token]], versions: [string], streams }`.
 */
export function normaliseTrace(records, context = {}) {
  const strings = normaliser({
    roots: context.roots ?? [],
    versions: context.versions ?? [],
    ids: [],
  });
  const streams = context.streams ?? { uiBound: [], uiIndependent: [] };
  const plugins = {};
  const unknown = new Set();
  // Per plugin: request ids by namespace, `${owner}:${id}` → [n, stream].
  const ids = {};
  const value = (input, key = "") => {
    if (typeof input === "number") {
      // A wall-clock time in ms (after 2001) under any key.
      if (TIME_KEY.test(key) || input >= EPOCH_MS) return "<time>";
      if (OWNER_KEY.test(key)) return "<owner>";
      return input;
    }
    if (typeof input === "string") return strings.text(input);
    if (Array.isArray(input)) return input.map((item) => value(item));
    if (input && typeof input === "object")
      return Object.fromEntries(
        Object.entries(input).map(([name, item]) => [name, value(item, name)]),
      );
    return input;
  };
  // Each kept entry by `plugin:seq`, for a failed write that marks it.
  const entries = new Map();
  for (const record of records) {
    const { plugin, dir, owner } = record;
    if ("failed" in record) {
      const entry = entries.get(`${plugin}:${record.failed}`);
      if (entry) entry.text += " [write failed]";
      else
        ((plugins[plugin] ??= { in: [], out: [] }).out ??= []).push({
          text: "[a write failed]",
          stream: "I",
          kind: "write failed",
          seq: record.failed,
          ms: null,
        });
      continue;
    }
    const line = record.line ?? {};
    if (
      plugin === RUNNER &&
      typeof line.event === "string" &&
      HEARTBEAT.has(line.event)
    )
      continue;
    const own = (ids[plugin] ??= { plugin: new Map(), host: new Map() });
    const fromPlugin = dir === "in";
    let stream = "I";
    let id = null;
    let kind = typeof line.event === "string" ? `event:${line.event}` : "other";
    if (isRequest(line)) {
      kind = `${fromPlugin ? "" : "host "}${line.method}`;
      const space = fromPlugin ? own.plugin : own.host;
      const key = `${owner}:${line.id}`;
      if (fromPlugin) {
        const found = streamOf(streams, line.method);
        stream = found.stream;
        if (!found.known) unknown.add(line.method);
      }
      if (!space.has(key)) space.set(key, [space.size + 1, stream, kind]);
      id = `${fromPlugin ? "#" : "h#"}${space.get(key)[0]}`;
    } else if (isReply(line)) {
      // A reply goes the other way from its request.
      const space = fromPlugin ? own.host : own.plugin;
      const found = space.get(`${owner}:${line.id}`);
      id = found ? `${fromPlugin ? "h#" : "#"}${found[0]}` : "#?";
      if (found) stream = found[1];
      kind = `reply:${found?.[2] ?? "?"}`;
    }
    const normalised = value(line);
    if (id !== null) normalised.id = id;
    // A shared counter's value (`REVISION_REPLIES`): not compared.
    if (
      isReply(line) &&
      REVISION_REPLIES.has(kind.slice("reply:".length)) &&
      normalised.result &&
      typeof normalised.result === "object" &&
      "revision" in normalised.result
    )
      normalised.result.revision = "<revision>";
    const text = JSON.stringify(normalised);
    const side = ((plugins[plugin] ??= { in: [], out: [] })[dir] ??= []);
    // Only a repeated state broadcast is collapsed: every other line,
    // a one-shot event included, counts each time it is sent.
    const broadcast =
      dir === "out" &&
      typeof line.event === "string" &&
      line.event.endsWith(".state");
    if (broadcast && side.length && side[side.length - 1].text === text)
      continue;
    const entry = { text, stream, kind, seq: record.seq, ms: record.ms };
    side.push(entry);
    entries.set(`${plugin}:${record.seq}`, entry);
  }
  return { plugins, unknown: [...unknown].sort() };
}

// --------------------------------------------------------------- compare

const where = (entry) =>
  entry ? `seq ${entry.seq}, ${entry.ms} ms: ${entry.text}` : "(nothing)";

/** The first index where two lists of entries differ, or -1. */
function firstDifference(a, b) {
  const length = Math.max(a.length, b.length);
  for (let i = 0; i < length; i++) if (a[i]?.text !== b[i]?.text) return i;
  return -1;
}

/**
 * The lines the core writes because of an interface report (core thread
 * plan S3a row, class 3; design §8): `window.state`, `ui.preparing`,
 * `ui.callback`, `ui.closed`, `ui.error`, `drops.*`, and `host.error` of an
 * interface failure (its codes below).
 */
export const REPORT_EVENTS = new Set([
  "window.state",
  "ui.preparing",
  "ui.callback",
  "ui.closed",
  "ui.error",
]);
export const INTERFACE_ERRORS = new Set([
  "menu-bridge",
  "menu-projection",
  "window-policy",
  "ui-owner-cleanup",
]);

/**
 * UI-independent methods answered from a worker thread of the core's, not
 * in the core's turn: their replies keep their order among the replies of
 * the same worker and may move against every other line (class 4 of
 * [`lineClass`]). Each entry: a worker, its methods by exact name, and
 * its method prefixes (ending in a dot).
 *
 * - `clipboard` (plan step S3a(3)): `clipboard.*` on its serial thread.
 * - `playback` (plan step S3b, the coordinator's K14 ruling): the methods
 *   the core hands to the playback worker, answered by the worker's own
 *   reply event (`api/mod.rs` `forward` and `forward_media`,
 *   `media_sources::forward_with_store`, and the direct sends of
 *   `mpv.rs`, `video_motion.rs`, `video_upscaling.rs`,
 *   `source_resolve.rs`). With the event-driven core a worker reply can
 *   come before or after the reply to a request the plugin sent at the
 *   same time; replies to concurrent requests were never ordered. Not
 *   here, answered in the core: `player.setStepSeek`,
 *   `power.preventSleep`, `sequence.set`, `sequence.step`,
 *   `timeline.marks`. A refusal the core gives before forwarding is
 *   counted here too (by method), which only loosens its place.
 */
export const WORKERS = [
  { name: "clipboard", exact: [], prefixes: ["clipboard."] },
  {
    name: "playback",
    exact: [
      // api/playback_read.rs
      "source.state",
      "processing.state",
      "playback.stats",
      "player.chapters",
      "player.audioDevices",
      // api/playback_control.rs
      "playback.set",
      "source.submit",
      "source.operation",
      "audio.addTrack",
      "audio.selectTrack",
      "player.setLoop",
      "player.setAudioDevice",
      "player.step",
      // api/processing_video.rs
      "processing.replace",
      "processing.operation",
      "video.shaders.set",
      // api/source_resolve.rs
      "sourceResolvers.register",
      "sourceResolvers.unregister",
      "sourceResolvers.respond",
    ],
    prefixes: [
      "video.view.", // processing_video.rs
      "video.render.", // processing_video.rs
      "snapshot.", // media_control.rs, media_read.rs
      "jobs.", // media_control.rs, media_read.rs
      "media.sources.", // media_sources.rs (not `locate`: `except`)
      "subtitles.", // subtitles_control.rs, subtitles_read.rs
      "producer.", // producer_av.rs
      "mpv.", // mpv.rs
      "video.motion.", // video_motion.rs
      "video.upscaling.", // video_upscaling.rs
    ],
    // UI-bound (design row 65c): its reply waits for the user's choice in
    // a file dialog, so it is a class 2 reply, not a worker reply.
    except: ["media.sources.locate"],
  },
];
/** The worker a method's reply comes from, or `null` (the core). */
export function workerOf(method) {
  return (
    WORKERS.find(
      (worker) =>
        !(worker.except ?? []).includes(method) &&
        (worker.exact.includes(method) ||
          worker.prefixes.some((prefix) => method.startsWith(prefix))),
    )?.name ?? null
  );
}
/** Kept for callers of the S3a(3) list: the clipboard prefixes. */
export const WORKER_REPLIES = WORKERS[0].prefixes;

/**
 * Replies whose `result.revision` is a shared counter of the host's, whose
 * value depends on how the host's own changes interleave with the request
 * (plan step S3b, the coordinator's ruling): the command registry's
 * (`commands.register`, `commands.update`, `commands.list`;
 * api/commands.rs, commands_run.rs) and the binding table's
 * (`commands.keys`, binding_manager.rs). Only that field is normalised;
 * a `revision` elsewhere (`processing.operation`'s own resource
 * revision) stays compared.
 */
export const REVISION_REPLIES = new Set([
  "commands.register",
  "commands.update",
  "commands.list",
  "commands.keys",
]);

/**
 * The class of a line written to a plugin (the S3a row's wire-trace rule):
 * 1 a core line, 2 a reply to a UI-bound request (R4), 3 a line caused by
 * an interface report, 4 a reply from a worker thread (`WORKER_REPLIES`).
 */
export function lineClass(entry) {
  if (entry.kind.startsWith("reply:")) {
    const method = entry.kind.slice("reply:".length);
    if (workerOf(method)) return 4;
    return entry.stream === "B" ? 2 : 1;
  }
  if (!entry.kind.startsWith("event:")) return 1;
  const name = entry.kind.slice("event:".length);
  if (REPORT_EVENTS.has(name) || name.startsWith("drops.")) return 3;
  if (name === "host.error") {
    let code;
    try {
      code = JSON.parse(entry.text).data?.code;
    } catch {
      code = undefined;
    }
    if (INTERFACE_ERRORS.has(code)) return 3;
  }
  return 1;
}

/**
 * Two normalised traces: `{ differences: [text], reorders: [text] }`, at
 * most one difference per plugin (its first), with `moves: { 2, 3 }` in
 * the allowed-reorders mode.
 *
 * With `allowReorders` (the S3a row's three classes): the lines read from
 * a plugin may interleave its UI-bound and UI-independent streams
 * differently, each stream in order (R4). The lines written to it are in
 * three classes (`lineClass`): (1) core lines keep their order; (2) the
 * replies of UI-bound requests and (3) the lines an interface report
 * caused keep their order among themselves and their place relative to
 * each other (a class-3 line keeps its place relative to the reply it
 * belongs to: R5, R13), and both may move against the core lines. Each
 * line of class 2 or 3 that has a different number of core lines before it
 * is a move, counted per class in `moves` and listed in `reorders`.
 */
export function compareTraces(a, b, { allowReorders = false } = {}) {
  const differences = [];
  const reorders = [];
  const moves = { 2: 0, 3: 0 };
  const plugins = [
    ...new Set([...Object.keys(a.plugins), ...Object.keys(b.plugins)]),
  ].sort();
  for (const plugin of plugins) {
    const left = a.plugins[plugin];
    const right = b.plugins[plugin];
    if (!left || !right) {
      differences.push(
        `${plugin}: only in ${left ? "A" : "B"} (${(left ?? right).in.length} lines read, ${(left ?? right).out.length} written)`,
      );
      continue;
    }
    for (const dir of ["in", "out"]) {
      const x = left[dir] ?? [];
      const y = right[dir] ?? [];
      const at = firstDifference(x, y);
      if (at < 0) continue;
      const label = dir === "in" ? "read from it" : "written to it";
      if (allowReorders) {
        const parts =
          dir === "in"
            ? [
                ["UI-independent stream", (e) => e.stream === "I"],
                ["UI-bound stream", (e) => e.stream === "B"],
              ]
            : [
                ["UI-independent stream", (e) => lineClass(e) === 1],
                ["UI-bound stream", (e) => lineClass(e) === 2],
                ["interface-report lines", (e) => lineClass(e) === 3],
                [
                  "UI-bound replies with the interface-report lines",
                  (e) => lineClass(e) === 2 || lineClass(e) === 3,
                ],
                ...WORKERS.map(({ name }) => [
                  `${name} worker replies`,
                  (e) =>
                    lineClass(e) === 4 &&
                    workerOf(e.kind.slice("reply:".length)) === name,
                ]),
              ];
        const broken = parts
          .map(([name, keep]) => [name, x.filter(keep), y.filter(keep)])
          .map(([name, xs, ys]) => [name, xs, ys, firstDifference(xs, ys)])
          .find(([, , , index]) => index >= 0);
        if (broken) {
          const [name, xs, ys, index] = broken;
          differences.push(
            `${plugin} (${label}), ${name}, line ${index + 1}:\n    A ${where(xs[index])}\n    B ${where(ys[index])}`,
          );
          break;
        }
        if (dir === "out") {
          // Each line of class 2 or 3 with a different number of core
          // lines before it moved.
          const placed = (list) => {
            let core = 0;
            const out = [];
            for (const entry of list) {
              const kind = lineClass(entry);
              if (kind === 1) core++;
              // A worker reply may move anywhere (its own order checked
              // above); it is not counted as a move.
              else if (kind !== 4) out.push([kind, core]);
            }
            return out;
          };
          const before = placed(x);
          const after = placed(y);
          const moved = { 2: 0, 3: 0 };
          before.forEach(([kind, core], i) => {
            if (after[i][1] !== core) moved[kind]++;
          });
          moves[2] += moved[2];
          moves[3] += moved[3];
          // Only worker replies moved (each worker's in order): nothing
          // to report (plan step S3b).
          if (moved[2] + moved[3] === 0) continue;
          reorders.push(
            `${plugin} (${label}): the UI-bound replies (${moved[2]} moved) and the interface-report lines (${moved[3]} moved) interleave differently with the core lines from line ${at + 1}; each class in order\n    A ${where(x[at])}\n    B ${where(y[at])}`,
          );
        } else {
          reorders.push(
            `${plugin} (${label}): the UI-bound and UI-independent streams interleave differently from line ${at + 1}; each stream in order\n    A ${where(x[at])}\n    B ${where(y[at])}`,
          );
        }
        continue;
      }
      differences.push(
        `${plugin} (${label}), line ${at + 1}:\n    A ${where(x[at])}\n    B ${where(y[at])}`,
      );
      break;
    }
  }
  // The move counts belong to the allowed-reorders mode only.
  return allowReorders
    ? { differences, reorders, moves }
    : { differences, reorders };
}

// ------------------------------------------------------------------ noise

/** A list of entries by position class: `kind` → its texts in order. */
function byKind(list) {
  const kinds = new Map();
  for (const entry of list) {
    if (!kinds.has(entry.kind)) kinds.set(entry.kind, []);
    kinds.get(entry.kind).push(entry.text);
  }
  return kinds;
}
const sameList = (x, y) =>
  x.length === y.length && x.every((text, i) => text === y[i]);
/** The order of position classes, those in `skip` left out. */
const kindOrder = (list, skip) =>
  list.filter((entry) => !skip.has(entry.kind)).map((entry) => entry.kind);

/**
 * The noise of two runs of one build (normalised traces): per plugin,
 * `{ presence: true }` when only one run has it, else per direction
 * `{ kinds: { [kind]: [every text either run had in it] }, interleave }`:
 * the position classes whose own sequence differs, and whether the order
 * of the other classes differs. Plugins and directions without noise are
 * left out.
 */
export function noiseOf(a, b) {
  const noise = {};
  const plugins = new Set([
    ...Object.keys(a.plugins),
    ...Object.keys(b.plugins),
  ]);
  for (const plugin of [...plugins].sort()) {
    const left = a.plugins[plugin];
    const right = b.plugins[plugin];
    if (!left || !right) {
      noise[plugin] = { presence: true };
      continue;
    }
    for (const dir of ["in", "out"]) {
      const x = left[dir] ?? [];
      const y = right[dir] ?? [];
      const kx = byKind(x);
      const ky = byKind(y);
      const kinds = {};
      for (const kind of new Set([...kx.keys(), ...ky.keys()])) {
        const tx = kx.get(kind) ?? [];
        const ty = ky.get(kind) ?? [];
        if (!sameList(tx, ty))
          kinds[kind] = [...new Set([...tx, ...ty])].sort();
      }
      const noisy = new Set(Object.keys(kinds));
      const interleave = !sameList(kindOrder(x, noisy), kindOrder(y, noisy));
      if (noisy.size || interleave)
        (noise[plugin] ??= {})[dir] = { kinds, interleave };
    }
  }
  return noise;
}

/** The size of a noise set: plugins, position classes, distinct lines. */
export function noiseSize(noise) {
  let plugins = 0;
  let presence = 0;
  let kinds = 0;
  let lines = 0;
  let interleave = 0;
  for (const entry of Object.values(noise)) {
    plugins++;
    if (entry.presence) presence++;
    for (const dir of ["in", "out"]) {
      const side = entry[dir];
      if (!side) continue;
      kinds += Object.keys(side.kinds).length;
      for (const texts of Object.values(side.kinds)) lines += texts.length;
      if (side.interleave) interleave++;
    }
  }
  return { plugins, presence, kinds, lines, interleave };
}

/**
 * Two normalised traces judged against a noise set (`noiseOf` of two base
 * runs): `{ differences: [text], suppressed }`. A difference is reported
 * when it is outside the noise: in a position class that was not noisy, a
 * line the base runs never had in a noisy class, a changed order of the
 * classes in a direction whose order was not noisy, or a plugin present in
 * one run only that was present in both base runs.
 */
export function compareWithNoise(a, b, noise = {}) {
  const differences = [];
  let suppressed = 0;
  const plugins = new Set([
    ...Object.keys(a.plugins),
    ...Object.keys(b.plugins),
  ]);
  for (const plugin of [...plugins].sort()) {
    const left = a.plugins[plugin];
    const right = b.plugins[plugin];
    const known = noise[plugin] ?? {};
    if (!left || !right) {
      if (known.presence) suppressed++;
      else differences.push(`${plugin}: only in ${left ? "A" : "B"}`);
      continue;
    }
    for (const dir of ["in", "out"]) {
      const label = dir === "in" ? "read from it" : "written to it";
      const base = known[dir] ?? { kinds: {}, interleave: false };
      const x = left[dir] ?? [];
      const y = right[dir] ?? [];
      const kx = byKind(x);
      const ky = byKind(y);
      const differing = new Set();
      for (const kind of [...new Set([...kx.keys(), ...ky.keys()])].sort()) {
        const tx = kx.get(kind) ?? [];
        const ty = ky.get(kind) ?? [];
        if (sameList(tx, ty)) continue;
        differing.add(kind);
        const seen = base.kinds[kind];
        if (!seen) {
          const at = tx.findIndex((text, i) => text !== ty[i]);
          const i = at < 0 ? tx.length : at;
          differences.push(
            `${plugin} (${label}), ${kind}, its line ${i + 1} (not noisy in the base runs):\n    A ${tx[i] ?? "(nothing)"}\n    B ${ty[i] ?? "(nothing)"}`,
          );
          continue;
        }
        const allowed = new Set(seen);
        const fresh = [...tx, ...ty].find((text) => !allowed.has(text));
        if (fresh)
          differences.push(
            `${plugin} (${label}), ${kind}: a line neither base run had:\n    ${fresh}`,
          );
        else suppressed++;
      }
      const skip = new Set([...differing, ...Object.keys(base.kinds)]);
      const ox = kindOrder(x, skip);
      const oy = kindOrder(y, skip);
      if (!sameList(ox, oy)) {
        if (base.interleave) suppressed++;
        else {
          const at = ox.findIndex((kind, i) => kind !== oy[i]);
          const i = at < 0 ? ox.length : at;
          differences.push(
            `${plugin} (${label}): the order of its position classes differs at ${i + 1} (A ${ox[i] ?? "(end)"}, B ${oy[i] ?? "(end)"}); not noisy in the base runs`,
          );
        }
      }
    }
  }
  return { differences, suppressed };
}

// ------------------------------------------------------------------ check

/** UI-bound methods answered with the user's choice (§5.1 notes 8, 9). */
const ANSWERED_BY_USER = new Set([
  "dialogs.open",
  "dialogs.confirm",
  "commands.run",
  "media.sources.locate",
]);
/**
 * UI-bound methods whose reply waits for a later UI fact, not for their own
 * execution: a mount batch applied (`MountApplied`), the window facts or
 * the 1 s deadline, the picture-in-picture state machine. Their replies may
 * leave after those of later UI-bound requests (seen today: a
 * `ui.setMount` answered before an earlier `ui.mount` of the same
 * plugin), so they are left out of the order check.
 */
const ANSWERED_LATER = new Set([
  "ui.mount",
  "ui.unmount",
  "window.setVideoArea",
  "window.setPictureInPicture",
]);
const ordered = (method) =>
  !ANSWERED_BY_USER.has(method) && !ANSWERED_LATER.has(method);
const same = (x, y) => JSON.stringify(x) === JSON.stringify(y);

/**
 * The rules one trace shows without a second run
 * (`{ violations: [text], notes: [text] }`), per plugin and process:
 *
 * - pairing: every reply answers a request made before it, once;
 * - R4, the UI-bound stream: those of its requests that are answered when
 *   they are carried out are answered in the order they arrived (not the
 *   four answered by the user, nor the four answered on a later UI fact:
 *   `ANSWERED_BY_USER`, `ANSWERED_LATER`);
 * - R5: a mount's `ui.preparing` (by slot) is not sent after its reply;
 * - R13: no `window.state` broadcast showing what a `window.setVideoArea`
 *   or `window.setPictureInPicture` changed goes out before its reply; the
 *   plugin's first `window.state` read after the reply sees the change
 *   (not on the 1 s timeout path); its first `layout.state` read after a
 *   `ui.mount` reply has that slot filled.
 *
 * Not seen on the wire (S3a hooks): that the core applied a UI report
 * before replying other than through a read; that a UI-independent request
 * did not wait for the UI (the stall test measures it); triggers before
 * the broadcast that flipped them (R6), which the wire cannot link.
 */
export function checkTrace(records, streams) {
  const violations = [];
  const notes = [];
  const runs = new Map();
  for (const record of records) {
    if (record.plugin === RUNNER || "failed" in record) continue;
    const key = `${record.plugin}\u0000${record.owner}`;
    if (!runs.has(key)) runs.set(key, []);
    runs.get(key).push(record);
  }
  for (const [key, list] of runs) {
    const [plugin, owner] = key.split("\u0000");
    const where = (record) => `${plugin} (owner ${owner}), seq ${record.seq}`;
    const pending = new Map();
    const bound = [];
    let lastWindow = null;
    let expectWindow = null;
    let expectLayout = null;
    const replied = new Map(); // slot → seq of the mount's reply
    const reads = new Map(); // read id → what it must show
    for (const record of list) {
      const line = record.line ?? {};
      if (record.dir === "in" && isRequest(line)) {
        const { stream } = streamOf(streams, line.method);
        const request = {
          ...line,
          seq: record.seq,
          ms: record.ms,
          stream,
          events: [],
        };
        pending.set(line.id, request);
        if (stream === "B" && ordered(line.method)) bound.push(line.id);
        if (line.method === "window.state" && expectWindow) {
          reads.set(line.id, expectWindow);
          expectWindow = null;
        }
        if (line.method === "layout.state" && expectLayout) {
          reads.set(line.id, expectLayout);
          expectLayout = null;
        }
        if (["ui.mount", "ui.unmount"].includes(line.method))
          replied.delete(line.params?.slot);
        continue;
      }
      if (record.dir === "out" && line.event === "window.state") {
        for (const request of pending.values())
          if (request.method.startsWith("window.set"))
            request.events.push({ data: line.data, before: lastWindow });
        lastWindow = line.data;
        continue;
      }
      if (record.dir === "out" && line.event === "ui.preparing") {
        const slot = line.data?.slot;
        if (replied.has(slot))
          violations.push(
            `${where(record)}: ui.preparing for ${slot} after its ui.mount reply (seq ${replied.get(slot)}) (R5)`,
          );
        continue;
      }
      if (record.dir !== "out" || !isReply(line)) continue;
      const request = pending.get(line.id);
      if (!request) {
        violations.push(
          `${where(record)}: a reply to request ${line.id}, which is not pending (pairing)`,
        );
        continue;
      }
      pending.delete(line.id);
      if (request.stream === "B" && ordered(request.method)) {
        if (bound[0] !== line.id)
          violations.push(
            `${where(record)}: ${request.method} (request seq ${request.seq}) answered before the earlier UI-bound request ${pending.get(bound[0])?.method ?? bound[0]} (R4)`,
          );
        bound.splice(bound.indexOf(line.id), 1);
      }
      const ok = "result" in line;
      if (ok && request.method === "window.setVideoArea") {
        const size = { width: line.result?.width, height: line.result?.height };
        for (const event of request.events)
          if (
            same(event.data?.videoArea, size) &&
            !same(event.before?.videoArea, size)
          )
            violations.push(
              `${where(record)}: window.state showing the video area ${size.width}x${size.height} went out before the window.setVideoArea reply (R13)`,
            );
        expectWindow =
          record.ms - request.ms >= 1000
            ? null
            : { what: "videoArea", value: size, reply: record.seq };
      }
      if (ok && request.method === "window.setPictureInPicture") {
        const on = Boolean(request.params?.on ?? request.params?.active);
        for (const event of request.events)
          if (
            event.data?.pictureInPicture?.active === on &&
            event.before?.pictureInPicture?.active !== on
          )
            violations.push(
              `${where(record)}: window.state showing picture-in-picture ${on} went out before the window.setPictureInPicture reply (R13)`,
            );
        expectWindow = {
          what: "pictureInPicture",
          value: on,
          reply: record.seq,
        };
      }
      if (ok && request.method === "ui.mount" && request.params?.slot) {
        replied.set(request.params.slot, record.seq);
        expectLayout = {
          what: "slot",
          value: request.params.slot,
          reply: record.seq,
        };
      }
      const read = reads.get(line.id);
      if (read && ok) {
        reads.delete(line.id);
        const result = line.result ?? {};
        if (read.what === "videoArea" && !same(result.videoArea, read.value))
          violations.push(
            `${where(record)}: the first window.state read after the window.setVideoArea reply (seq ${read.reply}) shows ${JSON.stringify(result.videoArea)}, not ${JSON.stringify(read.value)} (R13)`,
          );
        if (
          read.what === "pictureInPicture" &&
          result.pictureInPicture?.active !== read.value
        )
          violations.push(
            `${where(record)}: the first window.state read after the window.setPictureInPicture reply (seq ${read.reply}) does not show it (R13)`,
          );
        if (read.what === "slot" && read.value in result && !result[read.value])
          violations.push(
            `${where(record)}: the first layout.state read after the ui.mount reply (seq ${read.reply}) has ${read.value} empty (R13)`,
          );
      }
    }
    if (pending.size)
      notes.push(
        `${plugin} (owner ${owner}): ${pending.size} request(s) unanswered at the end of the trace (${[...new Set([...pending.values()].map((r) => r.method))].join(", ")})`,
      );
  }
  return { violations, notes };
}

// ------------------------------------------------------------------ files

const isDirectory = (path) => existsSync(path) && statSync(path).isDirectory();

/** The trace files under a directory, by relative path. */
function traceFiles(root) {
  const found = {};
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith(".wire.jsonl"))
        found[relative(root, path).replaceAll("\\", "/")] = path;
    }
  };
  walk(root);
  return found;
}

/** A V2 run directory's roots and versions (its run.json), per scenario. */
function runContext(root, file) {
  const info = join(root, "run.json");
  if (!existsSync(info)) return { roots: [], versions: [] };
  const run = JSON.parse(readFileSync(info, "utf8"));
  const scenario = file.split("/")[1];
  return {
    roots: [
      [run.profiles?.[scenario], "<profile>"],
      [run.stage, "<stage>"],
      [run.storeRoot, "<store-plugins>"],
      [run.media, "<media>"],
    ],
    versions: [run.release, run.sdk],
  };
}

/** Two files, or two directories' traces paired by relative path. */
function pairsOf(a, b) {
  if (isDirectory(a) && isDirectory(b)) {
    const left = traceFiles(a);
    const right = traceFiles(b);
    const names = [
      ...new Set([...Object.keys(left), ...Object.keys(right)]),
    ].sort();
    if (!names.length)
      throw new UsageError(`no *.wire.jsonl under ${a} or ${b}`);
    return names.map((name) => ({ name, a: left[name], b: right[name] }));
  }
  if (!isDirectory(a) && !isDirectory(b)) return [{ name: "trace", a, b }];
  throw new UsageError("compare two files or two directories");
}

/** Reads and normalises one side of a pair (`root`: its file or directory). */
function sideReader(options) {
  const streams =
    options.streams ?? JSON.parse(readFileSync(STREAMS_FILE, "utf8"));
  const unknown = new Set();
  const read = (path, root) => {
    const context = isDirectory(root)
      ? runContext(root, relative(root, path).replaceAll("\\", "/"))
      : { roots: [], versions: [] };
    const result = normaliseTrace(readTrace(readFileSync(path, "utf8")), {
      roots: [...context.roots, ...(options.roots ?? [])],
      versions: context.versions,
      streams,
    });
    for (const method of result.unknown) unknown.add(method);
    return result;
  };
  return { read, unknown, streams };
}

/**
 * `diff`: returns `{ text, failed, reorders, moved }` (`reorders`: the
 * allowed reorders listed; `moved`: per class 2 and 3, the lines moved
 * against the core lines; both 0 outside --allow-reorders).
 */
export function diffPaths(a, b, options = {}) {
  const pairs = pairsOf(a, b);
  const { read, unknown } = sideReader(options);
  const noise = options.noise
    ? JSON.parse(readFileSync(options.noise, "utf8"))
    : null;
  const lines = [
    "Wire trace differential",
    `  A  ${resolve(a)}`,
    `  B  ${resolve(b)}`,
    `  mode: ${noise ? `against the noise of ${options.noise}` : options.allowReorders ? "allowed reorders (R4; classes 1-3 of the S3a row)" : "strict"}`,
  ];
  let failed = 0;
  let suppressed = 0;
  const allReorders = [];
  const moved = { 2: 0, 3: 0 };
  for (const pair of pairs) {
    if (!pair.a || !pair.b) {
      failed++;
      lines.push(`${pair.name}: only in ${pair.a ? "A" : "B"}`);
      continue;
    }
    const left = read(pair.a, a);
    const right = read(pair.b, b);
    if (noise) {
      const result = compareWithNoise(left, right, noise.traces?.[pair.name]);
      failed += result.differences.length;
      suppressed += result.suppressed;
      lines.push(
        `${pair.name}: ${result.differences.length ? `${result.differences.length} difference(s) outside the noise` : "no difference outside the noise"}, ${result.suppressed} inside it`,
      );
      for (const text of result.differences) lines.push(`  ${text}`);
      continue;
    }
    const { differences, reorders, moves } = compareTraces(
      left,
      right,
      options,
    );
    failed += differences.length;
    moved[2] += moves?.[2] ?? 0;
    moved[3] += moves?.[3] ?? 0;
    lines.push(
      `${pair.name}: ${differences.length ? `${differences.length} plugin(s) differ` : "no difference"}${reorders.length ? `, ${reorders.length} allowed reorder(s)` : ""}`,
    );
    for (const text of differences) lines.push(`  ${text}`);
    for (const text of reorders) allReorders.push(`  ${pair.name}: ${text}`);
  }
  lines.push(
    failed
      ? `RESULT: FAIL, ${failed} difference(s)`
      : `RESULT: PASS, no difference${noise ? ` outside the noise (${suppressed} inside it)` : ""}`,
  );
  if (allReorders.length)
    lines.push(
      "",
      `ALLOWED REORDERS (rule R4; the S3a row's classes 2 and 3), not counted as differences: ${allReorders.length}; lines moved against the core lines: class 2 (UI-bound replies) ${moved[2]}, class 3 (interface-report lines) ${moved[3]}`,
      ...allReorders,
    );
  if (unknown.size)
    lines.push(
      "",
      `Methods not in the stream map (counted UI-independent): ${[...unknown].join(", ")}`,
    );
  return {
    text: lines.join("\n"),
    failed: failed > 0,
    reorders: allReorders.length,
    moved: { ...moved },
  };
}

/** `noise`: the noise set of two base runs; returns `{ text, noise }`. */
export function noisePaths(a, b, options = {}) {
  const { read } = sideReader(options);
  const noise = { format: 1, a: resolve(a), b: resolve(b), traces: {} };
  const lines = [
    "Wire trace noise of two base runs",
    `  A  ${noise.a}`,
    `  B  ${noise.b}`,
  ];
  const total = { plugins: 0, presence: 0, kinds: 0, lines: 0, interleave: 0 };
  for (const pair of pairsOf(a, b)) {
    if (!pair.a || !pair.b) {
      lines.push(`${pair.name}: only in ${pair.a ? "A" : "B"} (not recorded)`);
      continue;
    }
    const set = noiseOf(read(pair.a, a), read(pair.b, b));
    noise.traces[pair.name] = set;
    const size = noiseSize(set);
    for (const key of Object.keys(total)) total[key] += size[key];
    lines.push(
      `${pair.name}: ${size.plugins} noisy plugin(s) (${size.presence} present in one run only), ${size.kinds} noisy position class(es) with ${size.lines} distinct line(s), ${size.interleave} direction(s) with a noisy order`,
    );
  }
  lines.push(
    `NOISE: ${total.plugins} plugin(s), ${total.kinds} position class(es), ${total.lines} line(s), ${total.interleave} noisy order(s)`,
  );
  return { text: lines.join("\n"), noise };
}

/** `check`: the single-trace rules over a file or a directory's traces. */
export function checkPaths(path, options = {}) {
  const streams =
    options.streams ?? JSON.parse(readFileSync(STREAMS_FILE, "utf8"));
  const files = isDirectory(path) ? traceFiles(path) : { trace: path };
  const lines = [
    "Wire trace check (pairing, R4 UI-bound order, R5, R13)",
    `  ${resolve(path)}`,
  ];
  let failed = 0;
  for (const [name, file] of Object.entries(files).sort()) {
    const { violations, notes } = checkTrace(
      readTrace(readFileSync(file, "utf8")),
      streams,
    );
    failed += violations.length;
    lines.push(
      `${name}: ${violations.length ? `${violations.length} violation(s)` : "no violation"}`,
    );
    for (const text of violations) lines.push(`  ${text}`);
    for (const text of notes) lines.push(`  note: ${text}`);
  }
  lines.push(
    failed
      ? `RESULT: FAIL, ${failed} violation(s)`
      : "RESULT: PASS, no violation",
  );
  return { text: lines.join("\n"), failed: failed > 0 };
}

// ------------------------------------------------------------------- main

function parse(argv) {
  const options = { _: [], roots: [] };
  for (let i = 0; i < argv.length; i++) {
    const name = argv[i];
    if (name === "--allow-reorders") options.allowReorders = true;
    else if (name === "--root") {
      const value = argv[++i] ?? "";
      const at = value.lastIndexOf("=");
      if (at <= 0) throw new UsageError("--root needs <path>=<token>");
      options.roots.push([value.slice(0, at), value.slice(at + 1)]);
    } else if (["--report", "--noise", "--out"].includes(name)) {
      if (i + 1 >= argv.length) throw new UsageError(`${name} needs a value`);
      options[name.slice(2)] = argv[++i];
    } else if (name.startsWith("--")) throw new UsageError(`unknown ${name}`);
    else options._.push(name);
  }
  return options;
}

export function main(argv) {
  const [command, ...rest] = argv;
  const options = parse(rest);
  const report = (text) => {
    console.log(text);
    if (options.report) writeFileSync(resolve(options.report), text + "\n");
  };
  if (command === "diff" && options._.length === 2) {
    if (options.noise && options.allowReorders)
      throw new UsageError("--noise and --allow-reorders are separate modes");
    const { text, failed } = diffPaths(options._[0], options._[1], options);
    report(text);
    return failed ? 1 : 0;
  }
  if (command === "noise" && options._.length === 2 && options.out) {
    const { text, noise } = noisePaths(options._[0], options._[1], options);
    writeFileSync(resolve(options.out), JSON.stringify(noise) + "\n");
    report(`${text}\n  written: ${resolve(options.out)}`);
    return 0;
  }
  if (command === "check" && options._.length === 1) {
    const { text, failed } = checkPaths(options._[0], options);
    report(text);
    return failed ? 1 : 0;
  }
  throw new UsageError(
    "usage: verify-wire.mjs diff <A> <B> [--allow-reorders | --noise <file>] [--report <file>] [--root <path>=<token>] | noise <base A> <base B> --out <file> | check <trace or directory> (see the header)",
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof UsageError) && !(error instanceof SyntaxError))
      throw error;
    console.error(`verify-wire: ${error.message}`);
    process.exitCode = 2;
  }
}
