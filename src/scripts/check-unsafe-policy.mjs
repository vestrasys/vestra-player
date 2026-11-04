#!/usr/bin/env node
/**
 * First-party Rust may use `unsafe` only in the crates under src/host/unsafe/
 * (agentic-workflow §1, ci-research §10.4). Runs on any OS and needs no
 * build: it reads every tracked .rs file, so it also covers code that is
 * cfg'd out on the machine running it (Windows code on macOS and so on).
 *
 *   npm run unsafe-policy
 *
 * 1. Workspace packages (`cargo metadata --no-deps`): a package outside
 *    src/host/unsafe/ must inherit the workspace lints (`[lints] workspace =
 *    true`, where `unsafe_code = "forbid"`) or forbid `unsafe_code` itself.
 *    A package under src/host/unsafe/ must be in UNSAFE_CRATES below and
 *    deny `unsafe_op_in_unsafe_fn`, `clippy::undocumented_unsafe_blocks` and
 *    `clippy::missing_safety_doc`.
 * 2. Every tracked .rs file outside src/host/unsafe/ (comments and string
 *    literals removed): no `unsafe` keyword and no attribute that allows,
 *    warns or expects `unsafe_code`.
 * 3. In src/host/unsafe/: every `unsafe { … }` block and `unsafe impl` has a
 *    `// SAFETY:` comment directly above its statement (clippy checks this
 *    only for the platform it compiles).
 *
 * Adding an unsafe crate means editing UNSAFE_CRATES here (and CODEOWNERS),
 * so the change shows in review.
 */
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { relative, resolve, sep } from "node:path";

const UNSAFE_CRATES = new Map([
  ["vestra-open-files", "src/host/unsafe/open"],
  ["vestra-pip", "src/host/unsafe/pip"],
  ["vestra-gpu-render", "src/host/unsafe/render"],
  ["vestra-video-texture", "src/host/unsafe/texture"],
  ["vestra-win32", "src/host/unsafe/win32"],
]);
const UNSAFE_DIR = "src/host/unsafe/";
const UNSAFE_CRATE_LINTS = [
  ["rust", "unsafe_op_in_unsafe_fn", ["deny", "forbid"]],
  ["clippy", "undocumented_unsafe_blocks", ["deny", "forbid"]],
  ["clippy", "missing_safety_doc", ["deny", "forbid"]],
];

const repo = resolve(import.meta.dirname, "..", "..");
const host = resolve(repo, "src", "host");
const problems = [];
const rel = (path) => relative(repo, path).split(sep).join("/");

// ------------------------------------------------------------ Cargo.toml

/** `[section]` → { key: raw value } for the simple TOML Cargo uses here. */
function tomlSections(text) {
  const sections = new Map();
  let current = sections.set("", {}).get("");
  for (const raw of text.split("\n")) {
    const line = raw.replace(/\s+#.*$/, "").trim();
    if (!line || line.startsWith("#")) continue;
    const header = line.match(/^\[([^\]]+)\]$/);
    if (header) {
      current = {};
      sections.set(header[1].trim(), current);
      continue;
    }
    const pair = line.match(/^([\w.-]+)\s*=\s*(.+)$/);
    if (pair) current[pair[1]] = pair[2].trim();
  }
  return sections;
}

/** Lint level from `name = "deny"` or `name = { level = "deny", … }`. */
function level(value) {
  if (!value) return undefined;
  return (value.match(/level\s*=\s*"(\w+)"/) ?? value.match(/^"(\w+)"$/))?.[1];
}

function checkPackages() {
  const metadata = spawnSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1", "--offline"],
    { cwd: host, encoding: "utf8", maxBuffer: 64 << 20 },
  );
  if (metadata.status !== 0) {
    problems.push(`cargo metadata failed:\n${metadata.stderr}`);
    return;
  }
  const { packages, workspace_root: root } = JSON.parse(metadata.stdout);
  const workspace = tomlSections(
    readFileSync(resolve(root, "Cargo.toml"), "utf8"),
  );
  if (level(workspace.get("workspace.lints.rust")?.unsafe_code) !== "forbid")
    problems.push(
      `${rel(resolve(root, "Cargo.toml"))}: [workspace.lints.rust] must set unsafe_code = "forbid"`,
    );
  const seen = new Set();
  for (const pkg of packages) {
    const manifest = rel(pkg.manifest_path);
    const directory = manifest.replace(/\/Cargo\.toml$/, "");
    const sections = tomlSections(readFileSync(pkg.manifest_path, "utf8"));
    if (`${directory}/`.startsWith(UNSAFE_DIR)) {
      if (UNSAFE_CRATES.get(pkg.name) !== directory) {
        problems.push(
          `${manifest}: ${pkg.name} is under ${UNSAFE_DIR} but not in UNSAFE_CRATES (src/scripts/check-unsafe-policy.mjs)`,
        );
        continue;
      }
      seen.add(pkg.name);
      if (sections.get("lints")?.workspace === "true")
        problems.push(
          `${manifest}: an unsafe crate cannot inherit the workspace lints (they forbid unsafe); write [lints.rust] and [lints.clippy]`,
        );
      for (const [tool, lint, wanted] of UNSAFE_CRATE_LINTS) {
        const got = level(sections.get(`lints.${tool}`)?.[lint]);
        if (!wanted.includes(got))
          problems.push(
            `${manifest}: [lints.${tool}] ${lint} must be "deny" (is ${got ?? "unset"})`,
          );
      }
    } else {
      const inherits = sections.get("lints")?.workspace === "true";
      const forbids =
        level(sections.get("lints.rust")?.unsafe_code) === "forbid";
      if (!inherits && !forbids)
        problems.push(
          `${manifest}: ${pkg.name} must have [lints] workspace = true (or [lints.rust] unsafe_code = "forbid")`,
        );
    }
  }
  for (const [name, directory] of UNSAFE_CRATES)
    if (!seen.has(name))
      problems.push(
        `UNSAFE_CRATES lists ${name} (${directory}), which is not a workspace member`,
      );
}

