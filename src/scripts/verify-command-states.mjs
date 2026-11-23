#!/usr/bin/env node
/**
 * Differential check of the built-in plugins' command states: runs the
 * plugins of two revisions side by side in TestHosts over a matrix of player
 * states and compares, for every command, what the player would show
 * (enabled, checked, busy, the reason it shows, by the host's rule as
 * TestHost applies it) and what a key press would do (run, show the reason,
 * pass on). A refactor of how commands keep their state (functions to
 * `plugin.when` conditions, say) must change none of it.
 *
 *   node scripts/verify-command-states.mjs --old <rev> [--new <rev>] [--plugins audio,playback,subtitles] [--only states|busy] [--tracks none,one,...]
 *
 * `--new` defaults to the working tree. A revision's plugin folders are
 * extracted with `git archive` into src/.verify-command-states/ (removed at
 * the end), so they resolve `@vestra-player/sdk` from src/node_modules like
 * the working tree's. The states: the source's phase (empty, loading, ready,
 * ended), seeking on and off, a position or none, the audio tracks and the
 * subtitle tracks (none, one, several, more than the menus list, an external
 * one, none selected), a mono or stereo track, the rate and the A-B loop;
 * each once set before the plugin starts and once reached after it started.
 * Then each plugin's busy paths (a request held while the plugin works) and
 * changes the matrix does not reach (a track added beyond the menus' eight).
 * Exit 1 on any difference, listed by state, command and field.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const src = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const oldRev = option("old");
const newRev = option("new", null);
const plugins = option("plugins", "audio,playback,subtitles").split(",");
/** `states` or `busy`: only that part (both by default). */
const only = option("only", null);
/** Only these track variants (comma-separated), for a quick look at one. */
const trackFilter = option("tracks", null)?.split(",");
if (!oldRev) {
  console.error(
    "usage: verify-command-states.mjs --old <rev> [--new <rev>] [--plugins a,b]",
  );
  process.exit(2);
}

// One folder per run: two runs side by side must not remove each other's copies.
const scratch = join(src, ".verify-command-states", String(process.pid));
function extract(rev, plugin) {
  const dir = join(scratch, rev.replace(/[^\w.-]/g, "_"), plugin);
  if (existsSync(join(dir, "main.ts"))) return dir;
  mkdirSync(dir, { recursive: true });
  const tar = execFileSync("git", ["archive", rev, `src/plugins/${plugin}`], {
    cwd: resolve(src, ".."),
    maxBuffer: 1 << 28,
  });
  // In the folder itself: a Windows path in -C reads as a remote host to GNU tar.
  execFileSync("tar", ["-x", "--strip-components=3"], { input: tar, cwd: dir });
  return dir;
}
const dirOf = (rev, plugin) =>
  rev ? extract(rev, plugin) : join(src, "plugins", plugin);

const { startPlugin, TestHost } = await import(
  pathToFileURL(
    join(src, "node_modules/@vestra-player/sdk/dist/plugin/testing.js"),
  ).href
);
const activations = new Map();
async function activation(dir) {
  if (!activations.has(dir))
    activations.set(
      dir,
      (await import(pathToFileURL(join(dir, "main.ts")).href)).default,
    );
  return activations.get(dir);
}

// ---------------------------------------------------------------- the states
const audioTrack = (trackId, selected, external = false) => ({
  trackId,
  streamIndex: trackId,
  selected,
  external,
  codec: "aac",
  title: null,
  language: null,
});
const embedded = (id) => ({ kind: "embedded", id });
const subTrack = (target, extra = {}) => ({
  target,
  title: `S${target.id ?? target.resourceId}`,
  language: "en",
  styleSupported: true,
  styleReason: null,
  codec: "subrip",
  text: true,
  streamIndex: target.kind === "embedded" ? target.id : null,
  ...extra,
});
const external = {
  kind: "external",
  owner: 1,
  resourceId: "r1",
  contentRevision: 1,
};
/** Track variants, the audio and the subtitle lists side by side. */
const TRACKS = {
  none: { audio: [], subs: [], selected: null },
  one: {
    audio: [audioTrack(1, true)],
    subs: [subTrack(embedded(1))],
    selected: embedded(1),
  },
  several: {
    audio: [audioTrack(1, false), audioTrack(2, true), audioTrack(3, false)],
    subs: [1, 2, 3].map((id) => subTrack(embedded(id))),
    selected: embedded(2),
  },
  many: {
    audio: Array.from({ length: 10 }, (_, i) => audioTrack(i + 1, i === 8)),
    subs: Array.from({ length: 10 }, (_, i) => subTrack(embedded(i + 1))),
    selected: embedded(9),
  },
  external: {
    audio: [audioTrack(1, false), audioTrack(2, true, true)],
    subs: [subTrack(embedded(1)), subTrack(external)],
    selected: external,
  },
  eight: {
    audio: Array.from({ length: 8 }, (_, i) => audioTrack(i + 1, i === 0)),
    subs: Array.from({ length: 8 }, (_, i) => subTrack(embedded(i + 1))),
    selected: null,
  },
  unselected: {
    audio: [audioTrack(1, false), audioTrack(2, false)],
    subs: [subTrack(embedded(1)), subTrack(embedded(2))],
    selected: null,
  },
};
const PHASES = ["empty", "loading", "ready", "ended"];
const STATES = [];
for (const phase of PHASES)
  for (const seek of [true, false])
    for (const position of [12, null])
      for (const tracks of Object.keys(TRACKS).filter(
        (t) => !trackFilter || trackFilter.includes(t),
      ))
        for (const channels of [2, 1])
          for (const [rate, loop] of [
            [1, null],
            [1.5, { a: 5, b: 20, count: null }],
          ])
            STATES.push({
              phase,
              seek,
              position,
              tracks,
              channels,
              rate,
              loop,
            });
