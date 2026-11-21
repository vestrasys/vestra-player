#!/usr/bin/env node
/**
 * Core-thread plan step S2d, Done-when: "no core method takes a Slint type
 * as a parameter or returns one"; widened in plan step S2f(3) to all core
 * code and to `Host` (doc/process/core-thread-plan.md §4, rows R3 and "the
 * core turn takes no Host"). Needs no build.
 *
 *   node src/scripts/check-core-slint.mjs [<src/host/src>]
 *
 * Scans every `impl Core` / `impl <path>::Core` block, every function whose
 * signature names `Core`, and the fields of `struct Core` and of the
 * outbox's types (`Out`, `Then`) in the host's sources (default: the
 * repository's src/host/src), and prints each one that names a Slint type
 * or `Host`. Comments and string literals are blanked before matching; a
 * signature is the text from `fn` to the first `{` or `;` outside
 * parentheses and angle brackets.
 *
 * Plan step S3a(1) adds the driver check: the interface reaches the core
 * only through the driver (`host/driver.rs`). `struct Host` has no field
 * that names `Core`; the core sits in `struct CoreLink`, whose fields are
 * private (the compiler then keeps every other module off them); only
 * `host/driver.rs` names `link.core`; the driver's start-up and quit
 * accessors (`starting`, `quitting`, `backend`) are called from `main.rs`
 * only. The types that carry the interface's copies of core data
 * (`Outbox`, `MountOp`, `CoreFacts`, `PluginsView`, `Copies`, `Sent`) are
 * scanned like `Core`'s fields.
 *
 * Before the scan a self-test runs the same scanner on a small source with
 * known violations and known clean code; a scanner that misses one of them
 * or reports a clean one fails the run (exit 2), so a broken pattern cannot
 * pass silently. Exit 1 when the host's sources have a finding, 0 otherwise.
 */
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(
  process.argv[2] ?? path.join(import.meta.dirname, "..", "host", "src"),
);
if (!fs.existsSync(root)) {
  console.error(`check-core-slint: no such directory: ${root}`);
  process.exit(2);
}
const SLINT =
  /\bslint(_interpreter)?::|\bSValue\b|\bComponentInstance\b|\bComponentHandle\b|\bModelRc\b|\bVecModel\b|\bSharedString\b|\bSharedPixelBuffer\b|\bslint::Image\b|\bslint::Weak\b|\bPhysicalSize\b|\bLogicalSize\b/;

function files(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return files(full);
    return entry.name.endsWith(".rs") ? [full] : [];
  });
}

// Blank comments, string and char literals (same length, newlines kept).
function blank(text) {
  let out = "";
  let i = 0;
  while (i < text.length) {
    const two = text.slice(i, i + 2);
    if (two === "//") {
      while (i < text.length && text[i] !== "\n") ((out += " "), i++);
    } else if (two === "/*") {
      let depth = 0;
      do {
        if (text.slice(i, i + 2) === "/*") (depth++, (out += "  "), (i += 2));
        else if (text.slice(i, i + 2) === "*/")
          (depth--, (out += "  "), (i += 2));
        else ((out += text[i] === "\n" ? "\n" : " "), i++);
      } while (i < text.length && depth > 0);
    } else if (
      /r#*"/.test(text.slice(i, i + 3)) &&
      !/\w/.test(text[i - 1] ?? "")
    ) {
      const hashes = text.slice(i + 1).match(/^#*/)[0];
      const end = '"' + hashes;
      const start = i;
      i += 1 + hashes.length + 1;
      const close = text.indexOf(end, i);
      const stop = close < 0 ? text.length : close + end.length;
      out += text.slice(start, stop).replace(/[^\n]/g, " ");
      i = stop;
    } else if (text[i] === '"') {
      out += " ";
      i++;
      while (i < text.length && text[i] !== '"') {
        if (text[i] === "\\") ((out += "  "), (i += 2));
        else ((out += text[i] === "\n" ? "\n" : " "), i++);
      }
      out += " ";
      i++;
    } else if (
      /^'(\\.|[^\\'])'/.test(text.slice(i, i + 4)) ||
      /^'\\u\{/.test(text.slice(i, i + 4))
    ) {
      const m = text.slice(i).match(/^'(\\u\{[0-9a-fA-F]+\}|\\.|[^\\'])'/);
      const len = m ? m[0].length : 1;
      out += " ".repeat(len);
      i += len;
    } else {
      out += text[i];
      i++;
    }
  }
  return out;
}

