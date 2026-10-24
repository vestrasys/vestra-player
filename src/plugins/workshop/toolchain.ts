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
import { baseEnvironment, runNode, type RunResult } from "./run.ts";
import { zip, type ZipEntry } from "./zip.ts";

export interface ToolResult {
  ok: boolean;
  /** For the model: a one-line summary, then details (JSON). */
  text: string;
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
  capabilities: string[];
  modified: number;
}

/** Owner prefix when the user set no GitHub name: local only, never published as is. */
export const LOCAL_OWNER = "local";
const OWNER = /^[a-z0-9](?:[a-z0-9-]{0,38})$/;
const NAME = /^[a-z][a-z0-9-]{1,38}$/;
const ID = /^[A-Za-z0-9][A-Za-z0-9-]*\.[A-Za-z0-9][A-Za-z0-9._-]*$/;

/** A GitHub user or organization name usable as id prefix, lower case; "" when unset; throws when invalid. */
export function checkOwner(text: string): string {
  const owner = text.trim().toLowerCase();
  if (!owner) return "";
  if (!OWNER.test(owner)) throw new Error("GitHub 用户名只能有字母、数字和「-」，最多 39 个字符");
  if (owner === "vestra") throw new Error("「vestra」留给官方插件，请填你自己的 GitHub 用户名");
  if (owner === LOCAL_OWNER) throw new Error("「local」表示没有填用户名，请填你自己的 GitHub 用户名");
  return owner;
}

export const TIMEOUTS = { typecheck: 90_000, test: 90_000, check: 120_000, pack: 120_000 };
/** Text returned to the model at most per call. */
const MAX_RESULT = 12_000;

const json = (value: unknown) => JSON.stringify(value, null, 1);
const result = (ok: boolean, summary: string, details?: unknown): ToolResult => {
  let text = details === undefined ? summary : `${summary}\n${json(details)}`;
  if (text.length > MAX_RESULT) text = `${text.slice(0, MAX_RESULT)}\n…（输出过长，已截断）`;
  return { ok, text };
};

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
  else if (lines.length > MAX_OUTPUT.lines) text += `\n…（还有 ${lines.length - MAX_OUTPUT.lines} 行）`;
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

export class Toolchain {
  readonly packageDir: string;
  readonly dataDir: string;
  readonly vendor: string;
  readonly sdk: string;
  private mirroring: Promise<void> | undefined;
  /** One tool at a time per project (a build rewrites its dist/). */
  private readonly locks = new Map<string, Promise<unknown>>();

  constructor(packageDir: string, dataDir: string) {
    this.packageDir = packageDir;
    this.dataDir = dataDir;
    this.vendor = path.join(packageDir, "vendor");
    this.sdk = path.join(this.vendor, "@vestra-player", "sdk");
  }

  private version: string | undefined;
  /** The vendored SDK's version (the player's own, see tests/vendor-sdk.mjs). */
  get sdkVersion(): string {
    return (this.version ??= (JSON.parse(readFileSync(path.join(this.sdk, "package.json"), "utf8")) as { version: string }).version);
  }

  /** The read-only folders the agent may read: the SDK package, which carries the reference. */
  docsRoots(): string[] {
    return [this.sdk];
  }

