#!/usr/bin/env node
/**
 * Runtime differential of two staged players (doc/process/verify-additions.md):
 * runs the same start-up, standby and wake scenarios on each, offscreen and in
 * isolated profiles, and compares what the host's registries, its standby
 * log lines and the standby recordings say for every plugin. An empty
 * difference is a pass. It uses the host's existing test switches only
 * (host/src/host/supervisor/probe.rs, ui/offscreen.rs).
 *
 *   node src/scripts/verify-runtime.mjs run --stage <staged player> --out <directory>
 *     [--store-plugins <plugin repository>]   default C:\vestra\plugins-repo: each
 *                                   plugins/<name> with a manifest.json is loaded
 *                                   with --plugin (its dist/ when built, else
 *                                   the source directory)
 *     [--no-store-plugins]          the built-ins only
 *     [--scenarios idle,park,file,installed]   default: all four
 *     [--media <file>]              the file of the `file` scenario; default: a
 *                                   two-minute test clip made with the stage's ffmpeg
 *     [--slow <factor>]             multiply every run time (a loaded machine)
 *     [--quiet-flag <file>]         wait while it exists before each player run
 *                                   (default C:\vestra\quiet.flag)
 *     [--wire-trace]                each step also writes its wire trace
 *                                   (VESTRA_TEST_WIRE_TRACE, a host test switch
 *                                   since the core thread's S1.1) to
 *                                   raw/<scenario>/<step>.wire.jsonl, compared
 *                                   by verify-wire.mjs diff <A> <B>; a host
 *                                   without the switch writes none
 *   node src/scripts/verify-runtime.mjs diff <directory A> <directory B> [--report <file>]
 *   node src/scripts/verify-runtime.mjs compare --a <staged player> --b <staged player>
 *     --out <directory> [the options of run]     run A, run B, diff
 *
 * <staged player>: the directory with vestra-host.exe and resources\ (Windows,
 * packaging/bundle-windows.mjs --no-installer), or a `Vestra Player.app`.
 *
 * Scenarios, each in its own new profile under <out>/raw/<scenario>/profile:
 *
 *   idle  first:  a first start, then idle past the standby threshold (the
 *                 player's own 30 s; the run lasts until 40 s after
 *                 `lazy: start-up settled`)
 *         second: the next start of that profile (recordings replayed)
 *   park  park:   VESTRA_TEST_PARK=all, every plugin parked at its ready
 *         wake:   the next start, VESTRA_TEST_WAKE: replayed, then all woken
 *   file  warm:   a first start (the recordings of a player started before)
 *         open:   a start with a file argument, played for 20 s
 *   installed     the store plugins as packages installed from a file in the
 *                 profile's library instead of --plugin (development plugins
 *                 are never parked): needs them built (dist/)
 *         first:  a first start, then idle past the standby threshold
 *         wake:   the next start, then every parked plugin woken
 *
 * Compared per scenario, step and plugin (<out>/normalised.json):
 *
 *   registry   the VESTRA_TEST_REGISTRY lines in order (moment + commands, key
 *              bindings, settings pages, drop providers, wake events with
 *              their recorded marks). At `ready`, `parking` and `woken` a
 *              command's state and a page's values are left out: plugins set
 *              some after their ready (tests/lazy_activation.rs does the same);
 *              they are compared at `parked`, `replayed` and `quit`.
 *   log        the host's lines about the plugin, in order: `lazy:`
 *              (classified lazy|resident with its reasons, parked (N recorded
 *              calls), replayed, woken by, stays running …), `plugin <id>:
 *              keeps running | may be stopped while idle`, `supervisor:`,
 *              `shared runner: <id> admitted`, `runtime.failed`
 *   console    what the plugin wrote to its console (plugins.log), except
 *              `HostError: plugin is stopping` / `plugin stopped` (a
 *              request on its way when the plugin was stopped)
 *   recording  its standby recording after the step (residentAtReady, calls)
 *   and under `host`: the lines that name no plugin (`lazy: start-up settled`,
 *   `source: phase`, `open:`) and each `shell: compile with N panel(s) […]`
 *   (what the shell compiled with, its panels sorted; not in the `park`
 *   scenario, where plugins park in any order between compiles; the first
 *   and the last only in installed/first), all sorted.
 *   panels     each `panel: compile [<id>:<slot>] … (ready|failed)` (per-panel
 *              compilation, player 0.9.0: a panel's file compiles on its own),
 *              sorted (plugins start in any order). A difference here, and in
 *              the `shell: compile` lines above, is the compile pattern: `diff`
 *              lists it in a section of its own and does not count it in the
 *              result (the ruled change of SDK 1.9.0 C1, compared against a
 *              player before it; read it against the ruled pattern).
 *
 * Normalised away: owner, runtime and process numbers, times in ms, source
 * line numbers (`main.js:58`), instance GUIDs, package digests, the build
 * commit, the versions of the player, host, SDK and every plugin, and the
 * paths of the stage, the profile, the plugin repository and the media file.
 *
 * Exit code: 0 no difference (run: the runs are valid), 1 differences,
 * 2 usage error or an invalid run (a player that failed, a run that ended
 * before it had idled long enough: nothing was compared).
 * No dependencies. Starts no window: every run has VESTRA_TEST_OFFSCREEN=1.
 */
