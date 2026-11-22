// The plugin tools behind the agent's scaffold_plugin, typecheck, test,
// check and pack, and the source export: run in the plugin process (never in
// the agent's restricted worker), on fixed arguments, each with a time
// limit, capped output and cancellation. No Plugin object here: the eval
// runner (tests/eval/run.mjs) uses the same code.
//
// Layout (all inside the plugin's data folder):
//
//   node_modules/@vestra-player/sdk, @types/node, undici-types
//       a copy of the vendored SDK and its types, so that a project in
//       sessions/<sid>/work/<id>/ resolves `@vestra-player/sdk` the usual way
//       (TypeScript and Node walk up to this folder). Written once per SDK version.
//   sessions/<sid>/work/<id>/     the project (the agent's files)
//   sessions/<sid>/dist/          packed .vxp files
//   sessions/<sid>/tmp/           scratch for one tool call (TMPDIR of its child)
//   sessions/<sid>/tsconfig.json  strict settings, should the project lose its own
import { createHash, randomBytes } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { baseEnvironment, runNode, type RunResult } from "./run.ts";
import { zip, type ZipEntry } from "./zip.ts";

export interface ToolResult {
  ok: boolean;
  /** For the model: a one-line summary, then details (JSON). */
  text: string;
  /** For the window (the result card): the summary as a template key and its arguments (ai.slint). */
  say?: { code: string; args: string[] };
}

export interface SessionPaths {
  /** sessions/<sid>/ */
  dir: string;
  /** sessions/<sid>/work/ */
  work: string;
}

export interface Project {
  id: string;
  name: string;
  version: string;
  directory: string;
  modified: number;
}

/** Owner prefix when the user set no GitHub name: local only, never published as is. */
export const LOCAL_OWNER = "local";
const OWNER = /^[a-z0-9](?:[a-z0-9-]{0,38})$/;
const NAME = /^[a-z][a-z0-9-]{1,38}$/;
const ID = /^[A-Za-z0-9][A-Za-z0-9-]*\.[A-Za-z0-9][A-Za-z0-9._-]*$/;

/** Why a GitHub name cannot be the id prefix: `code` is the settings page's hint. */
export class OwnerError extends Error {
  readonly code: "owner-chars" | "owner-vestra" | "owner-local";
  constructor(code: OwnerError["code"], message: string) {
    super(message);
    this.code = code;
  }
}

/** A GitHub user or organization name usable as id prefix, lower case; "" when unset; throws an OwnerError when invalid. */
export function checkOwner(text: string): string {
  const owner = text.trim().toLowerCase();
  if (!owner) return "";
  if (!OWNER.test(owner)) throw new OwnerError("owner-chars", 'A GitHub name has only letters, digits and "-", at most 39 characters');
  if (owner === "vestra") throw new OwnerError("owner-vestra", '"vestra" is reserved for official plugins; enter your own GitHub name');
  if (owner === LOCAL_OWNER) throw new OwnerError("owner-local", '"local" means no name was entered; enter your own GitHub name');
  return owner;
}

export const TIMEOUTS = { typecheck: 90_000, test: 90_000, check: 120_000, pack: 120_000 };
/** Text returned to the model at most per call. */
const MAX_RESULT = 12_000;

const json = (value: unknown) => JSON.stringify(value, null, 1);
/** The last lines of a child's output (where a crash says why), at most 4000 characters. */
const tail = (text: string) => text.trim().split(/\r?\n/).slice(-40).join("\n").slice(-4000);
/** One flag for NODE_OPTIONS: in double quotes, where a backslash escapes the next character. */
export const nodeOption = (flag: string) => `"${flag.replace(/[\\"]/g, "\\$&")}"`;
/** For a summary: the exit code when the CLI failed without its usual report. */
const unexplained = (report: { exitCode?: number | null }) => (report.exitCode === undefined ? "" : ` (the plugin CLI exited with code ${report.exitCode} without a report; its stderr is below)`);
const result = (ok: boolean, summary: string, details?: unknown): ToolResult => {
  let text = details === undefined ? summary : `${summary}\n${json(details)}`;
  if (text.length > MAX_RESULT) text = `${text.slice(0, MAX_RESULT)}\n… (output too long, cut)`;
  return { ok, text };
};

/** What the model does after the player refused a package. */
export const PLAYER_NEXT =
  "Fix each error line at the file:line:column the player names (warning lines can be ignored; a JavaScript error names the built dist/main.js: fix the .ts source it came from), then run check again";
