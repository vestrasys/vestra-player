#!/usr/bin/env node
/**
 * Offscreen screenshots of a plugin's windows and settings pages
 *:
 *
 *   npm run ui:shots -- <plugin...> [--lang en,zh-Hans,de] [--out <dir>]
 *       [--state <text>] [--max <n>] [--theme vestra|vestra-light]
 *       [--scale <factor>] [--states <dir>]
 *
 * 1. Runs the plugins' tests with VESTRA_UI_STATES (scripts/ui-record.mjs),
 *    which logs every window and settings page the tests open and every
 *    property value sent to them, in order. Failing tests do not stop it.
 * 2. Rebuilds each view's state when it was closed or its test ended, and
 *    keeps the distinct ones; a window file no test opened is shown with its
 *    default values.
 * 3. Renders them in each language with the host's own pipeline (the ignored
 *    host test `ui::ui_shots::tests::render_plugin_screenshots`, Slint's software
 *    renderer, no window on screen), one process per language.
 * 4. Writes `index.md` and `index.json` next to the PNG files: per image the
 *    plugin, the view, the language and the state (the test it came from).
 *
 * A plugin is a directory under plugins/, or a path with a slash relative to
 * src/. The output goes to <system temp>/vestra-ui-shots/<plugins> unless
 * --out names a directory (never into the repository). --states reuses a
 * log from an earlier run instead of running the tests. The host is built
 * into CARGO_TARGET_DIR, or <system temp>/vestra-target when it is not set.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, isAbsolute, join, relative, resolve, sep } from "node:path";
import { reportErrors, stop } from "../packaging/lib.mjs";

reportErrors();

const source = resolve(import.meta.dirname, "..");
const hostDir = join(source, "host");
const LOCALES = ["en", "zh-Hans", "ja", "ko", "de", "fr", "es"];

const usage = () => {
  console.error(
    "usage: npm run ui:shots -- <plugin...> [--lang en,zh-Hans,de] [--out <dir>] [--state <text>] [--max <n>] [--theme vestra|vestra-light] [--scale <factor>] [--states <dir>]",
  );
  stop(2);
};

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(name);
  if (index < 0) return fallback;
  const value = args[index + 1];
  if (value === undefined || value.startsWith("--")) usage();
  args.splice(index, 2);
  return value;
};
const langs = option("--lang", "en,zh-Hans,de").split(",").filter(Boolean);
const outOption = option("--out");
const stateFilter = option("--state");
const max = Number(option("--max", "0"));
const theme = option("--theme", "vestra");
const scale = Number(option("--scale", "1"));
const statesOption = option("--states");
if (args.some((a) => a.startsWith("--")) || !args.length) usage();
for (const lang of langs)
  if (!LOCALES.includes(lang)) {
    console.error(`unknown language ${lang} (${LOCALES.join(", ")})`);
    stop(2);
  }
if (!(scale > 0) || !(max >= 0)) usage();

const names = args;
const directories = names.map((name) =>
  name.includes("/") ? resolve(source, name) : join(source, "plugins", name),
);
for (const dir of directories)
  if (!existsSync(join(dir, "manifest.json"))) {
    console.error(`not a plugin directory: ${dir}`);
    stop(2);
  }
const pluginName = (dir) => basename(dir);
const same = (a, b) =>
  resolve(a).toLowerCase() === resolve(b).toLowerCase() ||
  resolve(a) === resolve(b);

const out = resolve(
  outOption ??
    join(tmpdir(), "vestra-ui-shots", names.map((n) => basename(n)).join("+")),
);
const inRepo = relative(resolve(source, ".."), out);
if (!inRepo.startsWith("..") && !isAbsolute(inRepo)) {
  console.error(`the output must be outside the repository: ${out}`);
  stop(2);
}
if (!outOption) rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

// 1. The tests, with every view event logged.
let statesDir = statesOption && resolve(statesOption);
if (!statesDir) {
  statesDir = join(out, "states");
  rmSync(statesDir, { recursive: true, force: true });
  console.log(`ui-shots: running the tests of ${names.join(", ")}`);
  const result = spawnSync(
    process.execPath,
    [join(source, "scripts", "plugin.mjs"), "test", ...names],
    {
      cwd: source,
      stdio: ["ignore", "ignore", "inherit"],
      env: {
        ...process.env,
        VESTRA_UI_STATES: statesDir,
        VESTRA_UI_RECORD: "",
      },
    },
  );
  if (result.status !== 0)
    console.warn(
      "ui-shots: some tests failed; the views they opened are still shown (run `npm run plugin -- test` for details)",
    );
}
const events = existsSync(statesDir)
  ? readdirSync(statesDir)
      .filter((f) => f.endsWith(".jsonl"))
      .sort()
      .map((f) =>
        readFileSync(join(statesDir, f), "utf8")
          .split("\n")
          .filter(Boolean)
          .map((line) => ({ ...JSON.parse(line), log: f })),
      )
  : [];

// 2. Each view's state when it closed or its test host ended.
/** Image and Markdown references carry a per-run `ref`: not part of a state. */
const normal = (value) => {
  if (Array.isArray(value)) return value.map(normal);
  if (value && typeof value === "object") {
    const special = "$vestraImage" in value || "$vestraMarkdown" in value;
    return Object.fromEntries(
      Object.keys(value)
        .filter((k) => !(special && k === "ref"))
        .sort()
        .map((k) => [k, normal(value[k])]),
    );
  }
  return value;
};
const snapshots = [];
for (const lines of events) {
  const open = new Map();
  const emit = (key) => {
    const state = open.get(key);
    open.delete(key);
    if (state) snapshots.push(state);
  };
  for (const line of lines) {
    const key = `${line.host}#${line.view}`;
    if (line.event === "open") {
      emit(key);
      open.set(key, {
        plugin: line.plugin,
        kind: line.kind,
        file: line.file,
        component: line.component ?? null,
        global: line.global ?? null,
        title: line.title ?? null,
        category: line.category ?? null,
        test: line.test,
        case: line.case,
        properties: {},
      });
    } else if (line.event === "set") {
      const state = open.get(key);
      if (state) {
        state.properties[line.property] = line.value;
        if (line.case) state.case = line.case;
      }
    } else if (line.event === "close") emit(key);
  }
  for (const key of [...open.keys()]) emit(key);
}