import { spawn, spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { packageDigest } from "./stage-plugins.mjs";

class RunError extends Error {}

/** Seconds: how long each step runs, and what it must have seen by then. */
const STEPS = {
  idle: [
    // `settleWithin`: start-up must have settled by then; the run goes on
    // `idleAfterSettled` after that at least (it lasts the sum).
    { name: "first", settleWithin: 45, idleAfterSettled: 40 },
    { name: "second", settleWithin: 20, idleAfterSettled: 10 },
  ],
  park: [
    {
      name: "park",
      settleWithin: 30,
      idleAfterSettled: 10,
      env: { VESTRA_TEST_PARK: "all", VESTRA_IDLE_MS: "600000" },
      // Plugins park one after another in any order, and the shell compiles
      // again in between: which panels each compile has is not compared.
      compiles: false,
    },
    {
      // The wake comes `wakeAfterSettled` after `settleWithin`.
      name: "wake",
      settleWithin: 15,
      idleAfterSettled: 25,
      wakeAfterSettled: 5,
      env: { VESTRA_TEST_PARK: "all", VESTRA_IDLE_MS: "600000" },
      // Plugins park one after another in any order, and the shell compiles
      // again in between: which panels each compile has is not compared.
      compiles: false,
    },
  ],
  file: [
    { name: "warm", settleWithin: 30, idleAfterSettled: 10 },
    { name: "open", settleWithin: 30, idleAfterSettled: 20, media: true },
  ],
  installed: [
    {
      name: "first",
      settleWithin: 45,
      idleAfterSettled: 40,
      // Third-party first runs start a few at a time: their panels reach
      // the shell in one compile or in two. The first compile (the
      // built-ins') and the last (every panel) are compared.
      compiles: "ends",
    },
    {
      name: "wake",
      settleWithin: 15,
      idleAfterSettled: 25,
      wakeAfterSettled: 5,
    },
  ],
};
/** Scenarios whose store plugins are installed packages, not `--plugin`. */
const INSTALLED = new Set(["installed"]);

/** The host's lines that are compared (the rest is timing and diagnosis). */
const COMPARED_LINES = [
  /^lazy: /,
  /^plugin \S+: (keeps running|may be stopped) while idle/,
  /^supervisor: /,
  /^shared runner \d+: \S+ admitted/,
  /^shared runner: /,
  /^runtime \d+ runtime\.failed/,
  /^installed package /,
  /^packages\.\w+ /,
  /^plugin recorder: /,
  /^shell: compile /,
  /^panel: compile /,
  /^source: phase /,
  /^open: /,
  /^host\.warning /,
  /^exit: /,
];
/** Machine and timing facts inside compared lines. */
const DROPPED_LINES = [
  /^supervisor: idle after \d+ ms, \d+ starts at once/,
  /^supervisor: memory pressure/,
  /^lazy: owner \d+ ready after/,
  /^supervisor: owner \d+ ready/,
  // Shown only when a start takes longer than the notice delay.
  /^supervisor: OSD "Starting /,
];
/**
 * A request that was on its way when its plugin was stopped (at a park or at
 * quit): whether one is depends on the moment.
 */
const DROPPED_CONSOLE = [/^HostError: plugin (is stopping|stopped)\b/];

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * The normaliser of one build's output. `context`: `{ roots: [[path, token]],
 * versions: [string], digests: { hex: id }, guids: { guid: id }, build: string
 * | null, ids: [plugin id] }`.
 */
export function normaliser(context) {
  const roots = [...context.roots]
    .filter(([path]) => path)
    .sort((a, b) => b[0].length - a[0].length)
    .map(([path, token]) => {
      // C:\a\b, C:\\a\\b (inside JSON text) and C:/a/b; any case on Windows.
      const parts = path
        .split(/[\\/]+/)
        .filter(Boolean)
        .map(escapeRegExp);
      const lead = /^[\\/]/.test(path) ? "(?:\\\\\\\\|\\\\|/)" : "";
      return [
        new RegExp(lead + parts.join("(?:\\\\\\\\|\\\\|/)"), "gi"),
        token,
      ];
    });
  const versions = [...new Set(context.versions.filter(Boolean))].sort(
    (a, b) => b.length - a.length,
  );
  const version = versions.length
    ? new RegExp(
        `(?<![\\w.])v?(?:${versions.map(escapeRegExp).join("|")})(?![\\w]|\\.\\d)`,
        "g",
      )
    : null;
  const guid = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;
  const digest = /\b[0-9a-f]{64}\b/g;
  const build = context.build
    ? new RegExp(`\\b${escapeRegExp(context.build)}\\b`, "g")
    : null;
  const ids = [...(context.ids ?? [])].sort((a, b) => b.length - a.length);
  const idPattern = ids.length
    ? new RegExp(`(?<![\\w.-])(${ids.map(escapeRegExp).join("|")})(?![\\w-])`)
    : null;

  /** Any string: paths, GUIDs, digests, the build, versions. */
  const text = (value) => {
    let out = value;
    for (const [pattern, token] of roots) out = out.replace(pattern, token);
    out = out
      .replace(guid, (found) => {
        const id = context.guids?.[found.toLowerCase()];
        return id ? `<guid:${id}>` : "<guid>";
      })
      .replace(digest, (found) => {
        const id = context.digests?.[found.toLowerCase()];
        return id ? `<digest:${id}>` : "<digest>";
      });
    if (build) out = out.replace(build, "<build>");
    if (version) out = out.replace(version, "<version>");
    return out;
  };
  /** A host log line: also its numbers. */
  const line = (value) =>
    text(value)
      .replace(/\bowner \d+ → \d+/g, "owner N → N")
      .replace(/\bowner \d+/g, "owner N")
      .replace(/^runtime \d+ /, "runtime N ")
      .replace(/^shared runner \d+:/, "shared runner N:")
      .replace(/\bpid \d+/g, "pid N")
      .replace(/\bprocess \d+/g, "process N")
      .replace(/\+?\b\d+(\.\d+)? ms\b/g, "N ms")
      .replace(/\b\d+(\.\d+)? s\b/g, "N s")
      .replace(/\.([cm]?[jt]s):\d+(:\d+)?/g, ".$1:N");
  /** Every string of a JSON value. */
  const value = (input) => {
    if (typeof input === "string") return text(input);
    if (Array.isArray(input)) return input.map(value);
    if (input && typeof input === "object")
      return Object.fromEntries(
        Object.entries(input).map(([key, item]) => [key, value(item)]),
      );
    return input;
  };
  /** The plugin a line is about, or null. */
  const pluginOf = (raw) => idPattern?.exec(raw)?.[1] ?? null;
  return { text, line, value, pluginOf };
}

/** A registry snapshot as compared at `moment` (see the header). */
export function registryAt(moment, registry) {
  if (!["ready", "parking", "woken"].includes(moment)) return registry;
  const without = (list, field) =>
    (list ?? []).map((entry) => ({ ...entry, [field]: null }));
  return {
    ...registry,
    commands: without(registry.commands, "state"),
    pages: without(registry.pages, "properties"),
  };
}

/**
 * One step's raw output as compared: `{ plugins: { id: { registry, log,
 * console, recording } }, host: { log } }`. `raw`: `{ stderr, registry,
 * pluginsLog, recordings: { guid: object }, catalog }` (texts and parsed JSON).
 */
export function normaliseStep(raw, context) {
  const guids = {};
  for (const [id, entry] of Object.entries(raw.catalog?.installed ?? {}))
    if (entry?.instance?.guid) guids[entry.instance.guid.toLowerCase()] = id;
  const normalise = normaliser({ ...context, guids });
  const plugins = {};
  const plugin = (id) =>
    (plugins[id] ??= { registry: [], log: [], console: [], recording: null });
  for (const id of context.ids ?? []) plugin(id);

  for (const text of raw.registry.split("\n")) {
    if (!text.trim()) continue;
    const entry = JSON.parse(text);
    plugin(entry.id).registry.push({
      moment: entry.moment,
      registry: normalise.value(registryAt(entry.moment, entry.registry)),
    });
  }
  const host = [];
  const compiles = [];
  const panelCompiles = [];
  for (const text of raw.stderr.split("\n")) {
    const trimmed = text.replace(/\r$/, "");
    if (!COMPARED_LINES.some((pattern) => pattern.test(trimmed))) continue;
    if (DROPPED_LINES.some((pattern) => pattern.test(trimmed))) continue;
    if (trimmed.startsWith("shell: compile ")) {
      // The panels of one compile arrive in any order.
      const sorted = trimmed.replace(
        /\[([^\]]*)\]/,
        (_, panels) => `[${panels.split(", ").sort().join(", ")}]`,
      );
      compiles.push(normalise.line(sorted));
      continue;
    }
    if (trimmed.startsWith("panel: compile ")) {
      panelCompiles.push(normalise.line(trimmed));
      continue;
    }
    const id = normalise.pluginOf(trimmed);
    (id ? plugin(id).log : host).push(normalise.line(trimmed));
  }
  // The shell's compiles: all of them; or the first and the last (`ends`:
  // the ones between depend on which first runs came in together); or none.
  if (context.compiles === "ends") {
    const ends = (which, line) =>
      line.replace("shell: compile with", `shell: ${which} compile with`);
    if (compiles.length) host.push(ends("first", compiles[0]));
    if (compiles.length > 1) host.push(ends("last", compiles.at(-1)));
  } else if (context.compiles !== false) host.push(...compiles);
  // `<UTC time> [pid] [<id>] text`
  for (const text of (raw.pluginsLog ?? "").split("\n")) {
    const found = /^\S+ \[\d+\] \[([^\]]+)\] (.*)$/.exec(
      text.replace(/\r$/, ""),
    );
    if (!found || DROPPED_CONSOLE.some((pattern) => pattern.test(found[2])))
      continue;
    plugin(found[1]).console.push(normalise.line(found[2]));
  }
  for (const [guid, recording] of Object.entries(raw.recordings ?? {})) {
    const id = guids[guid.toLowerCase()] ?? `<unknown ${guid}>`;
    const {
      player: _player,
      host: _host,
      build: _build,
      digest: _digest,
      version: _version,
      recordedAt: _recordedAt,
      ...kept
    } = recording;
    plugin(id).recording = normalise.value(kept);
  }
  return {
    plugins: Object.fromEntries(
      Object.keys(plugins)
        .sort()
        .map((id) => [id, plugins[id]]),
    ),
    host: { log: host.sort() },
    panels: { compiles: panelCompiles.sort() },
  };
}

/**
 * The compile pattern among `differences` lines: the `panels` section's and
 * the host's `shell: compile` lines (`compiles`), reported apart from the
 * rest (`rest`, what decides the result).
 */
export function compilePattern(lines) {
  const compile =
    /^[^›]+ › [^›]+ › (?:panels(?: › compiles)?: |host › log: only in [AB](?: \(×\d+\))?: shell: (?:first |last )?compile with )/;
  return {
    compiles: lines.filter((line) => compile.test(line)),
    rest: lines.filter((line) => !compile.test(line)),
  };
}

const short = (value) => {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 240 ? `${text.slice(0, 240)}…` : text;
};

/**
 * The differences of two normalised values as lines `path: what`. Lists of
 * strings are compared as lines (missing, extra, order); lists of entries
 * with an `id` by id, then order; registry snapshots by position.
 */
export function differences(a, b, path = "") {
  const out = [];
  const at = (key) => (path ? `${path} › ${key}` : String(key));
  if (Array.isArray(a) && Array.isArray(b)) {
    const strings = [...a, ...b].every((item) => typeof item === "string");
    const byId =
      !strings &&
      [...a, ...b].every(
        (item) =>
          item && typeof item === "object" && typeof item.id === "string",
      ) &&
      new Set(a.map((item) => item.id)).size === a.length &&
      new Set(b.map((item) => item.id)).size === b.length;
    if (strings) {
      const count = (list) => {
        const map = new Map();
        for (const item of list) map.set(item, (map.get(item) ?? 0) + 1);
        return map;
      };
      const [left, right] = [count(a), count(b)];
      let unequal = false;
      for (const [item, n] of left) {
        const m = right.get(item) ?? 0;
        if (n > m) {
          unequal = true;
          out.push(
            `${path}: only in A${n - m > 1 ? ` (×${n - m})` : ""}: ${short(item)}`,
          );
        }
      }
      for (const [item, m] of right) {
        const n = left.get(item) ?? 0;
        if (m > n) {
          unequal = true;
          out.push(
            `${path}: only in B${m - n > 1 ? ` (×${m - n})` : ""}: ${short(item)}`,
          );
        }
      }
      if (!unequal) {
        const first = a.findIndex((item, index) => item !== b[index]);
        if (first >= 0)
          out.push(
            `${path}: same lines in another order, from line ${first + 1}: A ${short(a[first])} | B ${short(b[first])}`,
          );
      }
      return out;
    }
    if (byId) {
      const right = new Map(b.map((item) => [item.id, item]));
      const left = new Set(a.map((item) => item.id));
      for (const item of a) {
        if (!right.has(item.id)) out.push(`${at(item.id)}: only in A`);
        else out.push(...differences(item, right.get(item.id), at(item.id)));
      }
      for (const item of b)
        if (!left.has(item.id)) out.push(`${at(item.id)}: only in B`);
      const order = (list, other) =>
        list.map((item) => item.id).filter((id) => other.has(id));
      const [orderA, orderB] = [order(a, right), order(b, left)];
      const first = orderA.findIndex((id, index) => id !== orderB[index]);
      if (first >= 0)
        out.push(
          `${path}: order differs from entry ${first + 1}: A ${orderA[first]} | B ${orderB[first]}`,
        );
      return out;
    }
    const moments = (list) =>
      list.every((item) => item && typeof item.moment === "string")
        ? list.map((item) => item.moment).join(" → ")
        : `${list.length} entries`;
    if (a.length !== b.length) {
      out.push(`${path}: A has ${moments(a)}; B has ${moments(b)}`);
      return out;
    }
    a.forEach((item, index) => {
      const label =
        item && typeof item.moment === "string"
          ? `${index + 1}:${item.moment}`
          : `[${index}]`;
      out.push(...differences(item, b[index], at(label)));
    });
    return out;
  }
  const object = (value) =>
    value && typeof value === "object" && !Array.isArray(value);
  if (object(a) && object(b)) {
    for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) {
      if (!(key in b)) out.push(`${at(key)}: only in A: ${short(a[key])}`);
      else if (!(key in a)) out.push(`${at(key)}: only in B: ${short(b[key])}`);
      else out.push(...differences(a[key], b[key], at(key)));
    }
    return out;
  }
  if (JSON.stringify(a) !== JSON.stringify(b))
    out.push(`${path}: A ${short(a)} | B ${short(b)}`);
  return out;
}

/** What a step says about each plugin's standby, for the summary table. */
export function standbySummary(step) {
  const rows = {};
  for (const [id, plugin] of Object.entries(step.plugins)) {
    const has = (pattern) => plugin.log.some((line) => pattern.test(line));
    const count = (pattern) =>
      plugin.log.filter((line) => pattern.test(line)).length;
    rows[id] = {
      class: has(/classified resident/)
        ? "resident"
        : has(/classified lazy/)
          ? "lazy"
          : has(/keeps running while idle/)
            ? "keeps running"
            : has(/may be stopped while idle/)
              ? "may be stopped"
              : has(/starts: resident at its last ready/)
                ? "resident (replayed)"
                : has(/ replayed \(/)
                  ? "replayed"
                  : "-",
      parked: count(/ parked \(/),
      woken: count(/ woken by /),
      failed: has(/runtime\.failed/),
      moments: plugin.registry.map((entry) => entry.moment).join(","),
      recording: plugin.recording
        ? `${plugin.recording.residentAtReady ? "resident" : "lazy"}, ${plugin.recording.calls.length} calls`
        : "-",
    };
  }
  return rows;
}

// ---------------------------------------------------------------- running

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));

/** The parts of a staged player. */
function stageOf(directory) {
  const root = resolve(directory);
  const layouts = [
    { exe: join(root, "vestra-host.exe"), resources: join(root, "resources") },
    {
      exe: join(root, "Contents", "MacOS", "vestra-host"),
      resources: join(root, "Contents", "Resources"),
    },
  ];
  const layout = layouts.find((entry) => existsSync(entry.exe));
  if (!layout)
    throw new RunError(`${root}: no vestra-host.exe (not a staged player)`);
  const { resources } = layout;
  const builtins = readJson(join(resources, "plugins", "builtin.json")).plugins;
  const sdk = join(
    resources,
    "runtime/node_modules/@vestra-player/sdk/package.json",
  );
  const ffmpeg = [
    join(root, "ffmpeg", "ffmpeg.exe"),
    join(resources, "ffmpeg", "ffmpeg"),
  ].find((path) => existsSync(path));
  return {
    root,
    exe: layout.exe,
    resources,
    ffmpeg,
    release: readJson(join(resources, "release.json")).version,
    sdk: existsSync(sdk) ? readJson(sdk).version : null,
    builtins: builtins.map((entry) => ({
      id: entry.id,
      version: entry.version,
      digest: entry.digest,
    })),
  };
}

/** The store plugins to load with --plugin: `[{ id, version, directory }]`. */
function storePlugins(repository) {
  const root = join(resolve(repository), "plugins");
  if (!existsSync(root))
    throw new RunError(`${repository}: no plugins/ directory`);
  const found = [];
  for (const name of readdirSync(root).sort()) {
    const source = join(root, name);
    if (!existsSync(join(source, "manifest.json"))) continue;
    const built = join(source, "dist");
    const directory = existsSync(join(built, "manifest.json")) ? built : source;
    const manifest = readJson(join(directory, "manifest.json"));
    found.push({
      id: manifest.id,
      version: manifest.version,
      directory,
      built: directory === built,
    });
  }
  return found;
}

/** Runs one player to its end; resolves with its log and what was seen when. */
async function runPlayer({ stage, profile, step, args, env, log, slow }) {
  const inherited = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !/^VESTRA_/i.test(key)),
  );
  const child = spawn(stage.exe, args, {
    env: {
      ...inherited,
      VESTRA_TEST_OFFSCREEN: "1",
      VESTRA_PACKAGE_STORE: join(profile, "store"),
      VESTRA_PLUGIN_DATA: join(profile, "data"),
      VESTRA_UI_STATE: join(profile, "ui-state.json"),
      VESTRA_INSTANCE_DIR: join(profile, "instance"),
      VESTRA_SHADER_CACHE: join(profile, "shader-cache"),
      VESTRA_LOG_FILE: join(profile, "host.log"),
      APPDATA: join(profile, "appdata"),
      LOCALAPPDATA: join(profile, "local"),
      VESTRA_LANGUAGE: "en",
      ...env,
    },
    stdio: ["ignore", "ignore", "pipe"],
    windowsHide: true,
  });
  const started = Date.now();
  let stderr = "";
  let pending = "";
  const seen = { settled: null, wake: null, opened: null, normalExit: false };
  let settle;
  const settledPromise = new Promise((done) => (settle = done));
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
    pending += chunk;
    const lines = pending.split("\n");
    pending = lines.pop();
    for (const line of lines) {
      const at = (Date.now() - started) / 1000;
      if (line.startsWith("lazy: start-up settled")) {
        seen.settled ??= at;
        settle();
      } else if (line.startsWith("lazy: VESTRA_TEST_WAKE")) seen.wake ??= at;
      else if (line.includes(" handed over at +")) seen.opened ??= at;
      else if (line.startsWith("exit: normal")) seen.normalExit = true;
    }
  });
  const exited = new Promise((done, fail) => {
    child.on("error", fail);
    child.on("close", (code, signal) => done({ code, signal }));
  });
  // `--seconds` is fixed at launch: the run is given the longest it may need
  // and judged afterwards by what it saw.
  const limit = (step.total + 90) * 1000 * slow;
  const timer = setTimeout(() => child.kill(), limit);
  const result = await exited;
  clearTimeout(timer);
  settle();
  await settledPromise;
  writeFileSync(log, stderr);
  return {
    stderr,
    seen,
    code: result.code,
    signal: result.signal,
    seconds: (Date.now() - started) / 1000,
  };
}

async function waitWhileQuiet(flag) {
  let said = false;
  while (flag && existsSync(flag)) {
    if (!said) console.log(`  waiting: ${flag} exists`);
    said = true;
    await sleep(30_000);
  }
}

/** A two-minute test clip (longer than the run that plays it). */
function makeClip(stage, path) {
  if (!stage.ffmpeg)
    throw new RunError("the stage has no ffmpeg: give --media <file>");
  const made = spawnSync(
    stage.ffmpeg,
    [
      "-v",
      "error",
      "-f",
      "lavfi",
      "-i",
      "testsrc2=size=640x360:rate=30:duration=120",
      "-f",
      "lavfi",
      "-i",
      "sine=duration=120",
      "-c:v",
      "libx264",
      "-preset",
      "ultrafast",
      "-pix_fmt",
      "yuv420p",
      "-shortest",
      "-y",
      path,
    ],
    { encoding: "utf8" },
  );
  if (made.status !== 0)
    throw new RunError(`ffmpeg could not make ${path}: ${made.stderr}`);
}

/**
 * The store plugins as packages installed from a file, in the library of
 * `profile` (as tests/startup_compiles.rs installs its fixture): each built
 * plugin under `packages/<digest>/` with its record in `catalog.json`.
 * Returns `{ digest: id }`.
 */
function installStorePlugins(profile, store) {
  const library = join(profile, "store");
  const installed = {};
  const digests = {};
  store.forEach((plugin, index) => {
    if (!plugin.built)
      throw new RunError(
        `${plugin.directory}: the \`installed\` scenario needs built plugins (npm run build in the plugin repository), or leave it out with --scenarios`,
      );
    const files = new Map();
    for (const item of readdirSync(plugin.directory, {
      recursive: true,
      withFileTypes: true,
    })) {
      if (!item.isFile()) continue;
      const path = join(item.parentPath, item.name);
      files.set(
        relative(plugin.directory, path).split(sep).join("/"),
        readFileSync(path),
      );
    }
    const digest = packageDigest(files);
    const directory = join(library, "packages", digest);
    cpSync(plugin.directory, directory, { recursive: true });
    digests[digest] = plugin.id;
    installed[plugin.id] = {
      revision: 1,
      current: {
        digest,
        manifest: readJson(join(directory, "manifest.json")),
        directory,
      },
      previous: null,
      instance: {
        guid: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
        source: { kind: "file" },
        installedAt: 1,
      },
    };
  });
  writeFileSync(
    join(library, "catalog.json"),
    JSON.stringify({ installed, pending: {}, operations: {}, lastCommit: {} }),
  );
  return digests;
}

