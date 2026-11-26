// The runtime differential's normaliser and comparison
// (scripts/verify-runtime.mjs): two runs
// of one behaviour compare equal whatever their owner numbers, times, paths,
// versions and GUIDs; a changed registration, classification or recording
// is named. The player runs themselves are not started here.
import assert from "node:assert/strict";
import { test } from "node:test";

// @ts-expect-error: a JavaScript build script without declarations.
import * as tool from "../../scripts/verify-runtime.mjs";

const {
  compilePattern,
  differences,
  normaliseStep,
  normaliser,
  registryAt,
  standbySummary,
  summaryText,
} = tool;

const IDS = [
  "vestra.subtitles",
  "vestra.subtitle-online",
  "vestra.store",
  "zysilm.autoframe",
];

type Build = {
  profile: string;
  stage: string;
  version: string;
  build: string;
  guid: string;
  digest: string;
  owner: number;
};
const A: Build = {
  profile: "C:\\out\\a\\raw\\idle\\profile",
  stage: "C:\\stages\\main\\VestraPlayer-0.8.3-windows-x64",
  version: "0.8.3",
  build: "f54a87d3",
  guid: "0e0ed22d-9737-4a32-9263-5182c67f4495",
  digest: "07102615d620097777482a1fc33aed7371e09f6e6ae3e55baf7aa43f712eafc3",
  owner: 4,
};
const B: Build = {
  profile: "D:\\work\\b\\raw\\idle\\profile",
  stage: "D:\\stages\\batch\\VestraPlayer-0.8.4-windows-x64",
  version: "0.8.4",
  build: "0123abcd",
  guid: "9af817d9-491c-4c4d-87b1-f292814ef066",
  digest: "a0aad14c00000000000000000000000000000000000000000000000000000000",
  owner: 17,
};

const context = (build: Build) => ({
  roots: [
    [build.profile, "<profile>"],
    [build.stage, "<stage>"],
    ["D:\\work\\plugins-repo", "<store-plugins>"],
    [null, "<media>"],
  ],
  versions: [build.version, "1.7.20", build === A ? "0.2.17" : "0.2.18"],
  digests: { [build.digest]: "vestra.store" },
  build: build.build,
  ids: IDS,
});

