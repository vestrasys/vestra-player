// The static gates of an additions-only change (scripts/verify-gates.mjs,
// doc/process/verify-additions.md): each gate passes on additions and fails
// on a removed or modified line, in throwaway repositories built with git
// plumbing (no `git commit`: no hooks, no signing, no user configuration).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";

// @ts-expect-error: a JavaScript build script without declarations.
import * as tool from "../../scripts/verify-gates.mjs";

const {
  compareCapabilities,
  compareSurface,
  format,
  isTestPath,
  parseCapabilitiesTs,
  removedLines,
  runGates,
  rustTestRanges,
} = tool;

const roots: string[] = [];
after(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
});

type Files = Record<string, string | null>;

/** A repository; `commit(files)` writes them (null deletes) and returns a commit. */
function repository(): {
  root: string;
  commit(files: Files): string;
  write(files: Files): void;
} {
  const root = mkdtempSync(join(tmpdir(), "vestra-gates-"));
  roots.push(root);
  const git = (...args: string[]): string => {
    const result = spawnSync("git", ["-C", root, ...args], {
      encoding: "utf8",
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: "gate test",
        GIT_AUTHOR_EMAIL: "gate@test.invalid",
        GIT_COMMITTER_NAME: "gate test",
        GIT_COMMITTER_EMAIL: "gate@test.invalid",
        GIT_CONFIG_NOSYSTEM: "1",
      },
    });
    assert.equal(result.status, 0, `git ${args.join(" ")}: ${result.stderr}`);
    return result.stdout.trim();
  };
  git("init", "--quiet");
  git("config", "core.autocrlf", "false");
  let parent: string | null = null;
  const write = (files: Files): void => {
    for (const [path, text] of Object.entries(files)) {
      const file = join(root, path);
      if (text === null) rmSync(file, { force: true });
      else {
        mkdirSync(dirname(file), { recursive: true });
        writeFileSync(file, text);
      }
    }
  };
  return {
    root,
    write,
    commit(files) {
      write(files);
      git("add", "-A");
      const tree = git("write-tree");
      parent = git(
        "commit-tree",
        tree,
        "-m",
        "snapshot",
        ...(parent ? ["-p", parent] : []),
      );
      git("update-ref", "HEAD", parent);
      return parent;
    },
  };
}

type Gate = { id: string; status: string; problems: string[]; notes: string[] };
const gateOf = (result: { gates: Gate[] }, id: string): Gate => {
  const gate = result.gates.find((entry) => entry.id === id);
  assert.ok(gate, `gate ${id}`);
  return gate;
};

const table = (extra: object[] = [], uncapable: object[] = []): string =>
  JSON.stringify(
    {
      about: "generated",
      apiVersion: "vestra/1",
      capabilities: [
        {
          builtinOnly: false,
          label: "Add menu commands",
          methods: [
            { name: "commands.register", needs: "commands" },
            { name: "host.run", needs: "commands" },
          ],
          name: "commands",
          requires: [],
          residency: "recordable",
          risky: false,
        },
        {
          builtinOnly: false,
          label: "Show text on the video",
          methods: [{ name: "ui.osd", needs: "ui.osd" }],
          name: "ui.osd",
          requires: [],
          residency: "none",
          risky: false,
        },
        ...extra,
      ],
      uncapable: [{ name: "window.state", needs: null }, ...uncapable],
    },
    null,
    2,
  );
const MPV = {
  builtinOnly: false,
  label: "Control the player engine directly",
  methods: [{ name: "mpv.command", needs: "mpv" }],
  name: "mpv",
  requires: [],
  residency: "live",
  risky: true,
};

const RUST = `pub fn double(x: u32) -> u32 {
    x * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles() {
        assert_eq!(double(2), 4);
    }
}
`;

