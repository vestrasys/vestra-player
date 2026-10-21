#!/usr/bin/env node
/**
 * 临时，正式 CI 建立后删除。
 * Temporary portability check (portability round P0); delete it once real CI
 * builds the host on Windows.
 *
 *   npm run portability [-- --skip-host] [-- --skip-scan]
 *
 * 1. Host: `cargo check --target x86_64-pc-windows-msvc --all-targets`.
 *    With rustup the target is added when missing. Without rustup (e.g.
 *    Homebrew Rust) the standard library is checked from the toolchain's
 *    rust-src with `-Zbuild-std` (RUSTC_BOOTSTRAP=1, check only, nothing is
 *    linked). Dependencies whose build script or C code cannot be
 *    cross-checked are listed with the reason, and the host is checked again
 *    without the optional features that pull them in; that is reported as a
 *    partial check, never as a pass.
 *    Also lists `cfg(unix)` / `cfg(target_os)` code outside the host's
 *    platform module (notes, not failures).
 * 2. Scan: string literals in src/plugins, src/sdk and src/runtime (without
 *    tests, dist, vendor, node_modules) that name system command paths or
 *    macOS-only commands, and `process.platform` branches.
 *
 * Exit status 0 only when the host check passed fully and the scan found no
 * system command.
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import { extname, join, relative, resolve } from "node:path";
import ts from "typescript";

const TARGET = "x86_64-pc-windows-msvc";
const source = resolve(import.meta.dirname, "..");
const host = resolve(source, "host");
const args = new Set(process.argv.slice(2));
let failed = false;

const run = (command, list, options = {}) =>
  spawnSync(command, list, {
    encoding: "utf8",
    maxBuffer: 256 << 20,
    ...options,
  });
const found = (command) => !run(command, ["--version"]).error;

// ---------------------------------------------------------------- host

function toolchain() {
  if (found("rustup")) {
    const installed = run("rustup", ["target", "list", "--installed"]).stdout;
    if (!installed.split(/\s+/).includes(TARGET)) {
      console.log(`rustup: adding ${TARGET}`);
      const added = run("rustup", ["target", "add", TARGET], {
        stdio: "inherit",
      });
      if (added.status !== 0)
        return { error: `rustup target add ${TARGET} failed (offline?)` };
    }
    return { mode: "rustup", flags: [], env: {} };
  }
  const sysroot = run("rustc", ["--print", "sysroot"]).stdout?.trim();
  if (!sysroot) return { error: "rustc was not found" };
  if (existsSync(join(sysroot, "lib", "rustlib", TARGET)))
    return { mode: "installed target", flags: [], env: {} };
  if (existsSync(join(sysroot, "lib", "rustlib", "src", "rust", "library")))
    return {
      mode: "build-std (no rustup; std checked from rust-src)",
      flags: ["-Zbuild-std=std,panic_abort"],
      env: { RUSTC_BOOTSTRAP: "1" },
    };
  return {
    error:
      `no ${TARGET} standard library and no rustup. Install rustup ` +
      `(https://rustup.rs) and run \`rustup target add ${TARGET}\`, or a ` +
      "toolchain with rust-src.",
  };
}

/** `cargo check` with JSON messages: errors, warnings by file, blocked crates. */
function check(chain, extra) {
  const result = run(
    "cargo",
    [
      "check",
      "--locked",
      "--target",
      TARGET,
      "--all-targets",
      "--target-dir",
      join(host, "target", "portability"),
      "--message-format",
      "json-diagnostic-short",
      ...chain.flags,
      ...extra,
    ],
    { cwd: host, env: { ...process.env, ...chain.env } },
  );
  const errors = [];
  const warnings = new Map();
  const blocked = new Map();
  for (const line of result.stdout.split("\n")) {
    if (!line.startsWith("{")) continue;
    const message = JSON.parse(line);
    const crate =
      message.package_id?.match(/[#/]([\w-]+)@/)?.[1] ?? message.target?.name;
    if (message.reason === "build-script-executed") continue;
    if (message.reason !== "compiler-message") continue;
    const diagnostic = message.message;
    const own = message.target?.src_path?.startsWith(host);
    const text = diagnostic.rendered?.trim() ?? diagnostic.message;
    if (diagnostic.level === "error") {
      if (own) errors.push(text);
      else blocked.set(crate, text.split("\n").slice(0, 3).join(" "));
    } else if (diagnostic.level === "warning" && own) {
      // The binary and its tests report the same warning once each.
      const span = diagnostic.spans?.[0];
      const file = span?.file_name ?? "?";
      const key = `${span?.line_start}:${diagnostic.message}`;
      const list = warnings.get(file) ?? new Map();
      list.set(key, diagnostic.message);
      warnings.set(file, list);
    }
  }
  // Build scripts that fail (pkg-config, cc for the target) print to stderr.
  for (const match of result.stderr.matchAll(
    /failed to run custom build command for `([\w-]+) [^`]*`([\s\S]*?)(?=\n(?:warning|error):|\n\s*$|$)/g,
  )) {
    const tail = match[2]
      .trim()
      .split("\n")
      .filter(Boolean)
      .slice(-3)
      .join(" ");
    blocked.set(match[1], `build script: ${tail}`);
  }
  if (result.status !== 0 && !errors.length && !blocked.size)
    errors.push(result.stderr.trim().split("\n").slice(-15).join("\n"));
  for (const [file, list] of warnings) warnings.set(file, [...list.values()]);
  return { ok: result.status === 0, errors, warnings, blocked };
}

/**
 * `cfg(unix)` / `cfg(target_os)` / `std::os::` and absolute system paths
 * outside src/platform/ (comments skipped).
 */
async function hostNotes() {
  const root = join(host, "src");
  const notes = [];
  async function walk(directory) {
    for (const item of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, item.name);
      if (item.isDirectory()) {
        if (item.name !== "platform") await walk(path);
      } else if (item.name.endsWith(".rs")) {
        const lines = (await readFile(path, "utf8")).split("\n");
        lines.forEach((line, index) => {
          if (/^\s*\/\//.test(line)) return;
          if (
            /cfg\((not\()?(unix|windows|target_os)|std::os::/.test(line) ||
            /"(\/usr\/|\/bin\/|\/opt\/|\/System\/|\/Library\/|\/tmp\b)/.test(
              line,
            )
          )
            notes.push(
              `${relative(source, path)}:${index + 1}: ${line.trim()}`,
            );
        });
      }
    }
  }
  await walk(root);
  return notes;
}

async function checkHost() {
  console.log(`== Host: cargo check --target ${TARGET}`);
  const chain = toolchain();
  if (chain.error) {
    console.log(`NOT CHECKED: ${chain.error}`);
    failed = true;
    return;
  }
  console.log(`toolchain: ${chain.mode}`);
  let result = check(chain, []);
  let partial = false;
  if (!result.ok && result.blocked.size && !result.errors.length) {
    console.log("Blocked dependencies (cannot be cross-checked here):");
    for (const [crate, reason] of result.blocked)
      console.log(`  - ${crate}: ${reason}`);
    console.log("Checking again without default (optional) features…");
    result = check(chain, ["--no-default-features"]);
    partial = true;
  }
  if (result.ok) {
    console.log(
      partial
        ? "PARTIAL: the host compiles for Windows without the blocked optional features; the blocked dependencies above were NOT checked."
        : "OK: the host compiles for Windows (check only; nothing linked or run).",
    );
    if (partial) failed = true;
  } else {
    failed = true;
    console.log("FAILED:");
    for (const [crate, reason] of result.blocked)
      console.log(`  - dependency ${crate}: ${reason}`);
    for (const error of result.errors) console.log(error);
  }
  const total = [...result.warnings.values()].reduce((n, w) => n + w.length, 0);
  if (total) {
    console.log(
      `Windows-only warnings in the host (${total}; mostly code that is only used on Unix):`,
    );
    for (const [file, list] of [...result.warnings].sort())
      console.log(
        `  ${file}: ${list.length} (${[...new Set(list)].slice(0, 3).join("; ")}${list.length > 3 ? "; …" : ""})`,
      );
  }
  const notes = await hostNotes();
  if (notes.length) {
    console.log(
      `System-specific code outside src/host/src/platform/ (${notes.length}, notes):`,
    );
    for (const note of notes) console.log(`  ${note}`);
  }
}

// ---------------------------------------------------------------- scan

const SKIP = new Set([
  "tests",
  "dist",
  "vendor",
  "node_modules",
  ".typecheck",
  ".cache",
]);
const CODE = new Set([".ts", ".mts", ".cts", ".js", ".mjs", ".cjs"]);
/** Absolute locations of system programs and macOS-only directories. */
const PATHS =
  /(^|[\s"'`=:])(\/usr\/(s?bin|local|libexec)\/|\/s?bin\/|\/opt\/homebrew|\/opt\/local\/|\/System\/|\/Library\/|\/Applications\/|~\/Library|Library\/Application Support)/;
/** Programs that exist only on macOS (or are named differently elsewhere). */
const MACOS = new Set([
  "open",
  "sips",
  "plutil",
  "sw_vers",
  "sysctl",
  "system_profiler",
  "security",
  "osascript",
  "pbcopy",
  "pbpaste",
  "caffeinate",
  "defaults",
  "mdls",
  "mdfind",
  "launchctl",
  "diskutil",
  "pmset",
  "afplay",
  "say",
  "xattr",
  "codesign",
  "hdiutil",
  "ditto",
  "screencapture",
  "textutil",
  "qlmanage",
  "scutil",
]);
const SPAWNERS = new Set([
  "spawn",
  "spawnSync",
  "exec",
  "execSync",
  "execFile",
  "execFileSync",
  "fork",
]);

async function files(directory) {
  const out = [];
  let items;
  try {
    items = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return out;
    throw error;
  }
  for (const item of items) {
    const path = join(directory, item.name);
    if (item.isDirectory()) {
      if (!SKIP.has(item.name)) out.push(...(await files(path)));
    } else if (
      CODE.has(extname(item.name)) &&
      !/\.test\.[mc]?[jt]s$/.test(item.name)
    )
      out.push(path);
  }
  return out;
}

function scanFile(path, text) {
  const parsed = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true);
  const findings = [];
  const at = (node) =>
    parsed.getLineAndCharacterOfPosition(node.getStart()).line + 1;
  const callee = (call) => {
    const expression = call.expression;
    if (ts.isIdentifier(expression)) return expression.text;
    if (ts.isPropertyAccessExpression(expression)) return expression.name.text;
    return "";
  };
  function visit(node) {
    if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node)) {
      const value = node.text;
      if (PATHS.test(value))
        findings.push({ kind: "command", line: at(node), text: value });
      else if (
        ts.isCallExpression(node.parent) &&
        SPAWNERS.has(callee(node.parent)) &&
        node.parent.arguments[0] === node &&
        MACOS.has(value.split(/\s+/)[0])
      )
        findings.push({ kind: "command", line: at(node), text: value });
    }
    if (
      ts.isPropertyAccessExpression(node) &&
      node.name.text === "platform" &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "process"
    )
      findings.push({
        kind: "platform",
        line: at(node),
        text: node.parent.getText(parsed).slice(0, 80),
      });
    ts.forEachChild(node, visit);
  }
  visit(parsed);
  return findings;
}

async function scan() {
  console.log(
    "\n== Scan: system commands in src/plugins, src/sdk, src/runtime",
  );
  let commands = 0;
  let branches = 0;
  for (const root of ["plugins", "sdk", "runtime"]) {
    for (const path of (await files(join(source, root))).sort()) {
      for (const finding of scanFile(path, await readFile(path, "utf8"))) {
        const where = `${relative(source, path)}:${finding.line}`;
        if (finding.kind === "command") {
          commands++;
          console.log(`  COMMAND  ${where}: ${JSON.stringify(finding.text)}`);
        } else {
          branches++;
          console.log(`  PLATFORM ${where}: ${finding.text}`);
        }
      }
    }
  }
  console.log(
    `${commands} system command reference(s); ${branches} process.platform branch(es) (review, not failures).`,
  );
  if (commands) failed = true;
}

if (!args.has("--skip-host")) await checkHost();
if (!args.has("--skip-scan")) await scan();
console.log(failed ? "\nportability: problems found" : "\nportability: clean");
process.exit(failed ? 1 : 0);
