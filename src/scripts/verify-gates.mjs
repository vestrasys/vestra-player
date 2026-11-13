#!/usr/bin/env node
/**
 * Static gates for a change that must only add (doc/process/verify-additions.md):
 * compares a branch, or the working tree, of one repository with a base ref
 * and fails when something that existed was changed.
 *
 *   node src/scripts/verify-gates.mjs --repo <directory> --base <ref>
 *     [--head <ref>]        default: the working tree (tracked files, staged or
 *                           not; untracked files are new files and never fail)
 *     [--exact-base]        compare with <ref> itself; default: with the merge
 *                           base of <ref> and the head, so what landed on
 *                           <ref> afterwards does not count as a change
 *     [--kind player|sdk|plugins]   default: detected from the repository
 *     [--allow <path prefix>]...    findings under it are listed, not failed
 *                                   (an approved exception, named in the PR)
 *     [--all]               list every finding (default: 25 per gate)
 *     [--json]              the result as JSON on stdout
 *
 * Gates (a gate with nothing to compare in this repository is "n/a"):
 *
 *   (a) traces        SDK: every golden trace of the base
 *                     (test/fixtures/traces/) is byte-identical.
 *   (b) tests         no line of an existing test file is removed or modified
 *                     and no test file is deleted: only added lines and new
 *                     files. Test files: a path with a `test`, `tests` or
 *                     `__tests__` directory, `*.test.*`, `*.spec.*`, `tests.rs`,
 *                     `*_test.rs`, `*_tests.rs`; and, in any other Rust file,
 *                     the lines of the base under `#[cfg(test)]` or `#[test]`
 *                     (a module, a function, a `use`), and the files of a
 *                     module the base declares as `#[cfg(test)] mod x;`
 *                     (x.rs, x/mod.rs, a `#[path]`, and what is under x/).
 *                     A renamed or moved test file counts as deleted.
 *   (c) api-surface   SDK: no line of test/fixtures/api-surface.txt is
 *                     removed (a changed signature is a removed line).
 *   (d) capabilities  player src/host/capabilities.json, SDK
 *                     src/host-capabilities.ts: every capability and every
 *                     capability-free method of the base is there unchanged
 *                     and in the same order; a new entry takes no existing
 *                     method, also not one an existing `prefix.*` entry
 *                     serves (the host looks up the exact name first, then
 *                     the first prefix in table order); the other top-level
 *                     fields are unchanged.
 *
 * Exit code: 0 all gates pass, 1 a gate fails, 2 usage or git error.
 * No dependencies; needs git on PATH.
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { runInNewContext } from "node:vm";

/** What each kind of repository has to compare. */
export const KINDS = {
  player: {
    capabilities: { path: "src/host/capabilities.json", format: "json" },
  },
  sdk: {
    traces: "test/fixtures/traces/",
    surface: "test/fixtures/api-surface.txt",
    capabilities: { path: "src/host-capabilities.ts", format: "ts" },
  },
  plugins: {},
};

class GateError extends Error {}

function git(repo, args, { allowFail = false } = {}) {
  const result = spawnSync(
    "git",
    ["-C", repo, "-c", "core.quotePath=false", ...args],
    { encoding: "utf8", maxBuffer: 1 << 29 },
  );
  if (result.error) throw new GateError(`git: ${result.error.message}`);
  if (result.status !== 0) {
    if (allowFail) return null;
    throw new GateError(
      `git ${args.join(" ")}: ${result.stderr.trim() || `exit ${result.status}`}`,
    );
  }
  return result.stdout;
}

/** The text of `paths` at `commit`, in one git process: `[path, text]`. */
function blobs(repo, commit, paths) {
  if (!paths.length) return [];
  const result = spawnSync("git", ["-C", repo, "cat-file", "--batch"], {
    input: paths.map((path) => `${commit}:${path}\n`).join(""),
    maxBuffer: 1 << 30,
  });
  if (result.error || result.status !== 0)
    throw new GateError(
      `git cat-file: ${result.error?.message ?? result.stderr.toString().trim()}`,
    );
  const out = [];
  const data = result.stdout;
  let at = 0;
  for (const path of paths) {
    const end = data.indexOf(10, at);
    const header = data.toString("utf8", at, end).split(" ");
    at = end + 1;
    if (header[1] !== "blob") continue; // "<name> missing"
    const size = Number(header[2]);
    out.push([path, data.toString("utf8", at, at + size)]);
    at += size + 1;
  }
  return out;
}