/** `run`: every scenario on one stage; writes <out>/normalised.json. */
async function run(options) {
  const stage = stageOf(options.stage);
  const out = resolve(options.out);
  const slow = Number(options.slow ?? 1);
  if (!(slow >= 1)) throw new RunError("--slow needs a factor of 1 or more");
  const scenarios = (options.scenarios ?? "idle,park,file,installed").split(
    ",",
  );
  for (const name of scenarios)
    if (!STEPS[name]) throw new RunError(`unknown scenario ${name}`);
  const store = options.noStorePlugins
    ? []
    : storePlugins(options.storePlugins ?? "C:\\vestra\\plugins-repo");
  const storeRoot = options.noStorePlugins
    ? null
    : resolve(options.storePlugins ?? "C:\\vestra\\plugins-repo");
  rmSync(join(out, "raw"), { recursive: true, force: true });
  mkdirSync(join(out, "raw"), { recursive: true });

  let media = options.media ? resolve(options.media) : null;
  if (scenarios.includes("file") && !media) {
    media = join(out, "raw", "clip.mp4");
    makeClip(stage, media);
  }
  const info = {
    stage: stage.root,
    release: stage.release,
    sdk: stage.sdk,
    hostLine: null,
    builtins: stage.builtins,
    storePlugins: store,
    storeRoot,
    media,
    // Where each scenario's profile was when it ran (its paths are in the
    // raw output, also after <out> is moved).
    profiles: {},
    steps: {},
  };
  console.log(
    `stage ${stage.root}: player ${stage.release}, SDK ${stage.sdk}, ${stage.builtins.length} built-ins, ${store.length} store plugins${store.some((p) => !p.built) ? " (some from source: no dist/)" : ""}`,
  );
  const invalid = [];
  for (const name of scenarios) {
    const directory = join(out, "raw", name);
    const profile = join(directory, "profile");
    mkdirSync(join(profile, "store"), { recursive: true });
    info.profiles[name] = profile;
    const packages = INSTALLED.has(name);
    if (packages) installStorePlugins(profile, store);
    for (const template of STEPS[name]) {
      const step = {
        ...template,
        total: template.settleWithin + template.idleAfterSettled,
      };
      const seconds = Math.ceil(step.total * slow);
      const registry = join(directory, `${step.name}.registry.jsonl`);
      const env = { ...step.env, VESTRA_TEST_REGISTRY: registry };
      if (options.wireTrace)
        env.VESTRA_TEST_WIRE_TRACE = join(directory, `${step.name}.wire.jsonl`);
      if (step.wakeAfterSettled)
        env.VESTRA_TEST_WAKE = String(
          Math.ceil((step.settleWithin + step.wakeAfterSettled) * slow * 1000),
        );
      const args = ["--silent", "--seconds", String(seconds)];
      if (!packages)
        for (const plugin of store) args.push("--plugin", plugin.directory);
      if (step.media) args.push(media);
      // The plugin log is appended to across the steps of a profile.
      rmSync(join(profile, "plugins.log"), { force: true });
      await waitWhileQuiet(options.quietFlag);
      process.stdout.write(`  ${name}/${step.name}: ${seconds} s … `);
      const result = await runPlayer({
        stage,
        profile,
        step,
        args,
        env,
        log: join(directory, `${step.name}.stderr.txt`),
        slow,
      });
      // What the step left: recordings, the catalog, the plugin console.
      const activation = join(profile, "store", "activation");
      if (existsSync(activation))
        cpSync(activation, join(directory, `${step.name}.recordings`), {
          recursive: true,
        });
      const catalog = join(profile, "store", "catalog.json");
      if (existsSync(catalog))
        cpSync(catalog, join(directory, `${step.name}.catalog.json`));
      writeFileSync(
        join(directory, `${step.name}.plugins.log`),
        existsSync(join(profile, "plugins.log"))
          ? readFileSync(join(profile, "plugins.log"), "utf8")
          : "",
      );

      const problems = [];
      const { seen } = result;
      if (result.code !== 0)
        problems.push(
          `the player ended with ${result.signal ?? `exit code ${result.code}`}`,
        );
      if (!seen.normalExit) problems.push("no `exit: normal`");
      if (seen.settled === null) problems.push("start-up never settled");
      else {
        const idle = result.seconds - seen.settled;
        if (idle < step.idleAfterSettled * slow)
          problems.push(
            `start-up settled at ${seen.settled.toFixed(1)} s, ${idle.toFixed(1)} s before the end (needs ${step.idleAfterSettled * slow} s): use --slow`,
          );
      }
      if (step.wakeAfterSettled) {
        if (seen.wake === null) problems.push("VESTRA_TEST_WAKE never fired");
        else if (seen.settled !== null && seen.wake < seen.settled)
          problems.push("the wake came before start-up settled: use --slow");
      }
      if (step.media && seen.opened === null)
        problems.push("the file never opened (no `handed over at`)");
      info.hostLine ??= /^start: .*$/m.exec(result.stderr)?.[0] ?? null;
      info.steps[`${name}/${step.name}`] = {
        seconds: Number(result.seconds.toFixed(1)),
        settledAt: seen.settled,
        wakeAt: seen.wake,
        openedAt: seen.opened,
        problems,
      };
      console.log(
        problems.length
          ? `INVALID: ${problems.join("; ")}`
          : `ok (settled at ${seen.settled.toFixed(1)} s)`,
      );
      for (const problem of problems)
        invalid.push(`${name}/${step.name}: ${problem}`);
    }
  }
  writeFileSync(join(out, "run.json"), JSON.stringify(info, null, 2) + "\n");
  for (const warning of normaliseRun(out).warnings)
    console.log(`  warning: ${warning}`);
  console.log(
    `  written: ${join(out, "normalised.json")}, summary.txt, run.json, raw/`,
  );
  if (invalid.length)
    throw new RunError(
      `invalid run, nothing to compare:\n  ${invalid.join("\n  ")}`,
    );
  return out;
}