// The end (index after its `}`) of the block whose `{` is at `open`.
function blockEnd(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) return i + 1;
  }
  return text.length;
}

function lineOf(text, index) {
  return text.slice(0, index).split("\n").length;
}

// Plan step S2f(3): all core code, not only `impl Core` blocks. A
// function is core code when it is in an `impl Core` block or its
// signature names `Core` (the capabilities' trait methods and helpers, the
// core's free functions). Its signature must name no Slint type and no
// `Host` (the compiler-checked boundary: the core turn takes no `Host`).
// The fields of `struct Core` and of the outbox's types (`Out`, `Then`)
// name no Slint type and no `Host` either.
const HOST = /\bHost\b/;
// The end of a signature starting at `i`: `{` or `;` outside () and <>.
function signatureEnd(text, i) {
  let paren = 0;
  let angle = 0;
  for (let j = i; j < text.length; j++) {
    const d = text[j];
    if (d === "(") paren++;
    else if (d === ")") paren--;
    else if (d === "<") angle++;
    else if (d === ">" && text[j - 1] !== "-" && text[j - 1] !== "=") angle--;
    else if ((d === "{" || d === ";") && paren === 0 && angle <= 0) return j;
  }
  return text.length;
}

// Scans `sources` (`[rel, text]` pairs): the `impl Core` blocks, the core
// signatures, and the findings.
function scan(sources) {
  const findings = [];
  let blocks = 0;
  let signatures = 0;
  for (const [rel, source] of sources) {
    const text = blank(source);
    for (const m of text.matchAll(
      /\b(?:struct|enum)\s+(?:Core|Out|Then|Outbox|MountOp|CoreFacts|PluginsView|Copies|Sent)\s*\{/g,
    )) {
      const open = m.index + m[0].length - 1;
      const body = text.slice(open, blockEnd(text, open));
      body.split("\n").forEach((line, n) => {
        if (SLINT.test(line) || HOST.test(line)) {
          findings.push(
            `${rel}:${lineOf(text, open) + n}: field: ${line.trim()}`,
          );
        }
      });
    }
    const cores = [
      ...text.matchAll(/\bimpl(?:<[^>]*>)?\s+(?:[\w:]+::)?Core\s*\{/g),
    ].map((m) => {
      blocks++;
      const open = m.index + m[0].length - 1;
      return [open, blockEnd(text, open)];
    });
    for (const m of text.matchAll(/(?<!\w)fn\s+\w+/g)) {
      const end = signatureEnd(text, m.index);
      const signature = text.slice(m.index, end).replace(/\s+/g, " ").trim();
      const inCore = cores.some(
        ([open, close]) => m.index > open && m.index < close,
      );
      if (!inCore && !/\bCore\b/.test(signature)) continue;
      signatures++;
      if (SLINT.test(signature) || HOST.test(signature)) {
        findings.push(`${rel}:${lineOf(text, m.index)}: ${signature}`);
      }
    }
  }
  return { blocks, signatures, findings };
}

// The negative self-test: each line marked `BAD` must be found, nothing
// else may be.
const SELF_TEST = `struct Core {
    weak: slint::Weak<Window>, // BAD
    host: Option<Host>, // BAD
    plain: u64,
}
enum Then {
    Event(Event),
    Back(Box<Host>), // BAD
}
impl Core {
    fn takes_host(&mut self, host: &mut Host) {} // BAD
    fn returns_slint(&self) -> slint::SharedString { todo!() } // BAD
    fn clean(&self, owner: u64) -> Option<u64> { None }
}
impl super::Core {
    fn qualified(&self, value: SValue) {} // BAD
}
fn helper(core: &mut Core, instance: ComponentInstance) {} // BAD
fn generic<T: Fn(&mut Core) -> Vec<Host>>(f: T) {} // BAD
fn free_clean(core: &Core) -> u64 { 0 }
impl Host {
    fn interface(&self, instance: ComponentInstance) {}
}
// fn commented(core: &Core, host: &Host) {}
/* fn blocked(core: &Core, host: &Host) {} */
fn literal() -> &'static str { "fn quoted(core: &Core, host: &Host)" }
`;
{
  const want = SELF_TEST.split("\n")
    .map((line, n) => (line.includes("// BAD") ? n + 1 : 0))
    .filter(Boolean);
  const got = scan([["self-test.rs", SELF_TEST]]).findings.map((f) =>
    Number(f.split(":")[1]),
  );
  if (want.join() !== got.join()) {
    console.error(
      `check-core-slint: self-test failed: expected findings on lines ${want.join(", ")}, got ${got.join(", ") || "none"}`,
    );
    process.exit(2);
  }
}

// Plan step S3a(1): the interface reaches the core only through the driver.
const DRIVER = "host/driver.rs";
const ACCESSORS = /\.\s*(starting|quitting|backend)\s*\(/g;
function scanDriver(sources) {
  const findings = [];
  let links = 0;
  for (const [rel, source] of sources) {
    const text = blank(source);
    for (const m of text.matchAll(/\bstruct\s+(Host|CoreLink)\s*\{/g)) {
      const open = m.index + m[0].length - 1;
      const body = text.slice(open + 1, blockEnd(text, open) - 1);
      if (m[1] === "CoreLink" && rel !== DRIVER) {
        findings.push(
          `${rel}:${lineOf(text, m.index)}: CoreLink outside ${DRIVER}`,
        );
      }
      body.split("\n").forEach((line, n) => {
        const at = `${rel}:${lineOf(text, open) + n}`;
        if (m[1] === "Host" && /:\s*[^,]*\bCore\b/.test(line)) {
          findings.push(`${at}: a Host field names Core: ${line.trim()}`);
        }
        if (m[1] === "CoreLink") {
          if (/\bpub\b/.test(line)) {
            findings.push(
              `${at}: a CoreLink field is not private: ${line.trim()}`,
            );
          }
          if (/\bCore\b/.test(line)) links++;
        }
      });
    }
    if (rel !== DRIVER) {
      for (const m of text.matchAll(/\blink\s*\.\s*core\b/g)) {
        findings.push(
          `${rel}:${lineOf(text, m.index)}: link.core outside ${DRIVER}`,
        );
      }
    }
    if (rel !== "main.rs" && rel !== DRIVER) {
      for (const m of text.matchAll(ACCESSORS)) {
        // `backend()` is also the name of unrelated methods: only the
        // driver's, called on a `Host` (`host.backend()`), counts.
        const before = text.slice(Math.max(0, m.index - 12), m.index);
        if (m[1] === "backend" && !/\bhost\s*$/.test(before)) continue;
        findings.push(
          `${rel}:${lineOf(text, m.index)}: the driver's ${m[1]}() outside main.rs`,
        );
      }
    }
  }
  return { links, findings };
}

// Its negative self-test, over three files.
{
  const sources = [
    [
      DRIVER,
      `pub(crate) struct CoreLink {
    core: Core,
}
fn drive(host: &mut Host) { host.link.core.handle(); }`,
    ],
    [
      "host/mod.rs",
      `pub(crate) struct Host {
    pub(crate) core: Core, // BAD
    pub(crate) link: CoreLink,
    pub(crate) mounts: BTreeMap<u64, Mount>,
}
pub(crate) struct CoreLink { // BAD
    pub(crate) core: Core, // BAD
}
fn interface(host: &mut Host) {
    host.link.core.dialog_busy = true; // BAD
    host.starting(|core| ()); // BAD
    let backend = host.backend(); // BAD
    let other = renderer.backend();
    // host.link.core in a comment
}`,
    ],
    [
      "main.rs",
      `fn main() { host.starting(|core| ()); host.quitting(|core| ()); }`,
    ],
  ];
  const want = sources[1][1]
    .split("\n")
    .map((line, n) => (line.includes("// BAD") ? n + 1 : 0))
    .filter(Boolean);
  const { links, findings: found } = scanDriver(sources);
  const got = [...new Set(found.map((f) => Number(f.split(":")[1])))];
  if (
    links !== 2 ||
    found.some((f) => !f.startsWith("host/mod.rs:")) ||
    want.join() !== got.join()
  ) {
    console.error(
      `check-core-slint: driver self-test failed: expected findings on host/mod.rs lines ${want.join(", ")}, got ${found.join("; ") || "none"}`,
    );
    process.exit(2);
  }
}

// Plan step S3a(2): the grep gate over the bodies of core code (the S3a
// row; design §5.3 group G). Core code is a function in an `impl Core`
// block or one whose signature names `Core`. Its body may not: touch a
// thread-local of the host (any `thread_local!` of the sources, used as
// `NAME.with(..)` and the like); call a `platform::` function that reaches
// one of the platform's thread-locals (directly or through the platform
// file's own functions); call `slint::` or `slint_interpreter::` other than
// `slint::invoke_from_event_loop` (and what runs inside its closure, on the
// interface's thread); compile Slint through `composition::build` or
// `composition::shell_in`; or use `clipboard.rs` (`crate::clipboard::`).
// The named exceptions below are each a finding the gate prints apart,
// with the number of findings each covers pinned (`count`): a second use
// under the same exception (a new call in that file) fails the gate, and
// so does one gone (the exception is then stale and is removed).
const EXCEPTIONS = [
  {
    file: "ui/settings_window.rs",
    rule: "slint-compile",
    count: 1,
    why: "settings.register compiles its page in place only when the settings-check thread cannot take it (S3a brief item 8, the named exception: the check thread runs the same compile off the interface's thread; refusing would change a reply)",
  },
  // Plan step S3a(3): `clipboard.*` runs on its serial thread
  // (`clipboard::Serial`), so the clipboard exception of S3a(2) is gone.
];
const THREAD_LOCAL_USE = (names) =>
  new RegExp(
    `\\b(${names.join("|")})\\s*\\.\\s*(with|set|get|take|replace|with_borrow|with_borrow_mut)\\s*\\(`,
    "g",
  );
function threadLocals(sources) {
  const names = new Set();
  for (const [, source] of sources) {
    const text = blank(source);
    for (const m of text.matchAll(/thread_local!\s*\{/g)) {
      const open = m.index + m[0].length - 1;
      for (const s of text
        .slice(open, blockEnd(text, open))
        .matchAll(/\bstatic\s+([A-Z_][A-Z0-9_]*)/g))
        names.add(s[1]);
    }
  }
  return [...names].sort();
}
// The platform functions that reach a platform thread-local, through the
// platform file's own functions too.
function platformWindowFns(sources, names) {
  const deny = new Set();
  if (!names.length) return deny;
  for (const [rel, source] of sources) {
    if (!/^platform\/(windows|macos|other|mod)\.rs$/.test(rel)) continue;
    const text = blank(source);
    const bodies = new Map();
    for (const m of text.matchAll(/(?<!\w)fn\s+(\w+)/g)) {
      const end = signatureEnd(text, m.index);
      if (text[end] !== "{") continue;
      bodies.set(m[1], text.slice(end, blockEnd(text, end)));
    }
    const reaching = new Set(
      [...bodies]
        .filter(([, body]) => THREAD_LOCAL_USE(names).test(body))
        .map(([name]) => name),
    );
    let grew = true;
    while (grew) {
      grew = false;
      for (const [name, body] of bodies) {
        if (reaching.has(name)) continue;
        if (
          [...reaching].some((r) => new RegExp(`\\b${r}\\s*\\(`).test(body))
        ) {
          reaching.add(name);
          grew = true;
        }
      }
    }
    for (const name of reaching) deny.add(name);
  }
  return deny;
}
// The core bodies of `sources`: `[rel, text (blanked), [open, close]]`.
function coreBodies(sources) {
  const out = [];
  for (const [rel, source] of sources) {
    const text = blank(source);
    const cores = [
      ...text.matchAll(/\bimpl(?:<[^>]*>)?\s+(?:[\w:]+::)?Core\s*\{/g),
    ].map((m) => {
      const open = m.index + m[0].length - 1;
      return [open, blockEnd(text, open)];
    });
    for (const m of text.matchAll(/(?<!\w)fn\s+\w+/g)) {
      const end = signatureEnd(text, m.index);
      if (text[end] !== "{") continue;
      const signature = text.slice(m.index, end);
      const inCore = cores.some(
        ([open, close]) => m.index > open && m.index < close,
      );
      if (!inCore && !/\bCore\b/.test(signature)) continue;
      out.push([rel, text, [end, blockEnd(text, end)]]);
    }
  }
  return out;
}
// The ranges of `slint::invoke_from_event_loop(...)` calls.
function invokeRanges(text) {
  const ranges = [];
  for (const m of text.matchAll(/slint\s*::\s*invoke_from_event_loop\s*\(/g)) {
    let depth = 0;
    for (let i = m.index + m[0].length - 1; i < text.length; i++) {
      if (text[i] === "(") depth++;
      else if (text[i] === ")" && --depth === 0) {
        ranges.push([m.index, i]);
        break;
      }
    }
  }
  return ranges;
}
function scanBodies(sources) {
  const names = threadLocals(sources);
  const deny = platformWindowFns(sources, names);
  const findings = [];
  const seen = new Set();
  for (const [rel, text, [open, close]] of coreBodies(sources)) {
    const body = text.slice(open, close);
    const invokes = invokeRanges(text);
    const report = (offset, rule, what) => {
      const at = open + offset;
      const key = `${rel}:${at}:${rule}`;
      if (seen.has(key)) return;
      seen.add(key);
      findings.push({ rel, line: lineOf(text, at), rule, what });
    };
    if (names.length)
      for (const m of body.matchAll(THREAD_LOCAL_USE(names)))
        report(m.index, "thread-local", `${m[1]}.${m[2]}(..)`);
    for (const m of body.matchAll(/\bplatform\s*::\s*(\w+)/g))
      if (deny.has(m[1]))
        report(m.index, "platform-window", `platform::${m[1]}`);
    for (const m of body.matchAll(/\bslint(?:_interpreter)?\s*::\s*(\w+)/g)) {
      const at = open + m.index;
      if (m[1] === "invoke_from_event_loop") continue;
      if (invokes.some(([a, b]) => at > a && at < b)) continue;
      report(m.index, "slint", m[0].replace(/\s+/g, ""));
    }
    for (const m of body.matchAll(
      /\bcomposition\s*::\s*(build|shell_in)\s*\(/g,
    ))
      report(m.index, "slint-compile", `composition::${m[1]}`);
    for (const m of body.matchAll(/\bcrate\s*::\s*clipboard\s*::\s*\w+/g))
      report(m.index, "clipboard", m[0].replace(/\s+/g, ""));
  }
  return { names, deny, findings };
}
// Its negative self-test.
{
  const sources = [
    [
      "platform/windows.rs",
      `thread_local! { static WINDOWS: RefCell<Vec<u64>> = RefCell::new(Vec::new()); }
fn count() -> usize { WINDOWS.with(|w| w.borrow().len()) }
pub fn raise() { let _ = count(); }
pub fn memory() -> u64 { 7 }`,
    ],
    [
      "host/x.rs",
      `impl Core {
    fn a(&self) { platform::raise(); } // BAD
    fn b(&self) { let _ = platform::memory(); }
    fn c(&self) { let _ = slint::invoke_from_event_loop(|| { let _ = slint::quit_event_loop(); }); }
    fn d(&self) { let _ = slint::quit_event_loop(); } // BAD
    fn e(&self) { WINDOWS.with(|w| ()); } // BAD
    fn f(&self) { let _ = composition::build(x, "A"); } // BAD
    fn g(&self) { crate::clipboard::dispatch(); } // BAD
    // fn h(&self) { platform::raise(); }
}
impl Host {
    fn i(&self) { platform::raise(); WINDOWS.with(|w| ()); }
}
fn helper(core: &mut Core) { let _ = slint_interpreter::Value::Void; } // BAD`,
    ],
  ];
  const want = sources[1][1]
    .split("\n")
    .map((line, n) => (line.includes("// BAD") ? n + 1 : 0))
    .filter(Boolean);
  const got = scanBodies(sources)
    .findings.filter((f) => f.rel === "host/x.rs")
    .map((f) => f.line)
    .sort((a, b) => a - b);
  if (want.join() !== got.join()) {
    console.error(
      `check-core-slint: body gate self-test failed: expected findings on lines ${want.join(", ")}, got ${got.join(", ") || "none"}`,
    );
    process.exit(2);
  }
}

const sources = files(root).map((file) => [
  path.relative(root, file).replaceAll("\\", "/"),
  fs.readFileSync(file, "utf8"),
]);
const { blocks, signatures, findings } = scan(sources);
console.log(
  `impl Core blocks: ${blocks}, core signatures: ${signatures}, with a Slint type or Host: ${findings.length}`,
);
const driver = scanDriver(sources);
if (driver.links !== 1) {
  driver.findings.push(
    `${DRIVER}: expected one CoreLink holding the core, found ${driver.links}`,
  );
}
console.log(`driver (plan step S3a(1)): ${driver.findings.length} finding(s)`);
const gate = scanBodies(sources);
const excepted = gate.findings.filter((f) =>
  EXCEPTIONS.some((e) => e.file === f.rel && e.rule === f.rule),
);
const gated = gate.findings.filter((f) => !excepted.includes(f));
console.log(
  `core bodies (plan step S3a(2)): ${gate.names.length} thread-locals, ${gate.deny.size} platform functions that reach one; ${gated.length} finding(s), ${excepted.length} under a named exception`,
);
for (const f of excepted)
  console.log(
    `  exception: ${f.rel}:${f.line}: ${f.what} (${EXCEPTIONS.find((e) => e.file === f.rel && e.rule === f.rule).why})`,
  );
// Each exception covers exactly its pinned number of findings.
const miscounted = EXCEPTIONS.map((e) => [
  e,
  excepted.filter((f) => f.rel === e.file && f.rule === e.rule).length,
])
  .filter(([e, found]) => found !== e.count)
  .map(
    ([e, found]) =>
      `exception ${e.file} (${e.rule}): expected ${e.count} finding(s) under it, found ${found}`,
  );
for (const finding of [...findings, ...driver.findings]) console.log(finding);
for (const f of gated) console.log(`${f.rel}:${f.line}: ${f.rule}: ${f.what}`);
for (const line of miscounted) console.log(line);
process.exit(
  findings.length || driver.findings.length || gated.length || miscounted.length
    ? 1
    : 0,
);