/** Whether `path` (forward slashes) is a test file by its name. */
export function isTestPath(path) {
  const parts = path.split("/");
  const name = parts.pop();
  if (parts.some((part) => /^(tests?|__tests__)$/.test(part))) return true;
  return (
    /\.(test|spec)\.[cm]?[jt]sx?$/.test(name) ||
    name === "tests.rs" ||
    /_tests?\.rs$/.test(name)
  );
}

/**
 * The lines of a Rust source under `#[cfg(test)]`: `[{ start, end }]`,
 * 1-based and inclusive, from the attribute to the end of its item (the
 * matching `}` of a module, function or impl, or the `;` of a `use` or
 * `mod x;`). `#![cfg(test)]` covers the file. `cfg(not(test))` and
 * `cfg(any(…, test))` (code that also builds without tests) do not count,
 * nor does `cfg(feature = "test…")` (a feature is not the test build). An
 * item under `#[test]` (or `#[x::test]`) counts wherever it is.
 * Comments, strings (raw ones too) and character literals are skipped.
 */
export function rustTestRanges(source) {
  return scanRust(source).ranges;
}

/**
 * The modules a Rust source declares for tests only, in another file:
 * `#[cfg(test)] mod x;`, also with `#[path = "…"]`: `[{ name, path }]`
 * (`path` null without the attribute).
 */
export function rustTestModules(source) {
  return scanRust(source).modules;
}

/**
 * Where the test modules declared in `file` live: `{ files, directories }`
 * (repository paths with `/`). `mod x;` in `a/mod.rs`, `a/lib.rs` or
 * `a/main.rs` is `a/x.rs` or `a/x/mod.rs`, in `a/b.rs` it is `a/b/x.rs` or
 * `a/b/x/mod.rs`; a `#[path]` is relative to the declaring file's directory.
 * Everything under the module's directory belongs to it.
 */
export function testModuleTargets(file, modules) {
  const parts = file.split("/");
  const name = parts.pop().replace(/\.rs$/, "");
  const directory = parts.join("/");
  const join = (...items) => {
    const out = [];
    for (const part of items.join("/").split("/")) {
      if (part === "" || part === ".") continue;
      if (part === ".." && out.length) out.pop();
      else out.push(part);
    }
    return out.join("/");
  };
  const owner = ["mod", "lib", "main"].includes(name)
    ? directory
    : join(directory, name);
  const files = [];
  const directories = [];
  for (const module of modules) {
    if (module.path) {
      const target = join(directory, module.path);
      files.push(target);
      directories.push(
        target.endsWith("/mod.rs")
          ? target.slice(0, -"/mod.rs".length)
          : target.replace(/\.rs$/, ""),
      );
    } else {
      files.push(join(owner, `${module.name}.rs`));
      directories.push(join(owner, module.name));
    }
  }
  return { files, directories };
}