/**
 * A run's raw output as compared: reads <out>/run.json and <out>/raw/, writes
 * <out>/normalised.json and summary.txt. `diff` does it again for both sides,
 * so two runs are always compared by the rules of this script, also a
 * baseline kept from an earlier version of it.
 */
function normaliseRun(out) {
  const info = readJson(join(out, "run.json"));
  const normalised = { format: 1, scenarios: {} };
  const warnings = [];
  const plugins = [...info.builtins, ...info.storePlugins];
  const text = (path) => (existsSync(path) ? readFileSync(path, "utf8") : "");
  for (const key of Object.keys(info.steps)) {
    const [name, stepName] = key.split("/");
    const directory = join(out, "raw", name);
    const step = STEPS[name]?.find((entry) => entry.name === stepName);
    if (!step) throw new RunError(`${out}: unknown step ${key}`);
    const file = (suffix) => join(directory, `${stepName}.${suffix}`);
    const stderr = text(file("stderr.txt"));
    const recordings = {};
    if (existsSync(file("recordings")))
      for (const entry of readdirSync(file("recordings")))
        if (entry.endsWith(".json"))
          recordings[basename(entry, ".json")] = readJson(
            join(file("recordings"), entry),
          );
    const catalogFile = [
      file("catalog.json"),
      join(directory, "profile", "store", "catalog.json"),
    ].find((path) => existsSync(path));
    const catalog = catalogFile ? readJson(catalogFile) : {};
    const start = /^start: Vestra Player (\S+) \(host (\S+), (\S+)\)/m.exec(
      stderr,
    );
    const digests = Object.fromEntries(
      info.builtins.map((plugin) => [plugin.digest, plugin.id]),
    );
    for (const [id, entry] of Object.entries(catalog.installed ?? {}))
      if (entry?.current?.digest) digests[entry.current.digest] = id;
    const compared = normaliseStep(
      {
        stderr,
        registry: text(file("registry.jsonl")),
        pluginsLog: text(file("plugins.log")),
        recordings,
        catalog,
      },
      {
        compiles: step.compiles,
        roots: [
          [info.profiles?.[name] ?? join(directory, "profile"), "<profile>"],
          [info.stage, "<stage>"],
          [info.storeRoot, "<store-plugins>"],
          [info.media, "<media>"],
        ],
        versions: [
          info.release,
          info.sdk,
          start?.[1],
          start?.[2],
          ...plugins.map((plugin) => plugin.version),
        ],
        digests,
        build: start?.[3] ?? null,
        ids: plugins.map((plugin) => plugin.id),
      },
    );
    (normalised.scenarios[name] ??= {})[stepName] = compared;
    const failed = Object.entries(standbySummary(compared))
      .filter(([, row]) => row.failed)
      .map(([id]) => id);
    if (failed.length)
      warnings.push(
        `${key}: activation failed for ${failed.join(", ")} (compared as failures)`,
      );
  }
  writeFileSync(
    join(out, "normalised.json"),
    JSON.stringify(normalised, null, 1) + "\n",
  );
  writeFileSync(
    join(out, "summary.txt"),
    [summaryText(normalised, info), "", ...warnings].join("\n") + "\n",
  );
  return { normalised, warnings };
}