const SDK: Files = {
  "src/host-capabilities.ts": `import type { HostCapabilities } from "./capability-rules.ts";\n/** @internal */\nexport const HOST_CAPABILITIES: HostCapabilities = ${table()};\n`,
  "src/index.ts": "export const a = 1;\n",
  "test/fixtures/api-surface.txt":
    "sdk InterfaceDeclaration App\nsdk InterfaceDeclaration App :: system(): Promise<SystemInfo>;\n",
  "test/fixtures/traces/hello.json": '{"requests":["commands.register"]}\n',
  "test/hello.test.ts": 'test("a", () => {\n  assert.equal(1, 1);\n});\n',
};
const PLAYER: Files = {
  "src/host/capabilities.json": table(),
  "src/host/src/math.rs": RUST,
  "src/host/tests/lazy.rs": "#[test]\nfn parks() {\n    assert!(true);\n}\n",
  "src/tests/sdk/runner.test.ts": 'test("runs", () => {});\n',
  "src/plugins/store/tests/dialog.test.ts": 'test("lists", () => {});\n',
};

test("test files are recognised by directory and by name", () => {
  for (const path of [
    "test/a.ts",
    "test/fixtures/traces/x.json",
    "src/tests/sdk/runner.test.ts",
    "src/host/tests/lazy_activation.rs",
    "src/host/src/i18n/tests.rs",
    "src/host/src/ui/widgets_tests.rs",
    "plugins/clipexport/tests/clip.test.ts",
    "scripts/tests/index.test.mjs",
    "src/a.test.ts",
  ])
    assert.ok(isTestPath(path), path);
  for (const path of [
    "src/host/src/capabilities/mod.rs",
    "src/testing/host.ts",
    "docs/testing.md",
    "src/contest/tests.rs.txt",
  ])
    assert.ok(!isTestPath(path), path);
});

test("#[cfg(test)] items of a Rust file are found, strings and comments skipped", () => {
  assert.deepEqual(rustTestRanges(RUST), [{ start: 5, end: 13 }]);
  const tricky = [
    'const A: &str = "#[cfg(test)] mod x { not code }";', // 1
    "// #[cfg(test)] mod y {", // 2
    "/* #[cfg(test)] /* nested */ mod z { */", // 3
    'const B: &str = r#"}"#;', // 4
    "fn life<'a>(x: &'a str) -> char { '}' }", // 5
    "#[cfg(test)]", // 6
    "use std::fmt;", // 7
    "#[cfg(not(test))]", // 8
    "fn real() {}", // 9
    "#[cfg(any(windows, test))]", // 10
    "mod lease;", // 11
    "#[cfg(all(test, windows))]", // 12
    "#[allow(dead_code)]", // 13
    "fn helper(x: [u8; 2]) -> u8 {", // 14
    '    let s = "}"; x[0]', // 15
    "}", // 16
    "fn body() {", // 17
    "    #[cfg(test)]", // 18
    "    let x = if a { 1 } else {", // 19
    "        2", // 20
    "    };", // 21
    "}", // 22
  ].join("\n");
  assert.deepEqual(rustTestRanges(tricky), [
    { start: 6, end: 7 },
    { start: 12, end: 16 },
    { start: 18, end: 21 },
  ]);
  assert.deepEqual(rustTestRanges("#![cfg(test)]\nfn a() {}\n"), [
    { start: 1, end: 3 },
  ]);
  assert.deepEqual(rustTestRanges("fn a() {}\n"), []);
});

test("removed lines carry their line number in the old file", () => {
  const diff = [
    "diff --git a/x b/x",
    "--- a/x",
    "+++ b/x",
    "@@ -3,2 +3 @@ context",
    "-old three",
    "-old four",
    "+new",
    "@@ -9,0 +9,2 @@",
    "+added",
    "+--- not a header",
    "@@ -20 +21 @@",
    "---- a removed line starting with dashes",
    "+x",
  ].join("\n");
  assert.deepEqual(removedLines(diff), {
    binary: false,
    removed: [
      { line: 3, text: "old three" },
      { line: 4, text: "old four" },
      { line: 20, text: "--- a removed line starting with dashes" },
    ],
  });
  assert.equal(
    removedLines("diff --git a/x b/x\nBinary files a/x and b/x differ\n")
      .binary,
    true,
  );
});

test("the API surface may only grow", () => {
  const base = "a\nb :: f(): void;\nc\n";
  assert.deepEqual(compareSurface(base, "a\nb :: f(): void;\nc\nd\n"), {
    removed: [],
    added: 1,
    total: 3,
  });
  // Reordered lines and CRLF are no change; a changed signature is.
  assert.deepEqual(
    compareSurface(base, "c\r\na\r\nb :: f(): void;\r\n").removed,
    [],
  );
  assert.deepEqual(compareSurface(base, "a\nb :: f(x?: number): void;\nc\n"), {
    removed: ["b :: f(): void;"],
    added: 1,
    total: 3,
  });
});