// ------------------------------------------------------------ sources

/**
 * The source with comments and string/char literal contents replaced by
 * spaces (newlines kept, so offsets and line numbers stay valid). Comments are
 * returned separately per line for the SAFETY check.
 */
function strip(source) {
  const out = source.split("");
  const comments = new Map(); // line index → comment text
  let line = 0;
  const blank = (from, to) => {
    for (let i = from; i < to; i++) if (out[i] !== "\n") out[i] = " ";
  };
  const count = (from, to) => {
    for (let i = from; i < to; i++) if (source[i] === "\n") line++;
  };
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    const next = source[i + 1];
    if (c === "\n") {
      line++;
      i++;
    } else if (c === "/" && next === "/") {
      const end = source.indexOf("\n", i);
      const stop = end < 0 ? source.length : end;
      comments.set(line, (comments.get(line) ?? "") + source.slice(i, stop));
      blank(i, stop);
      i = stop;
    } else if (c === "/" && next === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < source.length && depth) {
        if (source[j] === "/" && source[j + 1] === "*") (depth++, (j += 2));
        else if (source[j] === "*" && source[j + 1] === "/")
          (depth--, (j += 2));
        else j++;
      }
      comments.set(line, (comments.get(line) ?? "") + source.slice(i, j));
      blank(i, j);
      count(i, j);
      i = j;
    } else if (
      (c === "r" || (c === "b" && next === "r")) &&
      /^b?r#*"/.test(source.slice(i, i + 260)) &&
      !/\w/.test(source[i - 1] ?? "")
    ) {
      const open = source.slice(i).match(/^b?r(#*)"/);
      const close = `"${open[1]}`;
      const end = source.indexOf(close, i + open[0].length);
      const stop = end < 0 ? source.length : end + close.length;
      blank(i + open[0].length, stop - close.length);
      count(i, stop);
      i = stop;
    } else if (c === '"') {
      let j = i + 1;
      while (j < source.length && source[j] !== '"')
        j += source[j] === "\\" ? 2 : 1;
      blank(i + 1, j);
      count(i, j);
      i = j + 1;
    } else if (c === "'") {
      // A char literal ('a', '\n', '\u{1F600}', '"'), not a lifetime ('a).
      const literal = source
        .slice(i)
        .match(/^'(?:\\(?:u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'/);
      if (literal) {
        blank(i + 1, i + literal[0].length - 1);
        i += literal[0].length;
      } else i++;
    } else i++;
  }
  return { code: out.join(""), comments };
}

const lineOf = (text, offset) => text.slice(0, offset).split("\n").length - 1;

/** `// SAFETY:` in the comment block directly above the statement at `index`. */
function documented(lines, comments, index) {
  // Same line, before the keyword (e.g. `/* SAFETY: … */ unsafe { … }`).
  if (/SAFETY:/.test(comments.get(index) ?? "")) return true;
  // Walk up to the start of the statement: lines that do not end one.
  let start = index;
  while (start > 0) {
    const above = lines[start - 1].trim();
    if (!above || /[;{}]$/.test(above) || above.startsWith("#[")) break;
    start--;
  }
  for (let row = start - 1; row >= 0; row--) {
    const code = lines[row].trim();
    const comment = comments.get(row);
    if (comment && !code) {
      if (/SAFETY:/.test(comment)) return true;
      continue;
    }
    if (code.startsWith("#[") || code.startsWith("#![")) continue;
    return false;
  }
  return false;
}

function checkSources() {
  const listed = spawnSync("git", ["ls-files", "-z", "--", "*.rs"], {
    cwd: repo,
    encoding: "utf8",
  });
  if (listed.status !== 0) {
    problems.push(`git ls-files failed:\n${listed.stderr}`);
    return;
  }
  const files = listed.stdout.split("\0").filter(Boolean);
  let blocks = 0;
  for (const file of files) {
    const { code, comments } = strip(readFileSync(resolve(repo, file), "utf8"));
    const lines = code.split("\n");
    const inside = file.startsWith(UNSAFE_DIR);
    if (!inside) {
      for (const match of code.matchAll(/\bunsafe\b/g))
        problems.push(
          `${file}:${lineOf(code, match.index) + 1}: \`unsafe\` outside ${UNSAFE_DIR}`,
        );
      for (const match of code.matchAll(
        /#!?\[\s*(?:allow|warn|expect)\s*\([^\]]*\bunsafe_code\b/g,
      ))
        problems.push(
          `${file}:${lineOf(code, match.index) + 1}: allow/warn/expect(unsafe_code) outside ${UNSAFE_DIR}`,
        );
      continue;
    }
    for (const match of code.matchAll(/\bunsafe\s*(\{|impl\b)/g)) {
      blocks++;
      const index = lineOf(code, match.index);
      if (!documented(lines, comments, index))
        problems.push(
          `${file}:${index + 1}: \`unsafe ${match[1]}\` without a // SAFETY: comment directly above`,
        );
    }
  }
  console.log(
    `scanned ${files.length} .rs files; ${blocks} unsafe blocks/impls in ${UNSAFE_DIR}`,
  );
}

// Cargo.toml `unsafe_code = "allow"` outside the workspace table is caught by
// checkPackages (a package must inherit or forbid).
checkPackages();
checkSources();
if (problems.length) {
  console.error(`unsafe policy: ${problems.length} problem(s)`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exitCode = 1;
} else console.log("unsafe policy: ok");