/** The standby table of a run (not compared; for people). */
export function summaryText(normalised, info = null) {
  const out = [];
  if (info)
    out.push(
      `${info.hostLine ?? info.stage}; SDK ${info.sdk}; ${info.builtins.length} built-ins, ${info.storePlugins.length} store plugins`,
    );
  for (const [scenario, steps] of Object.entries(normalised.scenarios))
    for (const [name, step] of Object.entries(steps)) {
      out.push("", `${scenario}/${name}`);
      const rows = standbySummary(step);
      const width = Math.max(...Object.keys(rows).map((id) => id.length));
      for (const [id, row] of Object.entries(rows))
        out.push(
          `  ${id.padEnd(width)}  ${row.class.padEnd(19)} parked ${row.parked}  woken ${row.woken}  recording ${row.recording.padEnd(18)} ${row.failed ? "ACTIVATION FAILED  " : ""}${row.moments}`,
        );
    }
  return out.join("\n");
}

/** `diff`: two runs' normalised.json; returns the difference lines. */
function diff(a, b, report) {
  const read = (directory) => {
    const out = resolve(directory);
    if (existsSync(join(out, "run.json")) && existsSync(join(out, "raw"))) {
      const invalid = Object.entries(readJson(join(out, "run.json")).steps)
        .filter(([, step]) => step.problems?.length)
        .map(([key, step]) => `${key}: ${step.problems.join("; ")}`);
      if (invalid.length)
        throw new RunError(
          `${out} is an invalid run, nothing to compare:\n  ${invalid.join("\n  ")}`,
        );
      return normaliseRun(out).normalised;
    }
    const file = join(out, "normalised.json");
    if (!existsSync(file)) throw new RunError(`${file} does not exist`);
    return readJson(file);
  };
  const [left, right] = [read(a), read(b)];
  const { compiles, rest: lines } = compilePattern(
    differences(left.scenarios, right.scenarios),
  );
  const count = (doc) =>
    Object.values(doc.scenarios).reduce(
      (sum, steps) =>
        sum +
        Object.values(steps).reduce(
          (n, step) => n + Object.keys(step.plugins).length,
          0,
        ),
      0,
    );
  const text = [
    `V2 runtime differential`,
    `  A  ${resolve(a)}`,
    `  B  ${resolve(b)}`,
    `  compared: ${Object.keys(left.scenarios).join(", ")}; ${count(left)} plugin runs in A, ${count(right)} in B`,
    ...(lines.length
      ? [`RESULT: FAIL, ${lines.length} difference(s)`, ...lines]
      : ["RESULT: PASS, no difference"]),
    ...(compiles.length
      ? [
          "",
          `COMPILE PATTERN, not counted in the result (per-panel compilation, SDK 1.9.0 C1): ${compiles.length} difference(s)`,
          ...compiles,
        ]
      : []),
  ].join("\n");
  console.log(text);
  if (report) writeFileSync(resolve(report), text + "\n");
  return lines;
}