test("the capability table: existing entries identical, additions noted", () => {
  const base = JSON.parse(table());
  assert.deepEqual(compareCapabilities(base, JSON.parse(table())), {
    problems: [],
    notes: [],
  });
  // A new capability and a new capability-free method.
  const grown = compareCapabilities(
    base,
    JSON.parse(table([MPV], [{ name: "app.locale", needs: null }])),
  );
  assert.deepEqual(grown.problems, []);
  assert.deepEqual(grown.notes, [
    'new capability "mpv"',
    'new capability-free method "app.locale"',
  ]);
  // Key order inside an entry is formatting.
  const reordered = JSON.parse(table());
  reordered.capabilities[0] = Object.fromEntries(
    Object.entries(reordered.capabilities[0]).reverse(),
  );
  assert.deepEqual(compareCapabilities(base, reordered).problems, []);

  const change = (edit: (head: any) => void): string[] => {
    const head = JSON.parse(table([MPV]));
    edit(head);
    return compareCapabilities(base, head).problems;
  };
  assert.match(
    change((head) =>
      head.capabilities[0].methods.push({
        name: "commands.x",
        needs: "commands",
      }),
    )[0],
    /capability "commands" changed \(methods: added commands\.x\)/,
  );
  assert.match(
    change((head) => (head.capabilities[0].methods[1].needs = null))[0],
    /needs changed for host\.run/,
  );
  assert.match(
    change((head) => head.capabilities[0].methods.reverse())[0],
    /methods: order changed/,
  );
  assert.match(
    change((head) => (head.capabilities[1].risky = true))[0],
    /capability "ui\.osd" changed \(risky: false → true\)/,
  );
  assert.match(
    change((head) => head.capabilities.splice(1, 1))[0],
    /capability "ui\.osd" is removed/,
  );
  assert.match(
    change((head) =>
      head.capabilities.unshift(head.capabilities.splice(1, 1)[0]),
    )[0],
    /existing entries changed order/,
  );
  assert.match(
    change((head) => head.uncapable.pop())[0],
    /capability-free method "window\.state" is removed/,
  );
  assert.match(
    change((head) => (head.apiVersion = "vestra/2"))[0],
    /top-level "apiVersion" changed/,
  );
  // A new capability may not take over a method that exists.
  assert.match(
    change((head) =>
      head.capabilities[2].methods.push({ name: "host.run", needs: "mpv" }),
    )[0],
    /new capability "mpv" takes the existing method "host\.run"/,
  );
  assert.match(
    change((head) =>
      head.capabilities[2].methods.push({ name: "commands.*", needs: "mpv" }),
    )[0],
    /takes the existing method "commands\.register" \(by "commands\.\*"\)/,
  );
});

test("the SDK's copy of the table is read as data", () => {
  const value = parseCapabilitiesTs(String(SDK["src/host-capabilities.ts"]));
  assert.deepEqual(value, JSON.parse(table()));
  assert.throws(() => parseCapabilitiesTs("export const X = 1;"));
});

test("an SDK branch that only adds passes every gate", () => {
  const repo = repository();
  const base = repo.commit(SDK);
  const head = repo.commit({
    "src/host-capabilities.ts": String(SDK["src/host-capabilities.ts"]).replace(
      table(),
      table([MPV]),
    ),
    "src/mpv.ts": "export const mpv = 1;\n",
    "test/fixtures/api-surface.txt":
      SDK["test/fixtures/api-surface.txt"] + "sdk InterfaceDeclaration Mpv\n",
    "test/fixtures/traces/mpv.json": "{}\n",
    "test/mpv.test.ts": 'test("mpv", () => {});\n',
    "test/hello.test.ts": SDK["test/hello.test.ts"] + 'test("b", () => {});\n',
  });
  const result = runGates({ repo: repo.root, base, head });
  assert.equal(result.kind, "sdk");
  assert.ok(result.pass, format(result));
  assert.deepEqual(
    result.gates.map((gate: Gate) => gate.status),
    ["pass", "pass", "pass", "pass"],
  );
  assert.deepEqual(gateOf(result, "d").notes, ['new capability "mpv"']);
  assert.match(format(result), /RESULT: PASS$/);
  // A base against itself.
  assert.ok(runGates({ repo: repo.root, base, head: base }).pass);
});