const views = new Map();
for (const [index, dir] of directories.entries()) {
  /** The same values in another test: listed with the first, not rendered again. */
  const seen = new Map();
  for (const snap of snapshots) {
    // Panels are not covered (their size is the slot's).
    if (!same(snap.plugin, dir) || snap.kind === "panel") continue;
    const state = `${basename(snap.test)} › ${snap.case || "(outside a test)"}`;
    if (stateFilter && !state.toLowerCase().includes(stateFilter.toLowerCase()))
      continue;
    const viewKey = [snap.kind, snap.file, snap.component ?? ""].join("|");
    const identity = JSON.stringify([
      viewKey,
      snap.title,
      normal(snap.properties),
    ]);
    const first = seen.get(identity);
    if (first) {
      if (first.state !== state && !first.sameFor.includes(state))
        first.sameFor.push(state);
      continue;
    }
    if (!views.has(viewKey + index))
      views.set(viewKey + index, { dir, snap, states: [] });
    const entry = { ...snap, plugin: dir, state, sameFor: [] };
    seen.set(identity, entry);
    views.get(viewKey + index).states.push(entry);
  }
  // Windows no test opened: their default values.
  if (stateFilter) continue;
  const walk = (folder) =>
    readdirSync(folder, { withFileTypes: true }).flatMap((entry) => {
      const path = join(folder, entry.name);
      if (entry.isDirectory())
        return ["dist", "node_modules", "vendor", "tests"].includes(entry.name)
          ? []
          : walk(path);
      return entry.name.endsWith(".slint") ? [path] : [];
    });
  for (const file of walk(dir)) {
    const text = readFileSync(file, "utf8");
    if (!/export\s+component\s+[\w-]+\s+inherits\s+Window\b/.test(text))
      continue;
    const rel = relative(dir, file).split(sep).join("/");
    const viewKey = ["window", rel, ""].join("|");
    if (views.has(viewKey + index)) continue;
    views.set(viewKey + index, {
      dir,
      snap: { kind: "window", file: rel },
      states: [
        {
          plugin: dir,
          kind: "window",
          file: rel,
          component: null,
          global: null,
          title: null,
          category: null,
          properties: {},
          state: "declared defaults (no test opened it)",
          sameFor: [],
        },
      ],
    });
  }
}

const shots = [];
for (const { dir, snap, states } of views.values()) {
  const view =
    snap.kind === "page"
      ? `${basename(snap.file, ".slint")}-${snap.component}`
      : basename(snap.file, ".slint");
  const chosen = max ? states.slice(0, max) : states;
  chosen.forEach((state, n) =>
    shots.push({
      ...state,
      id: `${pluginName(dir)}-${snap.kind}-${view}-${String(n + 1).padStart(2, "0")}`
        .replace(/[^\w.-]+/g, "_")
        .toLowerCase(),
      view: snap.kind === "page" ? `${snap.file} ${snap.component}` : snap.file,
    }),
  );
}
if (!shots.length) {
  console.error(
    "ui-shots: no window or settings page to show (no test opened one, and no .slint file declares a Window)",
  );
  stop(1);
}
console.log(`ui-shots: ${shots.length} views in ${langs.length} languages`);