/** The player's error as it said it: Slint's diagnostics, one per line (`<file>:<line>:<column>: error|warning: <message>`), stay whole. */
export const playerMessage = (error: unknown) => (error instanceof Error ? error.message : String(error)).slice(0, 6000);
export const playerCode = (error: unknown) => String((error as { code?: unknown })?.code ?? "error");
/** Waits for `work`; true when `signal` aborted first (or already had). `work`'s rejection passes through. */
async function raceAbort(work: Promise<unknown>, signal: AbortSignal | undefined): Promise<boolean> {
  if (!signal) return (await work, false);
  if (signal.aborted) return true;
  let onAbort = () => {};
  const aborted = new Promise<true>((resolve) => {
    onAbort = () => resolve(true);
    signal.addEventListener("abort", onAbort, { once: true });
  });
  try {
    return await Promise.race([work.then(() => false), aborted]);
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
}
const playerRefusal = (error: unknown) =>
  playerCode(error) === "slint" ? "Check failed: the player's Slint compile reported errors" : `Check failed: the player's package check refused it (${playerCode(error)})`;

export interface Diagnostic {
  file: string;
  line: number;
  column: number;
  code: string;
  message: string;
}

/** tsc's `file(line,col): error TSnnnn: message` lines (continuations joined). */
export function parseTsc(output: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  for (const line of output.split(/\r?\n/)) {
    const match = /^(.+?)\((\d+),(\d+)\): error (TS\d+): (.*)$/.exec(line);
    if (match) out.push({ file: match[1].split(path.sep).join("/"), line: Number(match[2]), column: Number(match[3]), code: match[4], message: match[5] });
    else if (out.length && /^\s+\S/.test(line)) out.at(-1)!.message += ` ${line.trim()}`;
  }
  return out;
}

export interface TestReport {
  passed: number;
  failed: number;
  failures: { name: string; error: string; location: string }[];
  /** What the tests printed (console.log / console.error), when a run failed. */
  output?: string;
}

/** Most of the tests' own output kept in a failed run's report. */
const MAX_OUTPUT = { lines: 60, chars: 4000 };

/** A TAP line (at any subtest depth), not something a test printed. */
const TAP_LINE = /^\s*(TAP version \d+|# |#$|ok \d+|not ok \d+|1\.\.\d+|---$|\.\.\.$|Bail out!)/;

/**
 * What the tests printed: `--test-isolation=none` writes console output to
 * stdout between the TAP lines, unprefixed; stderr has console.error.
 */
export function testOutput(stdout: string, stderr: string): string {
  const lines: string[] = [];
  let yaml = -1; // indent of the open YAML block's "---", -1 outside
  for (const line of stdout.split(/\r?\n/)) {
    const indent = line.length - line.trimStart().length;
    if (yaml >= 0) {
      if (line.trim() === "..." && indent === yaml) yaml = -1;
      continue;
    }
    if (line.trim() === "---") {
      yaml = indent;
      continue;
    }
    if (line.trim() === "" || TAP_LINE.test(line)) continue;
    lines.push(line);
  }
  for (const line of stderr.split(/\r?\n/)) if (line.trim() && !/^\(node:\d+\) (ExperimentalWarning|SecurityWarning)|^\(Use `node --trace-warnings/.test(line)) lines.push(line);
  let text = lines.slice(0, MAX_OUTPUT.lines).join("\n");
  if (text.length > MAX_OUTPUT.chars) text = `${text.slice(0, MAX_OUTPUT.chars)}…`;
  else if (lines.length > MAX_OUTPUT.lines) text += `\n… (${lines.length - MAX_OUTPUT.lines} more lines)`;
  return text;
}

/** The TAP reporter's output of `node --test`. */
export function parseTap(output: string, root: string): TestReport {
  const report: TestReport = { passed: 0, failed: 0, failures: [] };
  const lines = output.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const pass = /^# pass (\d+)/.exec(line);
    const fail = /^# fail (\d+)/.exec(line);
    if (pass) report.passed = Number(pass[1]);
    if (fail) report.failed = Number(fail[1]);
    const bad = /^(\s*)not ok \d+ - (.*)$/.exec(line);
    if (!bad) continue;
    // The YAML block below the line: location and error. Only its own keys
    // (two spaces deeper than "not ok"): the `actual:` / `expected:` values
    // of an assertion are nested deeper and may have an `error` field too.
    const key = " ".repeat(bad[1].length + 2);
    let error = "";
    let location = "";
    for (let j = i + 1; j < lines.length && !/^\s*\.\.\.$/.test(lines[j]); j++) {
      if (!lines[j].startsWith(key) || lines[j][key.length] === " ") continue;
      const loc = /^\s*location: '(.*)'$/.exec(lines[j]);
      if (loc) location = path.relative(root, loc[1].replace(/:\d+:\d+$/, "")).split(path.sep).join("/") + (/(:\d+:\d+)$/.exec(loc[1])?.[1] ?? "");
      const err = /^(\s*)error: (\|-)?(.*)$/.exec(lines[j]);
      if (err) {
        if (err[2]) {
          const indent = err[1].length + 2;
          const body: string[] = [];
          for (let k = j + 1; k < lines.length && (lines[k].startsWith(" ".repeat(indent)) || lines[k].trim() === ""); k++) body.push(lines[k].slice(indent));
          error = body.join("\n").trim();
        } else error = err[3].replace(/^'|'$/g, "");
      }
    }
    // Suites report their failed subtests again; keep the leaves (with an error).
    if (error || location) report.failures.push({ name: bad[2], error: error.slice(0, 800), location });
  }
  report.failures = report.failures.slice(0, 20);
  return report;
}

/**
 * The player's own package checks (packages.validateArchive): manifest, the
 * entry's JavaScript and every .slint file compiled. Rejects with the
 * player's error (`code`, message) when the package fails them.
 */
export type Validator = (file: string) => Promise<unknown>;

export class Toolchain {
  readonly packageDir: string;
  readonly dataDir: string;
  readonly vendor: string;
  readonly sdk: string;
  /** Without one (the eval runner has no player), check stops at the build. */
  private readonly validate: Validator | undefined;
  private mirroring: Promise<void> | undefined;
  /** One tool at a time per project (a build rewrites its dist/). */
  private readonly locks = new Map<string, Promise<unknown>>();

  constructor(packageDir: string, dataDir: string, validate?: Validator) {
    this.packageDir = packageDir;
    this.dataDir = dataDir;
    this.validate = validate;
    this.vendor = path.join(packageDir, "vendor");
    this.sdk = path.join(this.vendor, "@vestra-player", "sdk");
  }

  private version: string | undefined;
  /** The vendored SDK's version (the player's own, see tests/vendor-sdk.mjs). */
  get sdkVersion(): string {
    return (this.version ??= (JSON.parse(readFileSync(path.join(this.sdk, "package.json"), "utf8")) as { version: string }).version);
  }

  /** The read-only folders the agent may read: the SDK package (its .d.ts and Slint library; the documentation is read with the docs tools). */
  docsRoots(): string[] {
    return [this.sdk];
  }

  /** <data>/node_modules: the SDK and its types where the projects resolve them. */
  mirror(): Promise<void> {
    return (this.mirroring ??= (async () => {
      const root = path.join(this.dataDir, "node_modules");
      const stamp = path.join(root, "vestra-ai-toolchain.json");
      const want = JSON.stringify({ sdk: this.sdkVersion, digest: await digest(path.join(this.sdk, "package.json")) });
      if (existsSync(stamp) && (await fs.readFile(stamp, "utf8")) === want) return;
      await fs.rm(root, { recursive: true, force: true });
      for (const [from, to] of [
        [this.sdk, "@vestra-player/sdk"],
        [path.join(this.vendor, "@types/node"), "@types/node"],
        [path.join(this.vendor, "undici-types"), "undici-types"],
      ])
        // The CLI (bin/) stays in vendor/: it runs from there.
        await fs.cp(from, path.join(root, to), { recursive: true, filter: (file) => !/^bin($|[\\/])/.test(path.relative(from, file)) });
      await fs.writeFile(stamp, want);
    })().catch((error) => {
      this.mirroring = undefined;
      throw error;
    }));
  }

  /** The project folder of `id`, which must exist. */
  async project(session: SessionPaths, id: string): Promise<string> {
    if (typeof id !== "string" || !ID.test(id) || id.length > 128) throw new Error(`Not a plugin id: ${id} (it looks like <owner>.<name>)`);
    const directory = path.join(session.work, id);
    const stat = await fs.lstat(directory).catch(() => null);
    if (!stat?.isDirectory() || stat.isSymbolicLink()) throw new Error(`No plugin project work/${id}/ (create it with scaffold_plugin first)`);
    if (!existsSync(path.join(directory, "manifest.json"))) throw new Error(`work/${id}/ has no manifest.json`);
    // Real paths: tsc reports files relative to the real working directory,
    // and the permission model compares real paths (macOS: /var is /private/var).
    return fs.realpath(directory);
  }

  /** The projects of a session (folders of work/ with a manifest), newest first. */
  async projects(session: SessionPaths): Promise<Project[]> {
    let names: string[];
    try {
      names = await fs.readdir(session.work);
    } catch {
      return [];
    }
    const out: Project[] = [];
    for (const name of names.filter((n) => ID.test(n))) {
      const directory = path.join(session.work, name);
      try {
        const manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
        out.push({
          id: name,
          name: typeof manifest.name === "string" ? manifest.name : name,
          version: typeof manifest.version === "string" ? manifest.version : "0.0.0",
          directory,
          modified: await newest(directory),
        });
      } catch {
        // not (yet) a readable manifest
      }
    }
    return out.sort((a, b) => b.modified - a.modified);
  }

  private async locked<T>(key: string, work: () => Promise<T>): Promise<T> {
    const before = this.locks.get(key) ?? Promise.resolve();
    const next = before.catch(() => {}).then(work);
    this.locks.set(key, next);
    try {
      return await next;
    } finally {
      if (this.locks.get(key) === next) this.locks.delete(key);
    }
  }

  /** Removes the session's scratch packages a check left behind (a crash, a locked file); never throws. */
  async sweep(session: SessionPaths): Promise<void> {
    const tmp = path.join(session.dir, "tmp");
    const names = await fs.readdir(tmp).catch(() => [] as string[]);
    await Promise.all(names.filter((name) => name.startsWith("check-")).map((name) => fs.rm(path.join(tmp, name), { recursive: true, force: true }).catch(() => {})));
  }

  /** Strict settings at the session level, a fresh tmp/ for one call. */
  private async prepare(session: SessionPaths): Promise<string> {
    await this.mirror();
    const config = path.join(session.dir, "tsconfig.json");
    if (!existsSync(config)) await fs.writeFile(config, `${json(TSCONFIG)}\n`);
    const tmp = path.join(session.dir, "tmp", randomBytes(4).toString("hex"));
    await fs.mkdir(tmp, { recursive: true });
    return fs.realpath(tmp);
  }

  private cliEnvironment(tmp: string): Record<string, string> {
    // NODE_PATH: the CLI finds TypeScript and @types/node in vendor/ (see tests/vendor-sdk.mjs).
    return { ...baseEnvironment(), NODE_PATH: this.vendor, TMPDIR: tmp, TMP: tmp, TEMP: tmp, NO_COLOR: "1", FORCE_COLOR: "0" };
  }

  /**
   * The type check (`vestra-plugin check`) runs under Node's permission model:
   * it reads this plugin's package (vendor/ holds the CLI and its TypeScript;
   * Node's resolver also reads the node_modules/ it meets on the way up to
   * NODE_PATH, which a development checkout has), the SDK copy, the project
   * and the session's tsconfig.json, and writes only its scratch folder; no
   * network, workers, addons or WASI. Child processes are allowed because the
   * type checker is one (`tsc`, this Node), and it inherits the same flags
   * through NODE_OPTIONS (see cli()). The only other child the CLI starts is `npm ci` for
   * dependencies, which packageProblems() refuses before.
   *
   * build and pack do not, yet: they copy files with fs.cp, which under the
   * permission model stats every folder above the destination up to the
   * file system root (Node's checkParentPaths) for a file in a subfolder
   * (ui/main.slint), and fails. They run the same check first, on the same
   * fixed arguments.
   */
  private async confined(directory: string, session: SessionPaths, tmp: string): Promise<string[]> {
    const read = [await fs.realpath(this.packageDir), await fs.realpath(path.join(this.dataDir, "node_modules")), directory, tmp];
    const config = path.join(await fs.realpath(session.dir), "tsconfig.json");
    if (existsSync(config)) read.push(config);
    return ["--permission", ...read.map((p) => `--allow-fs-read=${p}`), `--allow-fs-write=${tmp}`, "--allow-child-process"];
  }

  private async cli(command: string, directory: string, session: SessionPaths, extra: string[], signal?: AbortSignal): Promise<RunResult> {
    const tmp = await this.prepare(session);
    try {
      // Trusted code on fixed arguments; it never runs the project's code
      // (bundling and npm installs are refused before, see packageProblems()).
      const permissions = command === "check" ? await this.confined(directory, session, tmp) : [];
      // ES module imports in the SDK find vendor/ too (see worker/vendor-resolve.ts).
      const resolver = pathToFileURL(await fs.realpath(path.join(this.packageDir, "worker", "vendor-resolve.ts"))).href;
      // The flags go in NODE_OPTIONS, each quoted. Given as arguments, Node
      // hands the permission flags on to the type checker child in a
      // NODE_OPTIONS it writes itself, unquoted: a space in a path
      // ("C:\Program Files\Vestra Player\…", a user name) splits the
      // --allow-fs-read and tsc may not read its own files. A NODE_OPTIONS
      // that is set is passed on as it is.
      const env = { ...this.cliEnvironment(tmp), NODE_OPTIONS: [...permissions, `--import=${resolver}`].map(nodeOption).join(" ") };
      return await runNode([path.join(this.sdk, "bin", "vestra-plugin.mjs"), command, directory, ...extra], {
        cwd: directory,
        env,
        timeoutMs: TIMEOUTS[command as keyof typeof TIMEOUTS] ?? 120_000,
        signal,
      });
    } finally {
      await fs.rm(tmp, { recursive: true, force: true });
    }
  }

  /** What the CLI said, for the model: type errors, manifest problems, hints. */
  private cliReport(run: RunResult, directory: string) {
    const all = `${run.stdout}\n${run.stderr}`;
    const errors = parseTsc(all).map((d) => ({ ...d, file: path.isAbsolute(d.file) ? path.relative(directory, d.file).split(path.sep).join("/") : d.file }));
    const problems = all
      .split(/\r?\n/)
      .filter((line) => line.startsWith("✖ "))
      .map((line) => line.slice(2).replace(/^[^:]+: /, ""))
      .filter((line) => !/^type check failed$/.test(line));
    const hints = all
      .split(/\r?\n/)
      .map((line) => /: hint: (.*)$/.exec(line)?.[1])
      .filter((x): x is string => !!x);
    const report: {
      errors: typeof errors;
      errorCount: number;
      problems: string[];
      hints: string[];
      exitCode?: number | null;
      stderr?: string;
      stdout?: string;
    } = {
      errors: errors.slice(0, 50),
      errorCount: errors.length,
      problems,
      hints,
    };
    // A failure the CLI did not explain in its usual lines (it crashed,
    // could not start, or printed something else): never an empty "failed".
    if (run.code !== 0 && !errors.length && !problems.length) {
      report.exitCode = run.code;
      report.stderr = tail(run.stderr) || "(empty)";
      if (run.stdout.trim()) report.stdout = tail(run.stdout);
    }
    return report;
  }

  /** `what`: the step, as in "Type check cancelled". */
  private stopped(run: RunResult, what: string): ToolResult | null {
    if (run.cancelled) return { ...result(false, `${what} cancelled`), say: { code: "cancelled", args: [what] } };
    if (run.timedOut) return { ...result(false, `${what} timed out and was stopped`), say: { code: "timed-out", args: [what] } };
    return null;
  }

  /** Workshop rules on top of the CLI's: id, dependencies, layout. */
  private async rules(directory: string, id: string, owner: string): Promise<{ errors: string[]; warnings: string[] }> {
    const errors: string[] = [];
    const warnings: string[] = [];
    let manifest: Record<string, unknown> = {};
    try {
      manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
    } catch (error) {
      errors.push(`manifest.json is not valid JSON: ${(error as Error).message}`);
    }
    if (manifest.id !== id) errors.push(`The id "${String(manifest.id)}" in manifest.json must match the folder name "${id}"`);
    if (/^vestra\./i.test(id)) errors.push('"vestra." is reserved for official plugins and cannot be used');
    const prefix = id.split(".")[0].toLowerCase();
    if (owner && prefix !== owner) warnings.push(`The id prefix "${prefix}" differs from the GitHub name "${owner}" in the settings; change it to ${owner}.<name> before publishing`);
    if (!owner && prefix === LOCAL_OWNER) warnings.push("The id starts with local.: for this computer only. To publish, enter a GitHub name in the settings, then change the id to <name>.<plugin>");
    errors.push(...(await packageProblems(directory)));
    errors.push(...(await slintProblems(directory)));
    if (!(await testFiles(directory)).length) warnings.push("No *.test.ts tests in tests/");
    return { errors, warnings };
  }

  async scaffold(session: SessionPaths, args: { name: string; title?: string }, owner: string): Promise<ToolResult> {
    const name = String(args?.name ?? "").trim();
    if (!NAME.test(name)) return result(false, 'name has only lower-case letters, digits and "-", starts with a letter, 2–39 characters, e.g. subtitle-delay');
    const title = String(args?.title ?? "").trim() || name;
    if ([...title].length > 40) return result(false, "title has at most 40 characters");
    const id = `${owner || LOCAL_OWNER}.${name}`;
    const directory = path.join(session.work, id);
    if (existsSync(directory)) return result(false, `work/${id}/ already exists; keep working on it, or choose another name`);
    await this.mirror();
    const files = template(id, name, title, this.sdkVersion);
    for (const [rel, text] of Object.entries(files)) {
      await fs.mkdir(path.dirname(path.join(directory, rel)), { recursive: true });
      await fs.writeFile(path.join(directory, rel), text);
    }
    return result(true, `Created the plugin project work/${id}/`, {
      id,
      folder: `work/${id}`,
      files: Object.keys(files),
      next: "Edit main.ts, ui/main.slint and tests/main.test.ts, then typecheck → test → check",
      ...(owner ? {} : { note: "No GitHub name in the settings: the id has the local. prefix (for this computer only)" }),
    });
  }

  typecheck(session: SessionPaths, id: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      // The CLI would start `npm ci` for dependencies.
      const problems = await packageProblems(directory);
      if (problems.length) return result(false, `Checks before the type check failed: ${problems.length} problem(s)`, { problems });
      const run = await this.cli("check", directory, session, [], signal);
      const stop = this.stopped(run, "Type check");
      if (stop) return stop;
      const report = this.cliReport(run, directory);
      if (run.code === 0) return result(true, `Type check passed (${/\((\d+) files?\)/.exec(run.stdout)?.[1] ?? "?"} files)`, report.hints.length ? { hints: report.hints } : undefined);
      const summary = report.errorCount ? `Type check failed: ${report.errorCount} error(s)` : `Check failed${unexplained(report)}`;
      return result(false, summary, report);
    });
  }

  test(session: SessionPaths, id: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const tests = await testFiles(directory);
      if (!tests.length) return result(false, "No tests: write *.test.ts in tests/ (startPlugin / TestHost from @vestra-player/sdk/testing)");
      const tmp = await this.prepare(session);
      try {
        const modules = await fs.realpath(path.join(this.dataDir, "node_modules"));
        // A fresh Node with the permission model: it reads the project and the
        // SDK copy, writes only its scratch folder; no network, no child
        // processes, workers, addons or WASI. One process (no isolation):
        // `node --test` would otherwise start a child per file.
        const run = await runNode(
          [
            "--permission",
            `--allow-fs-read=${directory}`,
            `--allow-fs-read=${modules}`,
            `--allow-fs-read=${tmp}`,
            `--allow-fs-write=${tmp}`,
            "--no-experimental-webstorage",
            "--test",
            "--test-isolation=none",
            "--test-reporter=tap",
            "--test-timeout=30000",
            ...tests,
          ],
          { cwd: directory, env: { ...baseEnvironment(), TMPDIR: tmp, TMP: tmp, TEMP: tmp, NO_COLOR: "1" }, timeoutMs: TIMEOUTS.test, signal },
        );
        const stop = this.stopped(run, "Tests");
        if (stop) return stop;
        const report = parseTap(run.stdout, directory);
        if (run.code === 0) return result(true, `All tests passed (${report.passed})`);
        if (!report.passed && !report.failed) {
          // Did not get to run: a syntax error, a missing module, a denied access.
          const stderr = run.stderr.trim().split("\n").slice(0, 30).join("\n");
          const tap = run.stdout.trim().split("\n").filter((l) => !/^# (tests|suites|pass|fail|cancelled|skipped|todo|duration_ms) /.test(l)).slice(0, 40).join("\n");
          return result(false, "The tests did not run", { stderr, output: tap });
        }
        const printed = testOutput(run.stdout, run.stderr);
        if (printed) report.output = printed;
        return result(false, `Tests failed: ${report.failed} failed, ${report.passed} passed`, report);
      } finally {
        await fs.rm(tmp, { recursive: true, force: true });
      }
    });
  }

  check(session: SessionPaths, id: string, owner: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const rules = await this.rules(directory, id, owner);
      if (rules.errors.length) return result(false, `Check failed: ${rules.errors.length} problem(s)`, { problems: rules.errors, warnings: rules.warnings });
      // build: manifest rules, strict types, transpiling, no runtime import of the SDK.
      // With the player at hand, pack (which builds) into a scratch folder,
      // and let the player check the package as an install would: the
      // compiler in the loop, so Slint errors reach the model before install.
      const scratch = this.validate ? path.join(session.dir, "tmp", `check-${randomBytes(4).toString("hex")}`) : undefined;
      try {
        const run = scratch ? await this.cli("pack", directory, session, ["--out", scratch], signal) : await this.cli("build", directory, session, [], signal);
        const stop = this.stopped(run, "Check");
        if (stop) return stop;
        const report = this.cliReport(run, directory);
        const warnings = [...rules.warnings, ...report.hints];
        if (run.code !== 0)
          return result(
            false,
            report.errorCount ? `Check failed: ${report.errorCount} type error(s)` : `Check failed${unexplained(report)}`,
            { ...report, warnings },
          );
        if (!scratch) return result(true, "Check passed (manifest, strict type check, build). The player compiles and checks Slint at install.", warnings.length ? { warnings } : undefined);
        const packed = (await fs.readdir(scratch).catch(() => [] as string[])).find((name) => name.endsWith(".vxp"));
        if (!packed) return result(false, "Check failed: the build wrote no package for the player to check", { output: tail(run.stdout) });
        try {
          // The player's reply cannot be withdrawn: on a stop, return at once and drop the reply when it comes.
          const validated = this.validate!(path.join(scratch, packed));
          validated.catch(() => {});
          if (await raceAbort(validated, signal)) return { ...result(false, "Check cancelled"), say: { code: "cancelled", args: ["Check"] } };
        } catch (error) {
          return result(false, playerRefusal(error), { problems: playerMessage(error).split("\n").filter(Boolean), warnings, next: PLAYER_NEXT });
        }
        return result(true, "Check passed (manifest, strict type check, build, the player's Slint compile)", warnings.length ? { warnings } : undefined);
      } finally {
        // A lingering handle (EBUSY on Windows) must not replace the check's result; sweep() removes it later.
        if (scratch) await fs.rm(scratch, { recursive: true, force: true }).catch(() => {});
      }
    });
  }

  /** Packs into sessions/<sid>/dist/; `file` is the .vxp on success. */
  pack(session: SessionPaths, id: string, owner: string, signal?: AbortSignal): Promise<ToolResult & { file?: string }> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const rules = await this.rules(directory, id, owner);
      if (rules.errors.length)
        return { ...result(false, `Checks before packing failed: ${rules.errors.length} problem(s)`, { problems: rules.errors }), say: { code: "pack-problems", args: [String(rules.errors.length)] } };
      const out = path.join(session.dir, "dist");
      await fs.mkdir(out, { recursive: true });
      const run = await this.cli("pack", directory, session, ["--out", out], signal);
      const stop = this.stopped(run, "Packing");
      if (stop) return stop;
      if (run.code !== 0) {
        const report = this.cliReport(run, directory);
        return report.errorCount
          ? { ...result(false, `Packing failed: ${report.errorCount} type error(s)`, report), say: { code: "pack-type-errors", args: [String(report.errorCount)] } }
          : { ...result(false, `Packing failed${unexplained(report)}`, report), say: { code: "pack-failed", args: [] } };
      }
      const packed = /packed (.+\.vxp) \((\d+) bytes, sha256 ([0-9a-f]{64})\)/.exec(run.stdout);
      const version = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8")).version as string;
      const file = path.join(out, `${id}-${version}.vxp`);
      if (!existsSync(file)) return { ...result(false, "Packing failed: no .vxp was written", { output: run.stdout.slice(-2000) }), say: { code: "pack-no-vxp", args: [] } };
      return { ...result(true, `Packed dist/${id}-${version}.vxp`, { file: `dist/${id}-${version}.vxp`, bytes: Number(packed?.[2] ?? 0), sha256: packed?.[3] ?? "" }), file };
    });
  }

  /**
   * The project as the user's own repository, zipped under `<name>/`: its
   * files (no node_modules, dist, hidden names), and what a repository needs
   * that the session leaves out: package.json with the SDK as dev dependency,
   * tsconfig.json, .gitignore, a CI workflow, VS Code settings (the
   * workspace's TypeScript, for the SDK's editor plugin), README/CHANGELOG/
   * LICENSE placeholders where missing.
   */
  async exportSource(session: SessionPaths, id: string): Promise<{ data: Buffer; fileName: string; local: boolean }> {
    const directory = await this.project(session, id);
    const manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
    const name = id.split(".").slice(1).join(".") || id;
    const files = new Map<string, Buffer>();
    for (const rel of await projectFiles(directory)) files.set(rel, await fs.readFile(path.join(directory, rel)));
    const text = (rel: string) => files.get(rel)?.toString("utf8");
    let pkg: Record<string, unknown> = {};
    try {
      pkg = JSON.parse(text("package.json") ?? "{}");
    } catch {
      pkg = {};
    }
    const version = this.sdkVersion;
    files.set(
      "package.json",
      Buffer.from(
        `${JSON.stringify(
          {
            name: typeof pkg.name === "string" ? pkg.name : `vestra-plugin-${name}`,
            version: manifest.version ?? "0.1.0",
            private: true,
            type: "module",
            scripts: { check: "vestra-plugin check", test: "vestra-plugin test", build: "vestra-plugin build", pack: "vestra-plugin pack" },
            ...pkg,
            devDependencies: { ...(pkg.devDependencies as object), "@vestra-player/sdk": `^${version}` },
          },
          null,
          2,
        )}\n`,
      ),
    );
    if (!files.has("tsconfig.json")) files.set("tsconfig.json", Buffer.from(`${JSON.stringify(TSCONFIG, null, 2)}\n`));
    files.set(".gitignore", Buffer.from("node_modules/\ndist/\n*.vxp\n"));
    files.set(".github/workflows/ci.yml", Buffer.from(CI_WORKFLOW));
    files.set(".vscode/settings.json", Buffer.from(`${JSON.stringify(VSCODE_SETTINGS, null, 2)}\n`));
    if (!files.has("LICENSE")) files.set("LICENSE", Buffer.from(LICENSE_PLACEHOLDER));
    if (!files.has("CHANGELOG.md")) files.set("CHANGELOG.md", Buffer.from(`# Changelog\n\n## ${manifest.version ?? "0.1.0"}\n\n- First version.\n`));
    if (!files.has("README.md")) files.set("README.md", Buffer.from(`# ${manifest.name ?? name}\n\nA Vestra Player plugin.\n`));
    const entries: ZipEntry[] = [...files.keys()].sort().map((rel) => ({ name: `${name}/${rel}`, data: files.get(rel)! }));
    // The window tells the user what to change before publishing (ai.slint).
    return { data: zip(entries), fileName: `${id}-${manifest.version ?? "0.1.0"}-source.zip`, local: id.startsWith(`${LOCAL_OWNER}.`) };
  }
}

async function digest(file: string): Promise<string> {
  return createHash("sha256").update(await fs.readFile(file)).digest("hex");
}

/** The latest modification time of the files in `directory` (not node_modules, dist). */
async function newest(directory: string): Promise<number> {
  let latest = 0;
  for (const rel of await projectFiles(directory)) latest = Math.max(latest, (await fs.stat(path.join(directory, rel))).mtimeMs);
  return latest;
}

/** The project's own files, relative and "/"-separated: no node_modules, dist, hidden names or links. */
async function projectFiles(directory: string): Promise<string[]> {
  const out: string[] = [];
  const visit = async (rel: string) => {
    for (const item of await fs.readdir(path.join(directory, rel), { withFileTypes: true })) {
      if (item.name.startsWith(".") || item.isSymbolicLink()) continue;
      const child = rel ? `${rel}/${item.name}` : item.name;
      if (item.isDirectory()) {
        if (!["node_modules", "dist"].includes(item.name)) await visit(child);
      } else if (item.isFile()) out.push(child);
      if (out.length > 2000) throw new Error("Too many project files (more than 2000)");
    }
  };
  await visit("");
  return out.sort();
}

async function testFiles(directory: string): Promise<string[]> {
  try {
    return (await fs.readdir(path.join(directory, "tests")))
      .filter((file) => /\.test\.(ts|mjs)$/.test(file))
      .sort()
      .map((file) => path.join("tests", file));
  } catch {
    return [];
  }
}

/**
 * Slint types a plugin cannot write from TypeScript: a property value is
 * JSON (booleans, numbers, strings, arrays, structs) plus images and
 * Markdown, and the player refuses a string for a `brush` or `color`
 * declaration at run time ("high-tint: wrong type: high-tint is a string
 * where the declaration has a brush"), after every check has passed: the
 * TestHost has no Slint and the player's compile sees no values. So the
 * check reads the declarations itself.
 */
const UNWRITABLE_SLINT_TYPES = new Set(["brush", "color"]);

/**
 * `in` and `in-out` properties of an exported component or global whose
 * Slint type the plugin cannot write (UNWRITABLE_SLINT_TYPES), one problem
 * per declaration with its file and line. Only the exported component and
 * global are the plugin's side of a window, panel or settings page; a helper
 * component may well take a brush from the Slint code that uses it.
 * `library`: the file is imported by another of the project's .slint files,
 * so its exported components are helpers (Slint needs `export` to import a
 * component from another file, as in a `ui/parts.slint`) and are skipped;
 * its globals are still checked.
 * Not caught here: a colour inside an array or a struct property (a model of
 * rows with a `brush` field). The TestHost (SDK 1.7.3) refuses such a value
 * against the .slint declaration when the plugin's tests write it.
 */
export function slintDeclarationProblems(file: string, source: string, { library = false }: { library?: boolean } = {}): string[] {
  // Comments and string literals become spaces of the same length, so
  // offsets and line numbers stay those of the source.
  const blank = (text: string) => text.replace(/[^\n]/g, " ");
  const stripped = source.replace(/\/\*[\s\S]*?\*\/|\/\/[^\n]*|"(?:[^"\\\n]|\\.)*"/g, blank);
  const line = (offset: number) => stripped.slice(0, offset).split("\n").length;
  const problems: string[] = [];
  const exported = /\bexport\s+(component|global)\s+([\w-]+)[^{]*\{/g;
  for (let block = exported.exec(stripped); block; block = exported.exec(stripped)) {
    const skip = library && block[1] === "component";
    // The block's body: from its opening brace to the matching closing one.
    let depth = 0;
    let start = block.index + block[0].length;
    let end = start;
    let at = start - 1; // the opening brace
    for (; at < stripped.length; at++) {
      const char = stripped[at];
      if (char === "{") depth++;
      else if (char === "}" && --depth === 0) break;
    }
    end = at;
    // Declarations at the body's own level (depth 1): a nested element's
    // properties are the Slint code's business.
    depth = 0;
    let declaration = "";
    for (let i = start; i < end; i++) {
      const char = stripped[i];
      if (char === "{") {
        depth++;
        declaration = "";
      } else if (char === "}") {
        depth--;
        declaration = "";
      } else if (depth === 0) {
        if (char === ";") {
          const found = /\b(in|in-out)\s+property\s*<\s*([\w-]+)\s*>\s*([\w-]+)/.exec(declaration);
          if (found && !skip && UNWRITABLE_SLINT_TYPES.has(found[2])) {
            const offset = i - declaration.length + (found.index ?? 0);
            problems.push(
              `${file}:${line(offset)}: ${found[1]} property <${found[2]}> ${found[3]} in ${block[1]} ${block[2]}: the player cannot write a ${found[2]} from a plugin (a property value is a boolean, a number, a string, an array, a struct, an image or Markdown; the window would fail with "wrong type" when shown). Declare a number or string instead and compute the ${found[2]} in Slint.`,
            );
          }
          declaration = "";
        } else declaration += char;
      }
    }
    exported.lastIndex = Math.max(exported.lastIndex, end);
  }
  return problems;
}

/**
 * The project's .slint files that another of its .slint files imports
 * (`import { … } from "./parts.slint"`, a path relative to the importing
 * file): libraries, not entry files. `@vestra/…` and `std-widgets.slint`
 * imports are the player's, not the project's.
 */
export function slintLibraries(sources: Map<string, string>): Set<string> {
  const libraries = new Set<string>();
  const imported = /\bimport\b[^;]*?\bfrom\s*"([^"]+)"|\bimport\s*"([^"]+)"/g;
  for (const [file, source] of sources) {
    const code = source.replace(/\/\*[\s\S]*?\*\/|\/\/[^\n]*/g, "");
    for (let found = imported.exec(code); found; found = imported.exec(code)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), found[1] ?? found[2]));
      if (target !== file && sources.has(target)) libraries.add(target);
    }
    imported.lastIndex = 0;
  }
  return libraries;
}

/**
 * Every .slint file of the project checked with slintDeclarationProblems,
 * the imported ones as libraries. projectFiles leaves out node_modules and
 * dist; vendor/ is a copy of someone else's Slint (the SDK's own
 * `icons.slint` exports `Icon` with an `in property <brush> color`).
 */
async function slintProblems(directory: string): Promise<string[]> {
  const sources = new Map<string, string>();
  for (const rel of (await projectFiles(directory)).filter((file) => file.endsWith(".slint") && !/(^|\/)vendor\//.test(file))) sources.set(rel, await fs.readFile(path.join(directory, rel), "utf8"));
  const libraries = slintLibraries(sources);
  const problems: string[] = [];
  for (const [rel, source] of sources) problems.push(...slintDeclarationProblems(rel, source, { library: libraries.has(rel) }));
  return problems;
}

/** package.json: no npm dependencies (the CLI would install them) and no bundling (no esbuild here). */
async function packageProblems(directory: string): Promise<string[]> {
  if (!existsSync(path.join(directory, "package.json"))) return [];
  try {
    const pkg = JSON.parse(await fs.readFile(path.join(directory, "package.json"), "utf8"));
    const problems: string[] = [];
    const deps = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies, ...pkg.optionalDependencies, ...pkg.peerDependencies });
    if (deps.length) problems.push(`No dependencies in package.json (${deps.join(", ")}): only Node's built-in modules and the plugin SDK can be used here; the source export adds @vestra-player/sdk itself`);
    if (pkg.vestra?.bundle) problems.push("vestra.bundle in package.json is not available (no esbuild)");
    return problems;
  } catch (error) {
    return [`package.json is not valid JSON: ${(error as Error).message}`];
  }
}

const TSCONFIG = {
  compilerOptions: {
    target: "ES2023",
    module: "NodeNext",
    moduleResolution: "NodeNext",
    strict: true,
    noEmit: true,
    allowImportingTsExtensions: true,
    verbatimModuleSyntax: true,
    erasableSyntaxOnly: true,
    skipLibCheck: true,
    types: ["node"],
  },
  include: ["*.ts", "tests/*.ts"],
};

/**
 * VS Code settings for the exported repository: the editor uses the project's
 * own TypeScript, the version the SDK's `vestra-plugin check` uses, which
 * `npm install` puts in node_modules/ as a dependency of @vestra-player/sdk. Not in the session's project: it has no node_modules/
 * of its own, and nothing opens it in an editor.
 */
const VSCODE_SETTINGS = { "typescript.tsdk": "node_modules/typescript/lib", "typescript.enablePromptUseWorkspaceTsdk": true };

/** The starting point of a new project: the official repository's layout, one window and its command, one test. */
export function template(id: string, name: string, title: string, sdkVersion: string): Record<string, string> {
  const manifest = {
    id,
    version: "0.1.0",
    apiVersion: "vestra/1",
    entry: "main.ts",
    name: title,
    // A player without manifest declarations (SDK 1.9.0): an older one requires them.
    minPlayerVersion: "0.9.0",
  };
  return {
    "manifest.json": `${JSON.stringify(manifest, null, 2)}\n`,
    "main.ts": `import type { Plugin } from "@vestra-player/sdk";

/** ${title}: a menu command opens a tool window. */
export default async function activate(plugin: Plugin) {
  const mainWindow = plugin.windows.create("ui/main.slint");
  mainWindow.bind(() => ({ message: "Hello" }));

  plugin.commands.add({
    id: "${name}.open",
    path: ["${title}…"],
    run: () => mainWindow.show(),
  });
}
`,
    "ui/main.slint": `import { VerticalBox } from "std-widgets.slint";

export component MainWindow inherits Window {
    title: "${title}";
    preferred-width: 320px;
    in property <string> message;
    VerticalBox {
        Text { text: root.message; }
    }
}
`,
    "tests/main.test.ts": `import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

test("the menu command opens the window; idle, nothing keeps the plugin running", async () => {
  const { host, stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url) });
  // Idle: the player may stop the plugin and start it again on its next use.
  assert.deepEqual(host.keepsRunning, []);
  await host.command("${name}.open").run();
  assert.equal(host.window("main").props.message, "Hello");
  await stop();
});
`,
    "README.md": `# ${title}

A Vestra Player plugin (id \`${id}\`).

## Usage

The menu item "${title}…" opens the window.

## Development

\`\`\`sh
npm install
npm run check && npm test && npm run pack
\`\`\`

Try it in the player: \`--plugin <this folder>\`. Plugin SDK: @vestra-player/sdk ${sdkVersion}.
`,
    "CHANGELOG.md": `# Changelog\n\n## 0.1.0\n\n- First version.\n`,
    "package.json": `${JSON.stringify({ name: `vestra-plugin-${name}`, version: "0.1.0", private: true, type: "module" }, null, 2)}\n`,
    "tsconfig.json": `${JSON.stringify(TSCONFIG, null, 2)}\n`,
  };
}

const CI_WORKFLOW = `# Check, test and pack the plugin on every push (the SDK's vestra-plugin CLI).
name: CI
on:
  push:
  pull_request:
permissions:
  contents: read
jobs:
  plugin:
    strategy:
      matrix:
        os: [macos-latest, windows-latest]
    runs-on: \${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
        with:
          persist-credentials: false
      - uses: actions/setup-node@v4
        with:
          node-version: 26
      - run: npm install
      - run: npx vestra-plugin check
      - run: npx vestra-plugin test
      - run: npx vestra-plugin pack --out dist
      - uses: actions/upload-artifact@v4
        if: matrix.os == 'macos-latest'
        with:
          name: plugin
          path: dist/*.vxp
`;

const LICENSE_PLACEHOLDER = `TODO: choose a license for this plugin and put its full text here.

The official plugin repository builds third-party plugins from public source
under a license that allows building and distributing them (for example MIT or
Apache-2.0). Also set "license" in manifest.json.
`;
