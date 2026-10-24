// The SDK package (installed from the vestra-player-sdk repository) repeats a
// few of the player's constants and its Slint library; they must agree with
// this host. A mismatch means: change the SDK repository, release it and
// upgrade the dependency here (doc/process/community-workflow.md §1.6).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  API_VERSION,
  CAPABILITIES,
  Catalog,
  COMMAND_LIMITS,
  HOST_ZH_HANS,
  LOCALES,
  MANIFEST_FIELDS,
  MANIFEST_LIMITS,
  MARKDOWN_LIMITS,
  MENU_IDS,
  MENU_KEYS,
  parsePo,
  rewriteSlint,
  scanSlint,
} from "@vestra-player/sdk/runtime";
import { TestHost } from "@vestra-player/sdk/testing";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

test("COMMAND_LIMITS matches the player's registry", () => {
  const rust = host("src/commands.rs");
  const constant = (name: string) =>
    Number(new RegExp(`const ${name}: usize = (\\d+);`).exec(rust)?.[1]);
  assert.equal(constant("MAX_PER_OWNER"), COMMAND_LIMITS.perPlugin);
  assert.equal(constant("MAX_COMMANDS"), COMMAND_LIMITS.total);
});

test("MENU_IDS are the player's built-in menu ids", () => {
  const table = /pub const MENU_ANCHORS: [^=]+= \[([\s\S]*?)\];/.exec(
    host("src/commands.rs"),
  );
  assert.ok(table, "MENU_ANCHORS in commands.rs");
  const ids = [...table[1]!.matchAll(/\("([^"]+)",/g)].map((m) => m[1]);
  assert.deepEqual(ids, [...MENU_IDS]);
});

type Step = {
  source: "decoded" | "generated" | "loading";
  stage: "video" | "audio";
  nodes: unknown[];
  offsetMs?: number;
  expect: string;
  resources: number;
};

// host/src/processing.rs runs the same cases against the player's own
// processing book (conformance_cases_shared_with_the_sdk_test_host).
test("TestHost answers processing changes like the player", async () => {
  const fixture = JSON.parse(
    host("tests/fixtures/processing-conformance.json"),
  ) as { cases: { name: string; steps: Step[] }[] };
  for (const { name, steps } of fixture.cases) {
    const fake = new TestHost();
    for (const [index, step] of steps.entries()) {
      fake.setSource(
        step.source === "loading"
          ? { phase: "loading" }
          : {
              phase: "ready",
              kind: step.source === "generated" ? "producer" : "media",
            },
      );
      const outcome = await fake
        .request("processing.replace", {
          stage: step.stage,
          resourceId: "r",
          revision: index + 1,
          expectedGeneration: fake.source.stamp.generation,
          nodes: step.nodes as never,
          ...(step.offsetMs === undefined ? {} : { offsetMs: step.offsetMs }),
        })
        .then(
          () => "applied",
          (error: { code?: string }) => error.code,
        );
      assert.equal(outcome, step.expect, `${name}, step ${index}`);
      const held = [...fake.processing.resources.values()].filter(
        (resource) => resource.stage === step.stage,
      ).length;
      assert.equal(held, step.resources, `${name}, step ${index}`);
    }
    fake.close();
  }
});

test("capabilities and apiVersion match the player's manifest rules", () => {
  const rust = host("src/plugins.rs");
  const list = /pub const CAPABILITIES: &\[&str\] = &\[([^\]]*)\];/.exec(rust);
  assert.ok(list, "CAPABILITIES in plugins.rs");
  const names = [...list[1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(names, [...CAPABILITIES]);
  assert.equal(/pub const API: &str = "([^"]+)";/.exec(rust)?.[1], API_VERSION);
});

test("the installed SDK's Slint library copy matches the player's", () => {
  // Every file of the library is in the copy (the script lists them).
  const library = readdirSync(
    new URL("../../host/ui/vestra/", import.meta.url),
  );
  const script = readFileSync(
    new URL("../../scripts/sync-sdk-slint.mjs", import.meta.url),
    "utf8",
  );
  for (const file of library)
    assert.ok(script.includes(`"vestra/${file}"`), `${file} is synced`);
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(
        new URL("../../scripts/sync-sdk-slint.mjs", import.meta.url),
      ),
      "--check",
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
});

test("manifest rules match the player's (host/src/manifest.rs)", () => {
  const rust = host("src/manifest.rs");
  const constant = (name: string) => {
    const match = new RegExp(`pub const ${name}: \\w+ = ([^;]+);`).exec(rust);
    assert.ok(match, name);
    return Number(
      match[1]!
        .split("*")
        .map((part) => Number(part.trim().replaceAll("_", "")))
        .reduce((a, b) => a * b, 1),
    );
  };
  assert.deepEqual(
    {
      idBytes: constant("ID_MAX"),
      name: constant("NAME_MAX"),
      author: constant("AUTHOR_MAX"),
      punchline: constant("PUNCHLINE_MAX"),
      description: constant("DESCRIPTION_MAX"),
      tags: constant("TAGS_MAX"),
      tag: constant("TAG_MAX"),
      license: constant("LICENSE_MAX"),
      urlBytes: constant("URL_MAX"),
      iconBytes: constant("ICON_BYTES_MAX"),
      iconSide: constant("ICON_SIDE_MAX"),
      documentBytes: constant("DOCUMENT_BYTES_MAX"),
    },
    { ...MANIFEST_LIMITS },
  );
  // The Rust struct's fields (camelCase) are the fields the SDK accepts.
  const struct = /pub struct Manifest \{([\s\S]*?)\n\}/.exec(rust);
  assert.ok(struct, "struct Manifest");
  const fields = [...struct[1]!.matchAll(/pub (\w+):/g)].map((m) =>
    m[1]!.replace(/_(\w)/g, (_, c: string) => c.toUpperCase()),
  );
  assert.deepEqual([...fields].sort(), [...MANIFEST_FIELDS].sort());
});

test("Markdown limits match the player's (host/src/ui_markdown.rs)", () => {
  const text = /pub const MAX_TEXT: usize = ([^;]+);/.exec(
    host("src/ui_markdown.rs"),
  )?.[1];
  assert.ok(text, "MAX_TEXT");
  const bytes = text
    .split("*")
    .map((part) => Number(part.trim()))
    .reduce((a, b) => a * b, 1);
  assert.equal(bytes, MARKDOWN_LIMITS.textBytes);
});

// ---------------------------------------------------------------- i18n

test("LOCALES and the menu keys are the player's (host/src/i18n, commands.rs)", () => {
  const locales = /pub const LOCALES: \[&str; \d+\] = \[([^\]]*)\];/.exec(
    host("src/i18n/mod.rs"),
  );
  assert.ok(locales, "LOCALES in i18n/mod.rs");
  assert.deepEqual(
    [...locales[1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    [...LOCALES],
  );
  const anchors = /pub const MENU_ANCHORS: [^=]+= \[([\s\S]*?)\];/.exec(
    host("src/commands.rs"),
  );
  assert.ok(anchors, "MENU_ANCHORS in commands.rs");
  const keys = Object.fromEntries(
    [...anchors[1]!.matchAll(/\("([^"]+)", "([^"]+)"/g)].map((m) => [
      m[1],
      m[2],
    ]),
  );
  assert.deepEqual(keys, { ...MENU_KEYS });
});

// host/src/i18n/tests.rs runs the same cases against the player's scanner.
test("the SDK's Slint @tr scanner agrees with the player's", () => {
  const fixture = JSON.parse(host("tests/fixtures/slint-tr.json")) as {
    cases: {
      name: string;
      source: string;
      catalog: Record<string, string | [string, string]>;
      messages: { id: string; context?: string; plural?: string }[];
      output: string;
    }[];
  };
  for (const item of fixture.cases) {
    const found = scanSlint(item.source).map(({ id, context, plural }) => ({
      id,
      ...(context === undefined ? {} : { context }),
      ...(plural === undefined ? {} : { plural }),
    }));
    assert.deepEqual(found, item.messages, `${item.name}: messages`);
    assert.equal(
      rewriteSlint(item.source, new Catalog(item.catalog)),
      item.output,
      `${item.name}: output`,
    );
  }
});

test("the test host's copy of the player's Chinese texts matches locales/zh-Hans.po", () => {
  const { entries } = parsePo(host("locales/zh-Hans.po"));
  const player = new Map(entries.map((e) => [e.id, e.strings[0]]));
  for (const [english, chinese] of Object.entries(HOST_ZH_HANS))
    assert.equal(player.get(english), chinese, english);
});