const label = (s) =>
  `${s.phase} seek=${s.seek} pos=${s.position} tracks=${s.tracks} ch=${s.channels} rate=${s.rate} loop=${s.loop ? "on" : "off"}`;

function prepare(host, s) {
  host.stats = {
    ...host.stats,
    audio: {
      codec: "aac",
      sampleRate: 48000,
      channels: s.channels,
      decoder: "aac",
      format: "floatp",
      channelLayout: s.channels === 1 ? "mono" : "stereo",
      bitrate: null,
    },
    video: { codec: "h264", width: 1920, height: 1080 },
  };
  const t = TRACKS[s.tracks];
  host.subtitles.tracks.splice(
    0,
    host.subtitles.tracks.length,
    ...structuredClone(t.subs),
  );
  host.subtitles.selection = {
    stamp: { generation: 1, revision: 1 },
    target: t.selected && structuredClone(t.selected),
  };
}
function source(host, s) {
  if (s.phase === "empty") return;
  host.openElsewhere("/movie.mp4");
  const t = TRACKS[s.tracks];
  const capabilities = { ...host.source.capabilities, seek: s.seek };
  host.setSource({
    phase: s.phase === "loading" ? "loading" : "ready",
    duration: 600,
    position: s.position,
    capabilities,
    audioTracks: structuredClone(t.audio),
    rate: s.rate,
    loop: s.loop,
  });
  if (s.phase === "ended")
    host.setSource({ phase: "ended", terminalReason: "natural" });
}

// ---------------------------------------------------------------- one run
const ticks = async (n = 15) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 1));
};
function snapshot(host) {
  const out = {};
  for (const [id, entry] of host.commands) {
    const st = host.command(id).state;
    const shown =
      typeof entry.display?.reason === "string" ? entry.display.reason : null;
    const reason =
      shown ??
      (typeof st.reason === "string"
        ? st.reason
        : st.reason
          ? JSON.stringify(st.reason)
          : null);
    const press = st.enabled
      ? st.busy
        ? "busy"
        : "ran"
      : reason
        ? "reason"
        : "passed";
    out[id] = {
      enabled: st.enabled,
      checked: st.checked ?? null,
      busy: !!st.busy,
      reason,
      press,
    };
  }
  return out;
}
async function run(dir, s, mode, scenario) {
  const host = new TestHost();
  if (mode === "start") {
    prepare(host, s);
    source(host, s);
  }
  const started = await startPlugin(await activation(dir), {
    directory: dir,
    host,
  });
  try {
    await host.settle();
    if (mode === "after") {
      prepare(host, s);
      source(host, s);
      host.setSource({});
      await host.settle();
    }
    await ticks();
    if (scenario) {
      await scenario(host);
      await ticks(30);
    }
    return snapshot(host);
  } finally {
    for (const release of releases.get(host) ?? []) release();
    releases.delete(host);
    await ticks();
    await started.stop().catch(() => {});
  }
}

// ---------------------------------------------------------------- busy paths
/** Requests held until the run ends (released before the stop, which waits for them). */
const releases = new Map();
const held = (host, method) =>
  host.handle(
    method,
    () =>
      new Promise((resolve) => {
        if (!releases.has(host)) releases.set(host, []);
        releases.get(host).push(() => {
          host.reset(method);
          resolve(null);
        });
      }),
  );