/** One step's raw output of a build; `edit` changes the behaviour. */
function raw(
  build: Build,
  edit: { title?: string; lazy?: boolean; calls?: number } = {},
) {
  const storeVersion = build === A ? "0.2.17" : "0.2.18";
  const registry = (moment: string) =>
    JSON.stringify({
      id: "vestra.store",
      moment,
      registry: {
        commands: [
          {
            id: "open",
            recorded: moment === "parked",
            title: edit.title ?? "Plugin Store…",
            button: {
              icon: {
                image: `${build.profile}\\data\\${build.guid}\\icons\\a.png`,
              },
            },
            state: moment === "ready" ? null : { enabled: true },
          },
        ],
        bindings: [],
        pages: [
          {
            id: "store",
            recorded: moment === "parked",
            file: `${build.stage}\\resources\\plugins\\vestra.store\\ui\\page.slint`,
            properties:
              moment === "ready" ? {} : { version: `v${storeVersion}` },
          },
        ],
        drops: [],
        wakes: ["packages.changed"],
      },
    });
  const lazy = edit.lazy ?? true;
  return {
    stderr: [
      `start: Vestra Player ${build.version} (host ${build.version}, ${build.build}) on windows x86_64`,
      "supervisor: idle after 30000 ms, 4 starts at once (16 cores), 16 third-party processes (28381 MiB)",
      `memory: private ${build.owner} MB`,
      `shared runner 1: vestra.store admitted`,
      "lazy: vestra.store starts for its first run",
      `runtime ${build.owner} runtime.ready: null`,
      `lazy: owner ${build.owner} ready after ${build.owner * 31} ms, 0 trigger(s)`,
      `shell: compile with 2 panel(s) [${build === A ? "vestra.b:bottom, vestra.a:sidebar" : "vestra.a:sidebar, vestra.b:bottom"}] in ${build.owner}1.5 ms at +${build.owner}00 ms (ready)`,
      `plugin zysilm.autoframe: keeps running while idle: an event listener; listens to source.state: a listener added at main.js:${build.owner}19 | a listener added at main.js:583:${build.owner}`,
      "lazy: vestra.subtitle-online classified resident: an event listener",
      "lazy: vestra.subtitles classified resident: an event listener; listens to playback.backend",
      lazy
        ? "lazy: vestra.store classified lazy"
        : "lazy: vestra.store classified resident: an event listener; listens to source.state",
      "lazy: start-up settled",
      'supervisor: OSD "Starting Plugin Store…"',
      ...(lazy
        ? [`lazy: vestra.store parked (${edit.calls ?? 4} recorded calls)`]
        : []),
      `lazy: vestra.store woken by packages.changed (owner ${build.owner} → ${build.owner + 20})`,
      `supervisor: 16 plugin processes stopped in ${build.owner * 9} ms`,
      "exit: normal",
    ].join("\r\n"),
    registry:
      [registry("ready"), registry("parked"), registry("quit")].join("\n") +
      "\n",
    pluginsLog: [
      `2026-11-13T05:00:14.935Z [${build.owner}00] [vestra.store] index: 3 plugins, checked in ${build.owner} ms`,
      `2026-11-13T05:00:14.935Z [${build.owner}00] [vestra.store] HostError: plugin is stopping`,
      `2026-11-13T05:00:14.935Z [${build.owner}00]     at Object.request (file:///${build.stage.replaceAll("\\", "/")}/resources/runtime/boot.js:153:29)`,
    ].join("\n"),
    recordings: {
      [build.guid]: {
        schema: 1,
        player: build.version,
        host: build.version,
        build: build.build,
        digest: build.digest,
        version: storeVersion,
        residentAtReady: !lazy,
        recordedAt: 1794545620477 + build.owner,
        calls: Array.from({ length: edit.calls ?? 4 }, (_, index) => ({
          method: "commands.register",
          params: {
            id: `c${index}`,
            icon: `${build.profile.replaceAll("\\", "/")}/data/${build.guid}/i.png`,
          },
        })),
      },
    },
    catalog: {
      installed: { "vestra.store": { instance: { guid: build.guid } } },
    },
  };
}

test("strings lose paths, GUIDs, digests, the build and versions", () => {
  const n = normaliser({ ...context(A), guids: { [A.guid]: "vestra.store" } });
  assert.equal(
    n.text(
      `${A.stage}\\resources\\x.slint and ${A.stage.replaceAll("\\", "/")}/y`,
    ),
    "<stage>\\resources\\x.slint and <stage>/y",
  );
  // The path as it is written inside JSON text, and in another case.
  assert.equal(
    n.text(JSON.stringify(`${A.profile.toLowerCase()}\\data`)),
    '"<profile>\\\\data"',
  );
  assert.equal(n.text(`data/${A.guid}/x`), "data/<guid:vestra.store>/x");
  assert.equal(n.text("11111111-2222-4333-8444-555555555555"), "<guid>");
  assert.equal(n.text(A.digest), "<digest:vestra.store>");
  assert.equal(n.text("build f54a87d3."), "build <build>.");
  assert.equal(
    n.text("Vestra Player 0.8.3, SDK v1.7.20"),
    "Vestra Player <version>, SDK <version>",
  );
  // A longer number that only contains a version is left alone.
  assert.equal(
    n.text("10.8.3 and 0.8.30 and 0.8.3.1"),
    "10.8.3 and 0.8.30 and 0.8.3.1",
  );
});