function scanRust(source) {
  const ranges = [];
  const modules = [];
  /** Where the run of attributes before the current item began. */
  let chain = null;
  const length = source.length;
  let at = 0;
  let line = 1;
  /** A pending `#[cfg(test)]`: its line and the bracket depth since. */
  let pending = null;
  let open = null;

  const advance = (to) => {
    for (; at < to && at < length; at++) if (source[at] === "\n") line++;
  };
  /** The end of the comment, string or character literal at `at`, or -1. */
  const literalEnd = () => {
    const c = source[at];
    const next = source[at + 1];
    if (c === "/" && next === "/") {
      const end = source.indexOf("\n", at);
      return end < 0 ? length : end;
    }
    if (c === "/" && next === "*") {
      let depth = 1;
      let i = at + 2;
      while (i < length && depth > 0) {
        if (source[i] === "/" && source[i + 1] === "*") (depth++, (i += 2));
        else if (source[i] === "*" && source[i + 1] === "/")
          (depth--, (i += 2));
        else i++;
      }
      return i;
    }
    const identifierBefore = at > 0 && /[A-Za-z0-9_]/.test(source[at - 1]);
    if (!identifierBefore) {
      // r"…", r#"…"#, br"…", cr#"…"#
      const raw = /^(?:[bc]?r)(#*)"/.exec(source.slice(at, at + 40));
      if (raw) {
        const close = `"${raw[1]}`;
        const end = source.indexOf(close, at + raw[0].length);
        return end < 0 ? length : end + close.length;
      }
    }
    if (c === '"') {
      let i = at + 1;
      while (i < length && source[i] !== '"') i += source[i] === "\\" ? 2 : 1;
      return Math.min(i + 1, length);
    }
    if (c === "'") {
      if (next === "\\") {
        const end = source.indexOf("'", at + 3);
        return end < 0 ? length : end + 1;
      }
      const width = (source.codePointAt(at + 1) ?? 0) > 0xffff ? 2 : 1;
      // A lifetime (`'a`) has no closing quote.
      return source[at + 1 + width] === "'" ? at + 2 + width : -1;
    }
    return -1;
  };

  while (at < length) {
    const end = literalEnd();
    if (end >= 0) {
      advance(end);
      continue;
    }
    const c = source[at];
    if (c === "#" && /^#!?\[/.test(source.slice(at, at + 3))) {
      const startLine = line;
      chain ??= at;
      const inner = source[at + 1] === "!";
      advance(at + (inner ? 2 : 1));
      let depth = 0;
      let text = "";
      while (at < length) {
        const skip = literalEnd();
        if (skip >= 0) {
          advance(skip);
          continue;
        }
        const d = source[at];
        text += d;
        advance(at + 1);
        if (d === "[") depth++;
        else if (d === "]" && --depth === 0) break;
      }
      const attribute = text.replace(/\s+/g, "");
      // `any(windows, test)` also builds without tests: not test code.
      // (A string is not part of `attribute`: `feature = "test"` is none.)
      const isTest =
        (/^\[cfg\(.*\btest\b.*\)\]$/.test(attribute) &&
          !/\b(not|any)\(/.test(attribute)) ||
        /^\[(\w+::)*test(\(.*\))?\]$/.test(attribute);
      if (isTest && inner) {
        return {
          ranges: [{ start: 1, end: source.split("\n").length }],
          modules: [],
        };
      }
      if (isTest && !pending && !open)
        pending = { start: startLine, depth: 0, from: chain };
      continue;
    }
    if (!/\s/.test(c)) chain = null;
    if (open) {
      if (c === "{") open.depth++;
      else if (c === "}" && --open.depth === 0) {
        // `#[cfg(test)] let x = if a { … } else { … };` goes on.
        if (/^\s*else\b/.test(source.slice(at + 1, at + 80)))
          pending = { start: open.start, depth: 0 };
        else ranges.push({ start: open.start, end: line });
        open = null;
      }
    } else if (pending) {
      if (c === "(" || c === "[") pending.depth++;
      else if (c === ")" || c === "]") pending.depth--;
      else if (c === ";" && pending.depth === 0) {
        ranges.push({ start: pending.start, end: line });
        // `#[cfg(test)] #[path = "x.rs"] mod x;`: the module is a file.
        const item = source.slice(pending.from, at);
        const module = /\bmod\s+(\w+)\s*$/.exec(item);
        if (module)
          modules.push({
            name: module[1],
            path: /#\[\s*path\s*=\s*"([^"]*)"\s*\]/.exec(item)?.[1] ?? null,
          });
        pending = null;
      } else if (c === "{" && pending.depth === 0) {
        open = { start: pending.start, depth: 1 };
        pending = null;
      }
    }
    advance(at + 1);
  }
  // An item cut off at the end of the file still counts to its end.
  const unfinished = open ?? pending;
  if (unfinished) ranges.push({ start: unfinished.start, end: line });
  return { ranges, modules };
}

/**
 * The removed lines of one file's `git diff -U0`: `{ binary, removed: [{
 * line, text }] }`, `line` being the line number in the old file.
 */
export function removedLines(diff) {
  const removed = [];
  let binary = false;
  let old = 0;
  let inHunk = false;
  for (const text of diff.split("\n")) {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+\d+(?:,\d+)? @@/.exec(text);
    if (hunk) {
      old = Number(hunk[1]);
      inHunk = true;
    } else if (!inHunk) {
      if (/^Binary files .* differ$/.test(text) || text === "GIT binary patch")
        binary = true;
    } else if (text.startsWith("-")) {
      removed.push({ line: old++, text: text.slice(1).replace(/\r$/, "") });
    }
  }
  return { binary, removed };
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonical(value[key])]),
    );
  return value;
}
const same = (a, b) =>
  JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));