const READY = {
  phase: "ready",
  seek: true,
  position: 12,
  tracks: "one",
  channels: 2,
  rate: 1,
  loop: null,
};
const ENDED = { ...READY, phase: "ended" };
const BUSY = {
  audio: {
    "an edit applying": [
      READY,
      async (host) => {
        await host.command("audio.workbench").run();
        held(host, "processing.replace");
        host.mounted().emit("delay-changed", 100);
      },
    ],
    "Load External Audio… choosing": [
      READY,
      async (host) => {
        held(host, "dialogs.open");
        void host.command("audio.load").run();
      },
    ],
  },
  playback: {
    "a speed being set": [
      READY,
      async (host) => {
        held(host, "source.submit");
        void host.command("rate-15").run();
      },
    ],
    "Jump to Start after the end": [
      ENDED,
      async (host) => {
        held(host, "source.submit");
        void host.command("jump-start").run();
      },
    ],
    "a keyframe step (command)": [
      READY,
      async (host) => {
        held(host, "player.step");
        void host.command("keyframe-forward").run();
      },
    ],
    "a keyframe step (panel)": [
      READY,
      async (host) => {
        held(host, "player.step");
        host.mounted("adjust").emit("keyframeStep", 1);
      },
    ],
    "Open File… choosing": [
      READY,
      async (host) => {
        held(host, "dialogs.open");
        void host.command("open").run();
      },
    ],
  },
  subtitles: {
    "a ninth track (beyond the menu's eight) added and selected": [
      { ...READY, tracks: "eight" },
      async (host) => {
        host.subtitles.tracks.push(subTrack(embedded(9)));
        host.subtitles.selection.target = embedded(9);
        host.setSource({});
        await host.settle();
      },
    ],
    "a track being selected": [
      READY,
      async (host) => {
        held(host, "subtitles.select");
        void host.command("show").run();
      },
    ],
  },
};

// ---------------------------------------------------------------- compare
let compared = 0;
const differences = [];
function compare(plugin, where, a, b) {
  for (const id of new Set([...Object.keys(a), ...Object.keys(b)])) {
    compared++;
    if (!a[id] || !b[id]) {
      differences.push({
        plugin,
        where,
        id,
        field: "present",
        old: !!a[id],
        now: !!b[id],
      });
      continue;
    }
    for (const field of ["enabled", "checked", "busy", "reason", "press"])
      if (JSON.stringify(a[id][field]) !== JSON.stringify(b[id][field]))
        differences.push({
          plugin,
          where,
          id,
          field,
          old: a[id][field],
          now: b[id][field],
        });
  }
}

const started = Date.now();
let runs = 0;
try {
  for (const plugin of plugins) {
    const oldDir = dirOf(oldRev, plugin);
    const newDir = dirOf(newRev, plugin);
    for (const s of only === "busy" ? [] : STATES)
      for (const mode of ["start", "after"]) {
        compare(
          plugin,
          `[${mode}] ${label(s)}`,
          await run(oldDir, s, mode),
          await run(newDir, s, mode),
        );
        runs += 2;
      }
    for (const [name, [s, scenario]] of only === "states"
      ? []
      : Object.entries(BUSY[plugin] ?? {})) {
      compare(
        plugin,
        `[busy: ${name}]`,
        await run(oldDir, s, "start", scenario),
        await run(newDir, s, "start", scenario),
      );
      runs += 2;
    }
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
const busyCount = plugins.reduce(
  (n, p) => n + Object.keys(BUSY[p] ?? {}).length,
  0,
);
console.log(
  `verify-command-states: ${plugins.join(", ")}; ${STATES.length} states × 2 (before/after start) + ${busyCount} busy paths; ` +
    `${runs} plugin runs, ${compared} command states compared in ${Math.round((Date.now() - started) / 1000)} s`,
);
if (differences.length) {
  // Grouped by command, field and values: how many states show it, and the first one.
  const kinds = new Map();
  for (const d of differences) {
    const key = JSON.stringify([
      d.plugin,
      d.id.replace(/d+$/, "#"),
      d.field,
      d.old,
      d.now,
    ]);
    if (!kinds.has(key)) kinds.set(key, { d, count: 0 });
    kinds.get(key).count++;
  }
  console.log(`${differences.length} differences, ${kinds.size} kinds:`);
  for (const { d, count } of kinds.values())
    console.log(
      `  ${d.plugin} ${d.id}.${d.field}: old ${JSON.stringify(d.old)}, new ${JSON.stringify(d.now)} (${count}×; first ${d.where})`,
    );
  process.exit(1);
}
console.log("no differences");
process.exit(0);