test("log lines also lose owner numbers, times and source lines", () => {
  const n = normaliser(context(A));
  assert.equal(
    n.line("lazy: vestra.store woken by packages.changed (owner 4 → 24)"),
    "lazy: vestra.store woken by packages.changed (owner N → N)",
  );
  assert.equal(
    n.line("open: 1 file(s) handed over at +2685 ms"),
    "open: 1 file(s) handed over at N ms",
  );
  assert.equal(
    n.line("a listener added at main.js:58 | bind (list.ts:47:3)"),
    "a listener added at main.js:N | bind (list.ts:N)",
  );
  assert.equal(
    n.line("lazy: vestra.store parked (4 recorded calls)"),
    "lazy: vestra.store parked (4 recorded calls)",
  );
  // The longest id wins; an id inside a longer name is not one.
  assert.equal(
    n.pluginOf("lazy: vestra.subtitle-online classified lazy"),
    "vestra.subtitle-online",
  );
  assert.equal(n.pluginOf("lazy: vestra.subtitles parked"), "vestra.subtitles");
  assert.equal(
    n.pluginOf("plugin zysilm.autoframe: keeps running"),
    "zysilm.autoframe",
  );
  assert.equal(n.pluginOf("lazy: start-up settled"), null);
  assert.equal(n.pluginOf("x.vestra.store-y"), null);
});

test("a command's state and a page's values are compared after the ready only", () => {
  const registry = {
    commands: [{ id: "a", state: { checked: true }, title: "A" }],
    pages: [{ id: "p", properties: { x: 1 } }],
    wakes: [],
  };
  for (const moment of ["ready", "parking", "woken"])
    assert.deepEqual(registryAt(moment, registry), {
      commands: [{ id: "a", state: null, title: "A" }],
      pages: [{ id: "p", properties: null }],
      wakes: [],
    });
  for (const moment of ["parked", "replayed", "quit"])
    assert.equal(registryAt(moment, registry), registry);
});

test("two builds with the same behaviour compare equal", () => {
  const [a, b] = [
    normaliseStep(raw(A), context(A)),
    normaliseStep(raw(B), context(B)),
  ];
  assert.deepEqual(differences(a, b), []);
  const store = a.plugins["vestra.store"];
  assert.deepEqual(
    store.registry.map((entry: { moment: string }) => entry.moment),
    ["ready", "parked", "quit"],
  );
  assert.deepEqual(store.log, [
    "shared runner N: vestra.store admitted",
    "lazy: vestra.store starts for its first run",
    "lazy: vestra.store classified lazy",
    "lazy: vestra.store parked (4 recorded calls)",
    "lazy: vestra.store woken by packages.changed (owner N → N)",
  ]);
  // A request on its way when the plugin stopped is not compared.
  assert.deepEqual(store.console, ["index: 3 plugins, checked in N ms"]);
  assert.deepEqual(Object.keys(store.recording), [
    "schema",
    "residentAtReady",
    "calls",
  ]);
  assert.equal(
    store.recording.calls[0].params.icon,
    "<profile>/data/<guid:vestra.store>/i.png",
  );
  assert.equal(
    store.registry[1].registry.pages[0].properties.version,
    "<version>",
  );
  // Machine facts, timings and start notices are not compared; lines that
  // name no plugin are sorted, a compile's panels too.
  assert.deepEqual(a.host.log, [
    "exit: normal",
    "lazy: start-up settled",
    "shell: compile with 2 panel(s) [vestra.a:sidebar, vestra.b:bottom] in N ms at N ms (ready)",
    "supervisor: 16 plugin processes stopped in N ms",
  ]);
  // Every known plugin has an entry, also one that said nothing.
  assert.deepEqual(Object.keys(a.plugins), [...IDS].sort());
});