/** The SDK's copy of the table (src/host-capabilities.ts) as a value. */
export function parseCapabilitiesTs(text) {
  const start = text.indexOf("HOST_CAPABILITIES");
  const open = start < 0 ? -1 : text.indexOf("{", text.indexOf("=", start));
  const close = text.lastIndexOf("}");
  if (open < 0 || close < open)
    throw new GateError("HOST_CAPABILITIES = { … } not found");
  // A data literal (generated from the player's JSON), evaluated with no
  // globals in reach.
  const value = runInNewContext(
    `(${text.slice(open, close + 1)})`,
    Object.create(null),
    { timeout: 2000 },
  );
  return JSON.parse(JSON.stringify(value));
}

/**
 * Gate (d) on two capability tables: `{ problems, notes }`. Existing
 * entries must be unchanged (key order inside an entry does not count; the
 * order of its lists does) and keep their order; new entries are notes.
 */
export function compareCapabilities(base, head) {
  const problems = [];
  const notes = [];
  for (const key of new Set([...Object.keys(base), ...Object.keys(head)])) {
    if (key === "capabilities" || key === "uncapable") continue;
    if (!(key in head)) problems.push(`top-level "${key}" is removed`);
    else if (!(key in base)) notes.push(`top-level "${key}" is new`);
    else if (!same(base[key], head[key]))
      problems.push(`top-level "${key}" changed`);
  }
  const taken = new Set();
  for (const entry of base.capabilities ?? [])
    for (const method of entry.methods ?? []) taken.add(method.name);
  for (const entry of base.uncapable ?? []) taken.add(entry.name);

  for (const [list, what] of [
    ["capabilities", "capability"],
    ["uncapable", "capability-free method"],
  ]) {
    const before = base[list] ?? [];
    const after = head[list] ?? [];
    const byName = new Map(after.map((entry) => [entry.name, entry]));
    const known = new Set(before.map((entry) => entry.name));
    for (const entry of before) {
      const now = byName.get(entry.name);
      if (!now) {
        problems.push(`${what} "${entry.name}" is removed`);
        continue;
      }
      if (same(entry, now)) continue;
      const fields = [];
      for (const key of new Set([...Object.keys(entry), ...Object.keys(now)])) {
        if (same(entry[key], now[key])) continue;
        if (key !== "methods") {
          fields.push(
            `${key}: ${JSON.stringify(entry[key])} → ${JSON.stringify(now[key])}`,
          );
          continue;
        }
        const old = new Map((entry.methods ?? []).map((m) => [m.name, m]));
        const cur = new Map((now.methods ?? []).map((m) => [m.name, m]));
        const gone = [...old.keys()].filter((name) => !cur.has(name));
        const added = [...cur.keys()].filter((name) => !old.has(name));
        const changed = [...old.keys()].filter(
          (name) => cur.has(name) && !same(old.get(name), cur.get(name)),
        );
        const parts = [];
        if (gone.length) parts.push(`removed ${gone.join(", ")}`);
        if (added.length) parts.push(`added ${added.join(", ")}`);
        if (changed.length)
          parts.push(`needs changed for ${changed.join(", ")}`);
        if (!parts.length) parts.push("order changed");
        fields.push(`methods: ${parts.join("; ")}`);
      }
      problems.push(`${what} "${entry.name}" changed (${fields.join("; ")})`);
    }
    const order = after
      .map((entry) => entry.name)
      .filter((name) => known.has(name));
    const expected = before
      .map((entry) => entry.name)
      .filter((name) => byName.has(name));
    if (order.join("\n") !== expected.join("\n")) {
      const first = expected.findIndex((name, index) => order[index] !== name);
      problems.push(
        `${list}: the existing entries changed order (first at "${expected[first]}")`,
      );
    }
    for (const entry of after) {
      if (known.has(entry.name)) continue;
      notes.push(`new ${what} "${entry.name}"`);
      const names =
        list === "uncapable"
          ? [entry.name]
          : (entry.methods ?? []).map((method) => method.name);
      for (const name of names) {
        const prefix = name.endsWith(".*") ? name.slice(0, -1) : null;
        const hit = prefix
          ? [...taken].find((old) => old.startsWith(prefix))
          : taken.has(name)
            ? name
            : null;
        if (hit)
          problems.push(
            `new ${what} "${entry.name}" takes the existing method "${hit}"${prefix ? ` (by "${name}")` : ""}`,
          );
      }
    }
  }
  // The host looks a method up by its exact name first, then by the first
  // `prefix.*` entry in table order (capabilities/mod.rs `lookup`): a new
  // exact name under an existing prefix, or a new prefix before an existing
  // one it overlaps, takes methods over without touching an old entry.
  const routes = (table) => {
    const exact = new Map();
    const prefixes = [];
    const add = (owner, method) => {
      const route = { owner, needs: method.needs ?? null, by: method.name };
      if (method.name.endsWith("*"))
        prefixes.push([method.name.slice(0, -1), route]);
      else exact.set(method.name, route);
    };
    for (const entry of table.capabilities ?? [])
      for (const method of entry.methods ?? []) add(entry.name, method);
    for (const entry of table.uncapable ?? []) add(null, entry);
    return {
      prefixes,
      names: [...exact.keys()],
      of: (name) =>
        exact.get(name) ??
        prefixes.find(([prefix]) => name.startsWith(prefix))?.[1] ??
        null,
    };
  };
  const [old, now] = [routes(base), routes(head)];
  const probes = new Set([
    ...now.names,
    ...old.prefixes.map(([prefix]) => `${prefix}<any other>`),
    ...now.prefixes.map(([prefix]) => `${prefix}<any other>`),
  ]);
  const served = (route) =>
    `${route.owner === null ? "no capability" : `"${route.owner}"`} (entry "${route.by}")`;
  for (const name of probes) {
    const [before, after] = [old.of(name), now.of(name)];
    if (!before || !after) continue;
    // The same entry on both sides: a change of it is reported above.
    if (before.by === after.by && before.owner === after.owner) continue;
    if (before.owner === after.owner && same(before.needs, after.needs))
      continue;
    problems.push(
      `method "${name}" was served by ${served(before)} and is now served by ${served(after)}`,
    );
  }
  return { problems, notes };
}