test("an SDK branch that rewrites what exists fails each gate by name", () => {
  const repo = repository();
  const base = repo.commit(SDK);
  const head = repo.commit({
    "src/host-capabilities.ts": String(SDK["src/host-capabilities.ts"]).replace(
      '"label": "Add menu commands"',
      '"label": "Commands"',
    ),
    "test/fixtures/api-surface.txt": "sdk InterfaceDeclaration App\n",
    "test/fixtures/traces/hello.json": '{"requests":[]}\n',
    "test/hello.test.ts": 'test("a", () => {\n  assert.equal(1, 2);\n});\n',
  });
  const result = runGates({ repo: repo.root, base, head });
  assert.equal(result.pass, false);
  assert.deepEqual(gateOf(result, "a").problems, [
    "test/fixtures/traces/hello.json: modified",
  ]);
  assert.equal(gateOf(result, "b").problems.length, 1);
  assert.match(
    gateOf(result, "b").problems[0],
    /^test\/hello\.test\.ts: 1 line\(s\) removed or modified\n\s+2: assert\.equal\(1, 1\);/,
  );
  assert.deepEqual(gateOf(result, "c").problems, [
    "removed or changed: sdk InterfaceDeclaration App :: system(): Promise<SystemInfo>;",
  ]);
  assert.match(
    gateOf(result, "d").problems[0],
    /capability "commands" changed \(label:/,
  );
  assert.match(format(result), /RESULT: FAIL \(\(a\) \(b\) \(c\) \(d\)\)$/);
});

test("deleted and renamed test files and traces fail", () => {
  const repo = repository();
  const base = repo.commit(SDK);
  const head = repo.commit({
    "test/hello.test.ts": null,
    "test/renamed.test.ts": SDK["test/hello.test.ts"],
    "test/fixtures/traces/hello.json": null,
  });
  const result = runGates({ repo: repo.root, base, head });
  assert.deepEqual(gateOf(result, "a").problems, [
    "test/fixtures/traces/hello.json: deleted",
  ]);
  assert.deepEqual(gateOf(result, "b").problems, [
    "test/hello.test.ts: deleted",
  ]);
});

test("the player: Rust test modules, test directories, the JSON table", () => {
  const repo = repository();
  const base = repo.commit(PLAYER);
  // Additions: a line in the production half, a new test in the module, a
  // new capability.
  const added = repo.commit({
    "src/host/capabilities.json": table([MPV]),
    "src/host/src/math.rs": RUST.replace(
      "    x * 2\n",
      "    // twice\n    x * 2\n",
    ).replace(
      "        assert_eq!(double(2), 4);\n    }\n",
      "        assert_eq!(double(2), 4);\n    }\n\n    #[test]\n    fn zero() {\n        assert_eq!(double(0), 0);\n    }\n",
    ),
    "src/host/tests/mpv.rs": "#[test]\nfn mpv() {}\n",
  });
  const good = runGates({ repo: repo.root, base, head: added });
  assert.equal(good.kind, "player");
  assert.ok(good.pass, format(good));
  assert.deepEqual(
    good.gates.map((gate: Gate) => gate.status),
    ["n/a", "pass", "n/a", "pass"],
  );

  // Production code may change; the expectation in the module may not.
  const edited = repo.commit({
    "src/host/src/math.rs": RUST.replace("x * 2", "x + x").replace(
      "assert_eq!(double(2), 4);",
      "assert_eq!(double(2), 5);",
    ),
    "src/host/tests/lazy.rs": "#[test]\nfn parks() {\n    assert!(false);\n}\n",
  });
  const bad = runGates({ repo: repo.root, base, head: edited });
  const problems = gateOf(bad, "b").problems;
  assert.equal(problems.length, 2, format(bad));
  assert.match(
    problems[0],
    /^src\/host\/src\/math\.rs: 1 line\(s\) under #\[cfg\(test\)\] removed or modified\n\s+11: assert_eq!\(double\(2\), 4\);/,
  );
  assert.match(problems[1], /^src\/host\/tests\/lazy\.rs: 1 line\(s\)/);

  // Only the production half changed: passes.
  const production = repo.commit({
    "src/host/src/math.rs": RUST.replace("x * 2", "x + x"),
    "src/host/tests/lazy.rs": PLAYER["src/host/tests/lazy.rs"],
  });
  assert.ok(runGates({ repo: repo.root, base, head: production }).pass);

  // A Rust file deleted with its test module.
  const deleted = repo.commit({ "src/host/src/math.rs": null });
  assert.match(
    gateOf(runGates({ repo: repo.root, base, head: deleted }), "b").problems[0],
    /math\.rs: deleted with its #\[cfg\(test\)\] code \(lines 5-13\)/,
  );
});

test("the working tree is compared when no head is given; --allow lists, not fails", () => {
  const repo = repository();
  const base = repo.commit(PLAYER);
  assert.ok(runGates({ repo: repo.root, base }).pass);
  repo.write({
    "src/plugins/store/tests/dialog.test.ts":
      'test("lists risky", () => {});\n',
    "src/host/tests/untracked_new.rs": "#[test]\nfn new() {}\n",
  });
  const result = runGates({ repo: repo.root, base });
  assert.equal(result.pass, false);
  assert.match(
    gateOf(result, "b").problems[0],
    /^src\/plugins\/store\/tests\/dialog\.test\.ts: 1 line/,
  );
  const allowed = runGates({
    repo: repo.root,
    base,
    allow: ["src/plugins/store/tests/"],
  });
  assert.ok(allowed.pass);
  assert.match(
    format(allowed),
    /allowed by --allow: src\/plugins\/store\/tests\/dialog/,
  );
});

test("the base is the merge base unless --exact-base", () => {
  const repo = repository();
  const base = repo.commit(SDK);
  const branch = repo.commit({ "test/new.test.ts": 'test("n", () => {});\n' });
  // The base ref moves on (a sibling commit on top of `base`).
  const git = (...args: string[]): string =>
    spawnSync("git", ["-C", repo.root, ...args], {
      encoding: "utf8",
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: "t",
        GIT_AUTHOR_EMAIL: "t@test.invalid",
        GIT_COMMITTER_NAME: "t",
        GIT_COMMITTER_EMAIL: "t@test.invalid",
      },
    }).stdout.trim();
  const blob = spawnSync(
    "git",
    ["-C", repo.root, "hash-object", "-w", "--stdin"],
    {
      encoding: "utf8",
      input: 'test("a", () => {\n  assert.equal(2, 2);\n});\n',
    },
  ).stdout.trim();
  git("read-tree", base);
  git("update-index", "--cacheinfo", `100644,${blob},test/hello.test.ts`);
  const moved = git(
    "commit-tree",
    git("write-tree"),
    "-p",
    base,
    "-m",
    "main moves",
  );
  git("read-tree", branch);
  const merged = runGates({ repo: repo.root, base: moved, head: branch });
  assert.ok(merged.pass, format(merged));
  assert.equal(merged.base.commit, base);
  const exact = runGates({
    repo: repo.root,
    base: moved,
    head: branch,
    exactBase: true,
  });
  assert.equal(exact.pass, false);
});

test("the command exits 0, 1 or 2", () => {
  const repo = repository();
  const base = repo.commit(SDK);
  const head = repo.commit({ "test/hello.test.ts": "changed\n" });
  const script = join(
    import.meta.dirname,
    "..",
    "..",
    "scripts",
    "verify-gates.mjs",
  );
  const run = (...args: string[]) =>
    spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  const pass = run("--repo", repo.root, "--base", base, "--head", base);
  assert.equal(pass.status, 0, pass.stderr);
  assert.match(pass.stdout, /\[PASS\] \(a\)/);
  const fail = run(
    "--repo",
    repo.root,
    "--base",
    base,
    "--head",
    head,
    "--json",
  );
  assert.equal(fail.status, 1);
  assert.equal(JSON.parse(fail.stdout).pass, false);
  assert.equal(run("--repo", repo.root).status, 2);
  assert.equal(run("--repo", repo.root, "--base", "no-such-ref").status, 2);
});