function parse(argv, flags) {
  const options = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const name = argv[i];
    if (!name.startsWith("--")) {
      options._.push(name);
      continue;
    }
    const key = name.slice(2).replace(/-(\w)/g, (_, c) => c.toUpperCase());
    if (flags.includes(name)) options[key] = true;
    else if (i + 1 < argv.length) options[key] = argv[++i];
    else throw new RunError(`${name} needs a value`);
  }
  return options;
}

async function main(argv) {
  const command = argv[0];
  const options = parse(argv.slice(1), ["--no-store-plugins", "--wire-trace"]);
  if (process.platform === "win32" && options.quietFlag === undefined)
    options.quietFlag = "C:\\vestra\\quiet.flag";
  if (command === "run" && options.stage && options.out) {
    await run(options);
    return 0;
  }
  if (command === "diff" && options._.length === 2)
    return diff(options._[0], options._[1], options.report).length ? 1 : 0;
  if (command === "compare" && options.a && options.b && options.out) {
    const out = resolve(options.out);
    await run({ ...options, stage: options.a, out: join(out, "a") });
    await run({ ...options, stage: options.b, out: join(out, "b") });
    return diff(join(out, "a"), join(out, "b"), join(out, "diff.txt")).length
      ? 1
      : 0;
  }
  throw new RunError(
    "usage: verify-runtime.mjs run --stage <staged player> --out <directory> | diff <A> <B> [--report <file>] | compare --a <stage> --b <stage> --out <directory> (see the header)",
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof RunError)) throw error;
    console.error(`verify-runtime: ${error.message}`);
    process.exitCode = 2;
  }
}