/** Gate (c): the lines of the base snapshot missing from the head's. */
export function compareSurface(base, head) {
  const lines = (text) =>
    text
      .split("\n")
      .map((line) => line.replace(/\r$/, ""))
      .filter((line) => line.trim() !== "");
  const left = new Map();
  for (const line of lines(head)) left.set(line, (left.get(line) ?? 0) + 1);
  const removed = [];
  const before = lines(base);
  for (const line of before) {
    const count = left.get(line) ?? 0;
    if (count > 0) left.set(line, count - 1);
    else removed.push(line);
  }
  const added = [...left.values()].reduce((sum, count) => sum + count, 0);
  return { removed, added, total: before.length };
}

/** `player`, `sdk` or `plugins`, from what the repository holds at `ref`. */
export function detectKind(repo, ref) {
  const has = (path) =>
    ref
      ? git(repo, ["cat-file", "-e", `${ref}:${path}`], { allowFail: true }) !==
        null
      : existsSync(join(repo, path));
  if (has("src/host/capabilities.json")) return "player";
  if (has("src/host-capabilities.ts") || has("test/fixtures/api-surface.txt"))
    return "sdk";
  if (has("plugins/owners.toml") || has("registry")) return "plugins";
  throw new GateError(
    `${repo}: not a player, SDK or plugin repository (use --kind)`,
  );
}