test("a step may leave the shell's compiles out, or keep the first and last", () => {
  const step = normaliseStep(raw(A), { ...context(A), compiles: false });
  assert.deepEqual(step.host.log, [
    "exit: normal",
    "lazy: start-up settled",
    "supervisor: 16 plugin processes stopped in N ms",
  ]);
  // Third-party first runs reach the shell in one compile or in two.
  const compile = (panels: string) =>
    `shell: compile with ${panels.split(", ").length} panel(s) [${panels}] in 1.5 ms at +9 ms (ready)`;
  const ends = (lines: string[]) =>
    normaliseStep(
      { ...raw(A), stderr: lines.join("\n") },
      { ...context(A), compiles: "ends" },
    ).host.log;
  const two = ends([
    compile("b:bottom, a:sidebar"),
    compile("a:sidebar, b:bottom, c:sidebar"),
  ]);
  const three = ends([
    compile("a:sidebar, b:bottom"),
    compile("a:sidebar, b:bottom, d:sidebar"),
    compile("a:sidebar, b:bottom, c:sidebar"),
  ]);
  assert.deepEqual(two, [
    "shell: first compile with 2 panel(s) [a:sidebar, b:bottom] in N ms at N ms (ready)",
    "shell: last compile with 3 panel(s) [a:sidebar, b:bottom, c:sidebar] in N ms at N ms (ready)",
  ]);
  assert.deepEqual(three, two);
  assert.deepEqual(ends([compile("a:sidebar")]), [
    "shell: first compile with 1 panel(s) [a:sidebar] in N ms at N ms (ready)",
  ]);
});

test("per-panel compiles are a section of their own, reported apart from the result", () => {
  // Player 0.9.0 (SDK 1.9.0 C1): each panel's file compiles on its own;
  // they arrive in any order and are compared sorted, without times.
  const panel = (what: string, ms: number) =>
    `panel: compile [${what}] in ${ms}.5 ms at +${ms}00 ms (ready)`;
  const withPanels = (build: Build, lines: string[]) => {
    const base = raw(build);
    return normaliseStep(
      {
        ...base,
        stderr: [...base.stderr.split("\r\n"), ...lines]
          .filter((line) => !line.startsWith("shell: compile "))
          .join("\r\n"),
      },
      context(build),
    );
  };
  const b = withPanels(B, [
    panel("vestra.video:adjust", 222),
    panel("vestra.b:bottom", 67),
    panel("vestra.a:sidebar", 65),
  ]);
  assert.deepEqual(b.panels.compiles, [
    "panel: compile [vestra.a:sidebar] in N ms at N ms (ready)",
    "panel: compile [vestra.b:bottom] in N ms at N ms (ready)",
    "panel: compile [vestra.video:adjust] in N ms at N ms (ready)",
  ]);
  // Not among the host's or a plugin's lines.
  assert.ok(!b.host.log.some((line: string) => line.includes("compile")));
  for (const step of Object.values(b.plugins) as { log: string[] }[])
    assert.ok(!step.log.some((line) => line.includes("compile")));
  // Same panels in another order and with other times: equal.
  const again = withPanels(A, [
    panel("vestra.a:sidebar", 90),
    panel("vestra.video:adjust", 300),
    panel("vestra.b:bottom", 80),
  ]);
  assert.deepEqual(differences(again, b), []);
  // A player before it (one shell compile) against per-panel compiles: the
  // compile pattern, apart from the result; nothing else differs.
  const lines = differences(
    { file: { open: normaliseStep(raw(A), context(A)) } },
    { file: { open: b } },
  );
  const { compiles, rest } = compilePattern(lines);
  assert.deepEqual(rest, []);
  assert.deepEqual(compiles, [
    "file › open › host › log: only in A: shell: compile with 2 panel(s) [vestra.a:sidebar, vestra.b:bottom] in N ms at N ms (ready)",
    "file › open › panels › compiles: only in B: panel: compile [vestra.a:sidebar] in N ms at N ms (ready)",
    "file › open › panels › compiles: only in B: panel: compile [vestra.b:bottom] in N ms at N ms (ready)",
    "file › open › panels › compiles: only in B: panel: compile [vestra.video:adjust] in N ms at N ms (ready)",
  ]);
  // A baseline normalised before the section existed: still the pattern.
  const { panels: _panels, ...old } = normaliseStep(raw(A), context(A));
  const fromOld = compilePattern(
    differences({ file: { open: old } }, { file: { open: b } }),
  );
  assert.equal(fromOld.compiles.length, 2);
  assert.deepEqual(fromOld.rest, []);
  // Any other difference still counts.
  const title = compilePattern(
    differences(
      { idle: { first: b } },
      { idle: { first: withPanels(B, []) } },
    ).concat(["idle › first › plugins › vestra.store › log: only in B: x"]),
  );
  assert.equal(title.compiles.length, 3);
  assert.deepEqual(title.rest, [
    "idle › first › plugins › vestra.store › log: only in B: x",
  ]);
});