// 3. The host's renderer (the test program), once per language.
const targetDir =
  process.env.CARGO_TARGET_DIR || join(tmpdir(), "vestra-target");
const build = spawnSync(
  "cargo",
  [
    "test",
    "--locked",
    "--bin",
    "vestra-host",
    "--no-run",
    "--message-format=json",
  ],
  {
    cwd: hostDir,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
  },
);
const executable = (build.stdout ?? "")
  .split("\n")
  .filter((line) => line.startsWith("{"))
  .map((line) => JSON.parse(line))
  .find(
    (m) =>
      m.reason === "compiler-artifact" &&
      m.target?.name === "vestra-host" &&
      m.profile?.test,
  )?.executable;
if (build.status !== 0 || !executable) {
  console.error("ui-shots: building the host's test program failed");
  stop(1);
}

const results = [];
for (const lang of langs) {
  const job = join(out, `job-${lang}.json`);
  writeFileSync(
    job,
    JSON.stringify({
      locale: lang,
      theme,
      scale,
      out,
      shots: shots.map((shot) => ({
        id: `${shot.id}-${lang}`,
        plugin: shot.plugin,
        kind: shot.kind,
        file: shot.file,
        component: shot.component,
        global: shot.global,
        title: shot.title,
        category: shot.category,
        properties: shot.properties,
      })),
    }),
  );
  const run = spawnSync(
    executable,
    [
      "--ignored",
      "--exact",
      "ui::ui_shots::tests::render_plugin_screenshots",
      "--nocapture",
    ],
    {
      cwd: hostDir,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, VESTRA_UI_SHOTS: job },
    },
  );
  const resultFile = join(out, `results-${lang}.json`);
  if (run.status !== 0 || !existsSync(resultFile)) {
    console.error(run.stdout, run.stderr);
    console.error(`ui-shots: rendering ${lang} failed`);
    stop(1);
  }
  const byId = new Map(shots.map((s) => [`${s.id}-${lang}`, s]));
  /** Identical pictures of one view: one PNG, its states listed together. */
  const pictures = new Map();
  for (const result of JSON.parse(readFileSync(resultFile, "utf8"))) {
    const shot = byId.get(result.id);
    let key;
    if (result.png && !result.problems.length) {
      const hash = createHash("sha256")
        .update(readFileSync(result.png))
        .digest("hex");
      key = `${shot.kind}|${shot.view}|${result.variant}|${hash}`;
      const first = pictures.get(key);
      if (first) {
        rmSync(result.png);
        first.sameFor.push(shot.state, ...shot.sameFor);
        continue;
      }
    }
    const entry = {
      png: result.png,
      plugin: JSON.parse(
        readFileSync(join(shot.plugin, "manifest.json"), "utf8"),
      ).id,
      kind: shot.kind,
      view: shot.view,
      variant: result.variant,
      lang,
      state: shot.state,
      width: result.width,
      height: result.height,
      problems: result.problems,
      sameFor: [...shot.sameFor],
    };
    if (key) pictures.set(key, entry);
    results.push(entry);
  }
}

// 4. The index.
writeFileSync(join(out, "index.json"), `${JSON.stringify(results, null, 2)}\n`);
const variantText = {
  window: "window",
  options: "page in the Options window",
  page: "whole page (taller than the Options window shows)",
};
const md = [
  `# UI screenshots: ${names.join(", ")}`,
  "",
  `Languages ${langs.join(", ")}; theme ${theme}; scale ${scale}. Rendered offscreen with the player's Slint pipeline; look at a PNG with the Read tool.`,
  "",
];
for (const r of results) {
  md.push(
    `- ${r.png ?? "(not rendered)"}`,
    `  - ${r.plugin} · ${r.view} · ${variantText[r.variant] ?? r.variant} · ${r.lang} · ${r.width}×${r.height}`,
    `  - state: ${r.state}`,
    ...(r.sameFor.length
      ? [
          `  - the same picture for ${r.sameFor.length} more: ${r.sameFor.join("; ")}`,
        ]
      : []),
    ...r.problems.map((p) => `  - problem: ${p}`),
  );
}
writeFileSync(join(out, "index.md"), `${md.join("\n")}\n`);
const problems = results.filter((r) => r.problems.length).length;
console.log(
  `ui-shots: ${results.filter((r) => r.png).length} images; ${problems} with problems\nindex: ${join(out, "index.md")}`,
);