/**
 * Runs the gates: `{ repo, kind, base, head, gates: [{ id, title, status:
 * "pass" | "fail" | "n/a", summary, problems, allowed, notes }], pass }`.
 * `head` null: the working tree.
 */
export function runGates({
  repo,
  base,
  head = null,
  kind = null,
  allow = [],
  exactBase = false,
}) {
  repo = resolve(repo);
  const resolveRef = (ref) =>
    git(repo, ["rev-parse", "--verify", `${ref}^{commit}`]).trim();
  const headCommit = resolveRef(head ?? "HEAD");
  let baseCommit = resolveRef(base);
  let baseNote = base;
  if (!exactBase) {
    const merge = git(repo, ["merge-base", baseCommit, headCommit], {
      allowFail: true,
    });
    if (merge === null)
      throw new GateError(`${base} and the head share no history`);
    if (merge.trim() !== baseCommit)
      baseNote = `merge base of ${base} and ${head ?? "HEAD"}`;
    baseCommit = merge.trim();
  }
  kind ??= detectKind(repo, baseCommit);
  const config = KINDS[kind];
  if (!config) throw new GateError(`unknown kind ${kind}`);
  const range = head ? [baseCommit, headCommit] : [baseCommit];
  const allowed = (path) =>
    allow.some((prefix) => path.startsWith(prefix.replaceAll("\\", "/")));

  // Every tracked path that differs: [status, path].
  const fields = git(repo, [
    "diff",
    "--name-status",
    "-z",
    "--no-renames",
    ...range,
  ]).split("\0");
  const changes = [];
  for (let i = 0; i + 1 < fields.length; i += 2)
    changes.push([fields[i][0], fields[i + 1]]);
  const baseFile = (path) =>
    git(repo, ["show", `${baseCommit}:${path}`], { allowFail: true });
  const headFile = (path) => {
    if (head)
      return git(repo, ["show", `${headCommit}:${path}`], { allowFail: true });
    const file = join(repo, path);
    return existsSync(file) ? readFileSync(file, "utf8") : null;
  };
  const diffOf = (path) =>
    removedLines(
      git(repo, [
        "diff",
        "-U0",
        "--no-renames",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        ...range,
        "--",
        path,
      ]),
    );

  const gates = [];
  const gate = (id, title) => {
    const entry = {
      id,
      title,
      status: "pass",
      summary: "",
      problems: [],
      allowed: [],
      notes: [],
    };
    gates.push(entry);
    return entry;
  };
  const report = (entry, path, text) =>
    (allowed(path) ? entry.allowed : entry.problems).push(text);
  const WHAT = { M: "modified", D: "deleted", T: "type changed" };

  // (a) golden traces
  const traces = gate("a", "golden traces byte-identical");
  if (!config.traces) traces.status = "n/a";
  else {
    const existing = git(repo, [
      "ls-tree",
      "-r",
      "--name-only",
      "-z",
      baseCommit,
      "--",
      config.traces,
    ])
      .split("\0")
      .filter(Boolean);
    if (!existing.length) traces.status = "n/a";
    let changed = 0;
    for (const [status, path] of changes) {
      if (!path.startsWith(config.traces) || status === "A") continue;
      changed++;
      report(traces, path, `${path}: ${WHAT[status] ?? status}`);
    }
    const added = changes.filter(
      ([status, path]) => status === "A" && path.startsWith(config.traces),
    ).length;
    traces.summary = `${existing.length - changed} of ${existing.length} identical, ${added} new`;
  }

  // (b) existing tests
  const tests = gate("b", "existing tests: only added lines and new files");
  let testFiles = 0;
  let rustFiles = 0;
  // Rust files that are test modules of the base by declaration
  // (`#[cfg(test)] mod x;` somewhere): path or directory -> declaring file.
  let declared = null;
  const declaredBy = (path) => {
    if (!declared) {
      declared = { files: new Map(), directories: [] };
      const sources = git(repo, [
        "ls-tree",
        "-r",
        "--name-only",
        "-z",
        baseCommit,
      ])
        .split("\0")
        .filter((name) => name.endsWith(".rs"));
      for (const [file, source] of blobs(repo, baseCommit, sources)) {
        if (!source.includes("mod ")) continue;
        const modules = rustTestModules(source);
        if (!modules.length) continue;
        const targets = testModuleTargets(file, modules);
        for (const target of targets.files) declared.files.set(target, file);
        for (const target of targets.directories)
          declared.directories.push([`${target}/`, file]);
      }
    }
    return (
      declared.files.get(path) ??
      declared.directories.find(([prefix]) => path.startsWith(prefix))?.[1] ??
      null
    );
  };
  for (const [status, path] of changes) {
    if (status === "A") continue;
    if (config.traces && path.startsWith(config.traces)) continue; // gate (a)
    if (path === config.surface) continue; // gate (c)
    const preview = (lines) =>
      lines
        .slice(0, 3)
        .map((l) => `\n        ${l.line}: ${l.text.trim().slice(0, 110)}`)
        .join("") +
      (lines.length > 3 ? `\n        … ${lines.length - 3} more` : "");
    const declarer =
      !isTestPath(path) && path.endsWith(".rs") ? declaredBy(path) : null;
    if (isTestPath(path) || declarer) {
      testFiles++;
      const why = declarer ? ` (a #[cfg(test)] module of ${declarer})` : "";
      if (status !== "M") {
        report(tests, path, `${path}: ${WHAT[status] ?? status}${why}`);
        continue;
      }
      const { binary, removed } = diffOf(path);
      if (binary) report(tests, path, `${path}: binary file modified`);
      else if (removed.length)
        report(
          tests,
          path,
          `${path}: ${removed.length} line(s) removed or modified${why}${preview(removed)}`,
        );
      continue;
    }
    if (!path.endsWith(".rs")) continue;
    const source = baseFile(path);
    if (source === null) continue;
    const ranges = rustTestRanges(source);
    if (!ranges.length) continue;
    rustFiles++;
    const inTests = (line) =>
      ranges.some((range) => line >= range.start && line <= range.end);
    if (status === "D") {
      report(
        tests,
        path,
        `${path}: deleted with its #[cfg(test)] code (lines ${ranges.map((r) => `${r.start}-${r.end}`).join(", ")})`,
      );
      continue;
    }
    const removed = diffOf(path).removed.filter((l) => inTests(l.line));
    if (removed.length)
      report(
        tests,
        path,
        `${path}: ${removed.length} line(s) under #[cfg(test)] removed or modified${preview(removed)}`,
      );
  }
  tests.summary = `${testFiles} changed test file(s) and ${rustFiles} changed Rust file(s) with #[cfg(test)] code looked at`;

  // (c) API surface
  const surface = gate("c", "API surface snapshot: additions only");
  const surfaceBase = config.surface ? baseFile(config.surface) : null;
  if (surfaceBase === null) surface.status = "n/a";
  else {
    const now = headFile(config.surface);
    if (now === null)
      report(surface, config.surface, `${config.surface}: deleted`);
    else {
      const { removed, added, total } = compareSurface(surfaceBase, now);
      surface.summary = `${total - removed.length} of ${total} lines kept, ${added} added`;
      for (const line of removed)
        report(surface, config.surface, `removed or changed: ${line}`);
    }
  }

  // (d) capability table
  const capabilities = gate(
    "d",
    "capability table: existing entries identical",
  );
  const table = config.capabilities;
  const tableBase = table ? baseFile(table.path) : null;
  if (tableBase === null) capabilities.status = "n/a";
  else {
    const read = (text) =>
      table.format === "json" ? JSON.parse(text) : parseCapabilitiesTs(text);
    const now = headFile(table.path);
    if (now === null)
      report(capabilities, table.path, `${table.path}: deleted`);
    else {
      let before = null;
      let after = null;
      try {
        before = read(tableBase);
        after = read(now);
      } catch (error) {
        capabilities.problems.push(
          `${table.path}: unreadable (${error.message})`,
        );
      }
      if (before && after) {
        const { problems, notes } = compareCapabilities(before, after);
        for (const problem of problems)
          report(capabilities, table.path, `${table.path}: ${problem}`);
        capabilities.notes.push(...notes);
        capabilities.summary = `${(before.capabilities ?? []).length} capabilities and ${(before.uncapable ?? []).length} capability-free methods in the base, ${notes.length} new entr${notes.length === 1 ? "y" : "ies"}`;
      }
    }
  }

  for (const entry of gates)
    if (entry.status !== "n/a" && entry.problems.length) entry.status = "fail";
  return {
    repo,
    kind,
    base: { commit: baseCommit, note: baseNote },
    head: head
      ? { commit: headCommit, note: head }
      : { commit: headCommit, note: "working tree" },
    gates,
    pass: gates.every((entry) => entry.status !== "fail"),
  };
}