test("a changed title, classification or recording is named", () => {
  const a = normaliseStep(raw(A), context(A));
  const title = differences(
    a,
    normaliseStep(raw(B, { title: "Store…" }), context(B)),
  );
  assert.deepEqual(title, [
    "plugins › vestra.store › registry › 1:ready › registry › commands › open › title: A Plugin Store… | B Store…",
    "plugins › vestra.store › registry › 2:parked › registry › commands › open › title: A Plugin Store… | B Store…",
    "plugins › vestra.store › registry › 3:quit › registry › commands › open › title: A Plugin Store… | B Store…",
  ]);
  const resident = differences(
    a,
    normaliseStep(raw(B, { lazy: false }), context(B)),
  );
  assert.ok(
    resident.includes(
      "plugins › vestra.store › log: only in A: lazy: vestra.store classified lazy",
    ),
    resident.join("\n"),
  );
  assert.ok(
    resident.some((line: string) =>
      line.startsWith(
        "plugins › vestra.store › log: only in B: lazy: vestra.store classified resident: an event listener",
      ),
    ),
  );
  assert.ok(
    resident.includes(
      "plugins › vestra.store › recording › residentAtReady: A false | B true",
    ),
  );
  const calls = differences(a, normaliseStep(raw(B, { calls: 5 }), context(B)));
  assert.ok(
    calls.some((line: string) =>
      /log: only in A: lazy: vestra\.store parked \(4 recorded calls\)/.test(
        line,
      ),
    ),
  );
  assert.ok(
    calls.some((line: string) =>
      /recording › calls: A has 4 entries; B has 5 entries/.test(line),
    ),
  );
});

test("lists: lines by content, entries by id, snapshots by position", () => {
  assert.deepEqual(differences(["a", "b", "b"], ["b", "a", "b"], "log"), [
    "log: same lines in another order, from line 1: A a | B b",
  ]);
  assert.deepEqual(differences(["a", "b", "b"], ["a", "b", "c"], "log"), [
    "log: only in A: b",
    "log: only in B: c",
  ]);
  const entries = (ids: string[]) => ids.map((id) => ({ id, title: id }));
  assert.deepEqual(
    differences(entries(["a", "b", "c"]), entries(["b", "a", "d"]), "commands"),
    [
      "commands › c: only in A",
      "commands › d: only in B",
      "commands: order differs from entry 1: A a | B b",
    ],
  );
  const moments = (names: string[]) =>
    names.map((moment) => ({ moment, registry: {} }));
  assert.deepEqual(
    differences(
      moments(["ready", "parking", "parked", "quit"]),
      moments(["ready", "quit"]),
      "registry",
    ),
    ["registry: A has ready → parking → parked → quit; B has ready → quit"],
  );
  assert.deepEqual(differences({ a: 1, b: null }, { a: 1, c: 2 }, "x"), [
    "x › b: only in A: null",
    "x › c: only in B: 2",
  ]);
});

test("the summary table says each plugin's class, parks and recording", () => {
  const step = normaliseStep(raw(A), context(A));
  const rows = standbySummary(step);
  assert.deepEqual(rows["vestra.store"], {
    class: "lazy",
    parked: 1,
    woken: 1,
    failed: false,
    moments: "ready,parked,quit",
    recording: "lazy, 4 calls",
  });
  assert.equal(rows["vestra.subtitles"].class, "resident");
  assert.equal(rows["zysilm.autoframe"].class, "keeps running");
  const text = summaryText({ scenarios: { idle: { first: step } } });
  assert.match(
    text,
    /idle\/first\n\s+vestra\.store\s+lazy\s+parked 1\s+woken 1\s+recording lazy, 4 calls/,
  );
});