  /** The SDK's llms.txt (llms-full.txt is next to it). */
  docsIndex(): string {
    return path.join(this.sdk, "llms.txt");
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
    if (typeof id !== "string" || !ID.test(id) || id.length > 128) throw new Error(`不是插件 id：${id}（形如 <owner>.<name>）`);
    const directory = path.join(session.work, id);
    const stat = await fs.lstat(directory).catch(() => null);
    if (!stat?.isDirectory() || stat.isSymbolicLink()) throw new Error(`没有插件项目 work/${id}/（先用 scaffold_plugin 创建）`);
    if (!existsSync(path.join(directory, "manifest.json"))) throw new Error(`work/${id}/ 里没有 manifest.json`);
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
          capabilities: Array.isArray(manifest.capabilities) ? manifest.capabilities.map(String) : [],
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
   * type checker is one (`tsc`, this Node), and Node passes the same flags on
   * to it (NODE_OPTIONS). The only other child the CLI starts is `npm ci` for
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
      return await runNode([...permissions, path.join(this.sdk, "bin", "vestra-plugin.mjs"), command, directory, ...extra], {
        cwd: directory,
        env: this.cliEnvironment(tmp),
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
    return { errors: errors.slice(0, 50), errorCount: errors.length, problems, hints };
  }

  private stopped(run: RunResult, what: string): ToolResult | null {
    if (run.cancelled) return result(false, `${what}已取消`);
    if (run.timedOut) return result(false, `${what}超时，已停止`);
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
      errors.push(`manifest.json 不是有效的 JSON：${(error as Error).message}`);
    }
    if (manifest.id !== id) errors.push(`manifest.json 的 id「${String(manifest.id)}」要和文件夹名「${id}」一致`);
    if (/^vestra\./i.test(id)) errors.push("「vestra.」留给官方插件，不能使用");
    const prefix = id.split(".")[0].toLowerCase();
    if (owner && prefix !== owner) warnings.push(`id 前缀「${prefix}」和设置里的 GitHub 用户名「${owner}」不一致；发布前改成 ${owner}.<名字>`);
    if (!owner && prefix === LOCAL_OWNER) warnings.push("id 以 local. 开头：只适合在本机用。要发布，先在设置里填 GitHub 用户名，再把 id 改成 <用户名>.<名字>");
    errors.push(...(await packageProblems(directory)));
    if (!(await testFiles(directory)).length) warnings.push("tests/ 里没有 *.test.ts 测试");
    return { errors, warnings };
  }

  async scaffold(session: SessionPaths, args: { name: string; title?: string }, owner: string): Promise<ToolResult> {
    const name = String(args?.name ?? "").trim();
    if (!NAME.test(name)) return result(false, "name 只能用小写字母、数字和「-」，以字母开头，2–39 个字符，例如 subtitle-delay");
    const title = String(args?.title ?? "").trim() || name;
    if ([...title].length > 40) return result(false, "title 最多 40 个字符");
    const id = `${owner || LOCAL_OWNER}.${name}`;
    const directory = path.join(session.work, id);
    if (existsSync(directory)) return result(false, `work/${id}/ 已存在；继续修改它，或换一个 name`);
    await this.mirror();
    const files = template(id, name, title, this.sdkVersion);
    for (const [rel, text] of Object.entries(files)) {
      await fs.mkdir(path.dirname(path.join(directory, rel)), { recursive: true });
      await fs.writeFile(path.join(directory, rel), text);
    }
    return result(true, `已创建插件项目 work/${id}/`, {
      id,
      folder: `work/${id}`,
      files: Object.keys(files),
      next: "改 manifest.json 的 capabilities、main.ts、ui/main.slint 和 tests/main.test.ts，然后 typecheck → test → check",
      ...(owner ? {} : { note: "设置里没有 GitHub 用户名，id 用了 local. 前缀（只适合本机）" }),
    });
  }

  typecheck(session: SessionPaths, id: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      // The CLI would start `npm ci` for dependencies.
      const problems = await packageProblems(directory);
      if (problems.length) return result(false, `类型检查前的检查未通过：${problems.length} 个问题`, { problems });
      const run = await this.cli("check", directory, session, [], signal);
      const stop = this.stopped(run, "类型检查");
      if (stop) return stop;
      const report = this.cliReport(run, directory);
      if (run.code === 0) return result(true, `类型检查通过（${/\((\d+) files?\)/.exec(run.stdout)?.[1] ?? "?"} 个文件）`, report.hints.length ? { hints: report.hints } : undefined);
      const summary = report.errorCount ? `类型检查未通过：${report.errorCount} 个错误` : "检查未通过";
      return result(false, summary, report);
    });
  }

  test(session: SessionPaths, id: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const tests = await testFiles(directory);
      if (!tests.length) return result(false, "没有测试：在 tests/ 里写 *.test.ts（用 @vestra-player/sdk/testing 的 startPlugin / TestHost）");
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
        const stop = this.stopped(run, "测试");
        if (stop) return stop;
        const report = parseTap(run.stdout, directory);
        if (run.code === 0) return result(true, `测试全部通过（${report.passed} 个）`);
        if (!report.passed && !report.failed) {
          // Did not get to run: a syntax error, a missing module, a denied access.
          const stderr = run.stderr.trim().split("\n").slice(0, 30).join("\n");
          const tap = run.stdout.trim().split("\n").filter((l) => !/^# (tests|suites|pass|fail|cancelled|skipped|todo|duration_ms) /.test(l)).slice(0, 40).join("\n");
          return result(false, "测试没能运行", { stderr, output: tap });
        }
        const printed = testOutput(run.stdout, run.stderr);
        if (printed) report.output = printed;
        return result(false, `测试未通过：${report.failed} 个失败，${report.passed} 个通过`, report);
      } finally {
        await fs.rm(tmp, { recursive: true, force: true });
      }
    });
  }

  check(session: SessionPaths, id: string, owner: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const rules = await this.rules(directory, id, owner);
      if (rules.errors.length) return result(false, `检查未通过：${rules.errors.length} 个问题`, { problems: rules.errors, warnings: rules.warnings });
      // build: manifest rules, strict types, transpiling, no runtime import of the SDK.
      const run = await this.cli("build", directory, session, [], signal);
      const stop = this.stopped(run, "检查");
      if (stop) return stop;
      const report = this.cliReport(run, directory);
      const warnings = [...rules.warnings, ...report.hints];
      if (run.code !== 0) return result(false, report.errorCount ? `检查未通过：${report.errorCount} 个类型错误` : "检查未通过", { ...report, warnings });
      return result(true, "检查通过（manifest、严格类型检查、构建）。Slint 由播放器在安装时编译检查。", warnings.length ? { warnings } : undefined);
    });
  }

  /** Packs into sessions/<sid>/dist/; `file` is the .vxp on success. */
  pack(session: SessionPaths, id: string, owner: string, signal?: AbortSignal): Promise<ToolResult & { file?: string }> {
    return this.locked(id, async () => {
      const directory = await this.project(session, id);
      const rules = await this.rules(directory, id, owner);
      if (rules.errors.length) return result(false, `打包前的检查未通过：${rules.errors.length} 个问题`, { problems: rules.errors });
      const out = path.join(session.dir, "dist");
      await fs.mkdir(out, { recursive: true });
      const run = await this.cli("pack", directory, session, ["--out", out], signal);
      const stop = this.stopped(run, "打包");
      if (stop) return stop;
      if (run.code !== 0) {
        const report = this.cliReport(run, directory);
        return result(false, report.errorCount ? `打包失败：${report.errorCount} 个类型错误` : "打包失败", report);
      }
      const packed = /packed (.+\.vxp) \((\d+) bytes, sha256 ([0-9a-f]{64})\)/.exec(run.stdout);
      const version = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8")).version as string;
      const file = path.join(out, `${id}-${version}.vxp`);
      if (!existsSync(file)) return result(false, "打包失败：没有生成 .vxp", { output: run.stdout.slice(-2000) });
      return { ...result(true, `已打包 dist/${id}-${version}.vxp`, { file: `dist/${id}-${version}.vxp`, bytes: Number(packed?.[2] ?? 0), sha256: packed?.[3] ?? "" }), file };
    });
  }

  /**
   * The project as the user's own repository, zipped under `<name>/`: its
   * files (no node_modules, dist, hidden names), and what a repository needs
   * that the session leaves out: package.json with the SDK as dev dependency,
   * tsconfig.json, .gitignore, a CI workflow, README/CHANGELOG/LICENSE
   * placeholders where missing.
   */
  async exportSource(session: SessionPaths, id: string): Promise<{ data: Buffer; fileName: string; notes: string[] }> {
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
    if (!files.has("LICENSE")) files.set("LICENSE", Buffer.from(LICENSE_PLACEHOLDER));
    if (!files.has("CHANGELOG.md")) files.set("CHANGELOG.md", Buffer.from(`# Changelog\n\n## ${manifest.version ?? "0.1.0"}\n\n- 第一版。\n`));
    if (!files.has("README.md")) files.set("README.md", Buffer.from(`# ${manifest.name ?? name}\n\n一个 Vestra Player 插件。\n`));
    const notes: string[] = [];
    if (id.startsWith(`${LOCAL_OWNER}.`)) notes.push("id 以 local. 开头：发布前把 manifest.json 的 id 改成 <你的 GitHub 用户名>.<名字>");
    notes.push("LICENSE 是占位文件：发布前换成你选的许可证全文，并在 manifest.json 填 license");
    const entries: ZipEntry[] = [...files.keys()].sort().map((rel) => ({ name: `${name}/${rel}`, data: files.get(rel)! }));
    return { data: zip(entries), fileName: `${id}-${manifest.version ?? "0.1.0"}-source.zip`, notes };
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
      if (out.length > 2000) throw new Error("项目文件太多（超过 2000 个）");
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

/** package.json: no npm dependencies (the CLI would install them) and no bundling (no esbuild here). */
async function packageProblems(directory: string): Promise<string[]> {
  if (!existsSync(path.join(directory, "package.json"))) return [];
  try {
    const pkg = JSON.parse(await fs.readFile(path.join(directory, "package.json"), "utf8"));
    const problems: string[] = [];
    const deps = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies, ...pkg.optionalDependencies, ...pkg.peerDependencies });
    if (deps.length) problems.push(`package.json 里不要写依赖（${deps.join("、")}）：这里只能用 Node 内置模块和插件 SDK；导出源码时会自动加上 @vestra-player/sdk`);
    if (pkg.vestra?.bundle) problems.push("package.json 的 vestra.bundle 不可用（没有 esbuild）");
    return problems;
  } catch (error) {
    return [`package.json 不是有效的 JSON：${(error as Error).message}`];
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

/** The starting point of a new project: the official repository's layout, one window and its command, one test. */
export function template(id: string, name: string, title: string, sdkVersion: string): Record<string, string> {
  const manifest = {
    id,
    version: "0.1.0",
    apiVersion: "vestra/1",
    entry: "main.ts",
    capabilities: ["commands", "ui.window"],
    name: title,
    minPlayerVersion: "0.4.1",
  };
  return {
    "manifest.json": `${JSON.stringify(manifest, null, 2)}\n`,
    "main.ts": `import type { Plugin } from "@vestra-player/sdk";

/** ${title}: a menu command opens a tool window. */
export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("ui/main.slint");
  window.bind(() => ({ message: "你好" }));

  plugin.commands.add({
    id: "${name}.open",
    path: ["${title}…"],
    run: () => window.show(),
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

test("the menu command opens the window", async () => {
  const { host, stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url) });
  await host.command("${name}.open").run();
  assert.equal(host.window("main").props.message, "你好");
  await stop();
});
`,
    "README.md": `# ${title}

一个 Vestra Player 插件（id \`${id}\`）。

## 用法

菜单「${title}…」打开窗口。

## 开发

\`\`\`sh
npm install
npm run check && npm test && npm run pack
\`\`\`

在播放器里试用：\`--plugin <这个文件夹>\`。插件 SDK：@vestra-player/sdk ${sdkVersion}。
`,
    "CHANGELOG.md": `# Changelog\n\n## 0.1.0\n\n- 第一版。\n`,
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