/** The result as the text the command prints. */
export function format(result, { all = false } = {}) {
  const limit = all ? Infinity : 25;
  const out = [
    `V1 static gates: ${result.kind}  ${result.repo}`,
    `  base  ${result.base.commit.slice(0, 8)} (${result.base.note})`,
    `  head  ${result.head.commit.slice(0, 8)} (${result.head.note})`,
  ];
  const list = (label, items) => {
    for (const item of items.slice(0, limit)) out.push(`    ${label}${item}`);
    if (items.length > limit)
      out.push(`    … ${items.length - limit} more (--all lists them)`);
  };
  for (const entry of result.gates) {
    const mark = { pass: "PASS", fail: "FAIL", "n/a": " n/a" }[entry.status];
    out.push(
      `[${mark}] (${entry.id}) ${entry.title}${entry.summary ? ` — ${entry.summary}` : ""}${entry.status === "fail" ? `; ${entry.problems.length} finding(s)` : ""}`,
    );
    if (entry.status === "n/a") continue;
    list("", entry.problems);
    list("allowed by --allow: ", entry.allowed);
    list("note: ", entry.notes);
  }
  const failed = result.gates.filter((entry) => entry.status === "fail");
  out.push(
    result.pass
      ? "RESULT: PASS"
      : `RESULT: FAIL (${failed.map((entry) => `(${entry.id})`).join(" ")})`,
  );
  return out.join("\n");
}

function main(argv) {
  const options = { allow: [] };
  for (let i = 0; i < argv.length; i++) {
    const name = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new GateError(`${name} needs a value`);
      return argv[++i];
    };
    if (name === "--repo") options.repo = value();
    else if (name === "--base") options.base = value();
    else if (name === "--head") options.head = value();
    else if (name === "--kind") options.kind = value();
    else if (name === "--allow") options.allow.push(value());
    else if (name === "--exact-base") options.exactBase = true;
    else if (name === "--all") options.all = true;
    else if (name === "--json") options.json = true;
    else throw new GateError(`unknown option ${name}`);
  }
  if (!options.repo || !options.base)
    throw new GateError(
      "usage: verify-gates.mjs --repo <directory> --base <ref> [--head <ref>] [--exact-base] [--kind player|sdk|plugins] [--allow <path prefix>]... [--all] [--json]",
    );
  const result = runGates(options);
  console.log(
    options.json ? JSON.stringify(result, null, 2) : format(result, options),
  );
  return result.pass ? 0 : 1;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof GateError)) throw error;
    console.error(`verify-gates: ${error.message}`);
    process.exitCode = 2;
  }
}
