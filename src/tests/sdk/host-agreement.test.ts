// The SDK package (installed from the vestra-player-sdk repository) repeats a
// few of the player's constants and its Slint library; they must agree with
// this host. A mismatch means: change the SDK repository, release it and
// upgrade the dependency here (agentic-workflow/agentic-workflow.md §1.6).
//
// Contract data (what decides whether the player accepts a request, or how a
// plugin or TestHost behaves: capabilities, apiVersion, limits, error codes,
// wire shapes, menu ids, wake events, the player's own menu paths, the Slint
// library's types) fails a `test`. Product data (a snapshot of how the
// player looks now: the rest of the built-in command and submenu tables, the
// Slint library copy byte for byte, the player's Chinese texts, palette
// colours) is an `advisory` test: between SDK releases a failed comparison
// is reported as a TODO and does not fail the run (any other error does). VESTRA_SDK_STRICT=1 (`npm run
// ci:local -- --strict-sdk`, the release workflow) makes it a plain test;
// that is the check when the SDK dependency is upgraded and before a release.
import assert from "node:assert/strict";
import {
  appendFileSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { TestContext } from "node:test";
import { fileURLToPath } from "node:url";
import {
  API_VERSION,
  BAR_BUTTON_ICONS,
  BAR_BUTTON_LIMITS,
  BINDING_INPUTS,
  BUILTIN_COMMANDS,
  BUILTIN_SUBMENUS,
  CAPABILITIES,
  CAPABILITY_INFO,
  Catalog,
  COMMAND_LIMITS,
  HOST_CAPABILITIES,
  HOST_ZH_HANS,
  IMAGE_LIMITS,
  LOCALES,
  MANIFEST_FIELDS,
  MANIFEST_LIMITS,
  MARKDOWN_LIMITS,
  MEDIA_SOURCE_LIMITS,
  MENU_IDS,
  MENU_KEYS,
  MESSAGE_LIMIT,
  OSD_LIMITS,
  parsePo,
  PATCH_FEATURE,
  PATCH_OPS,
  rewriteSlint,
  scanSlint,
  SECRET_LIMITS,
  SUBTITLE_LIMITS,
  THEME_PALETTES,
  TIMELINE_LIMITS,
  UI_VALUE_LIMITS,
  LIST_SEPARATORS,
  TEXT_LIMITS,
  VIDEO_PROCESSING_LIMITS,
} from "@vestra-player/sdk/runtime";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { CAPABILITIES as STORE_CAPABILITIES } from "../../plugins/store/labels.ts";
import { runNode } from "./children.ts";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

const STRICT = process.env.VESTRA_SDK_STRICT === "1";
const ADVISORY =
  "product data: warns until the next SDK sync; VESTRA_SDK_STRICT=1 enforces it";

/**
 * Whether an advisory test's failure is a product-data difference: only a
 * failed comparison (node:assert's AssertionError). Anything else (a
 * TypeError, a plugin that throws on start, a reader that no longer finds
 * its Rust or Slint, see `found`) is a broken check and fails in both modes.
 */
const productDifference = (error: unknown) =>
  error instanceof assert.AssertionError;

/**
 * `value`, or a plain Error (not an AssertionError, so an advisory test
 * fails too) when a reader did not find what it reads.
 */
function found<T>(value: T | null | undefined | false, what: string): T {
  if (value === null || value === undefined || value === false)
    throw new Error(`not found: ${what} (adjust the reader)`);
  return value;
}

/**
 * A product-data test: with VESTRA_SDK_STRICT=1 a plain test; otherwise a
 * failed comparison is shown as a TODO (the run passes) and, when
 * VESTRA_SDK_ADVISORY_FILE names a file, appended to it as one line
 * (ci:local prints them at the end). Other errors fail (productDifference).
 */
function advisory(name: string, fn: (t: TestContext) => void | Promise<void>) {
  if (STRICT) return test(name, fn);
  return test(name, async (t) => {
    try {
      await fn(t);
    } catch (error) {
      if (!productDifference(error)) throw error;
      t.todo(ADVISORY);
      const file = process.env.VESTRA_SDK_ADVISORY_FILE;
      if (file) {
        const message = error instanceof Error ? error.message : String(error);
        appendFileSync(file, `test:sdk: ${name}: ${message.split("\n")[0]}\n`);
      }
      throw error;
    }
  });
}

test("advisory tests downgrade only failed comparisons", () => {
  const thrown = (fn: () => unknown) => {
    try {
      fn();
    } catch (error) {
      return error;
    }
    throw new Error("nothing thrown");
  };
  assert.equal(productDifference(thrown(() => assert.equal(1, 2))), true);
  assert.equal(
    productDifference(thrown(() => Object.entries(null as never))),
    false,
  );
  assert.equal(productDifference(thrown(() => found(null, "x"))), false);
  assert.equal(productDifference(new Error("crash")), false);
});

test("COMMAND_LIMITS matches the player's registry", () => {
  const rust = host("src/commands.rs");
  const constant = (name: string) =>
    Number(new RegExp(`const ${name}: usize = (\\d+);`).exec(rust)?.[1]);
  assert.equal(constant("MAX_PER_OWNER"), COMMAND_LIMITS.perPlugin);
  assert.equal(constant("MAX_COMMANDS"), COMMAND_LIMITS.total);
});

test("UI_VALUE_LIMITS match the player's (host/src/ui/ui_values.rs)", () => {
  const rust = host("src/ui/ui_values.rs");
  const constant = (name: string) => {
    const value = new RegExp(`pub const ${name}: usize = ([^;]+);`).exec(
      rust,
    )?.[1];
    assert.ok(value, name);
    return value
      .split("*")
      .map((part) => Number(part.trim()))
      .reduce((a, b) => a * b, 1);
  };
  assert.deepEqual(
    {
      depth: constant("MAX_DEPTH"),
      values: constant("MAX_NODES"),
      arrayRows: constant("MAX_ARRAY"),
      stringBytes: constant("MAX_STRING_BYTES"),
      textBytes: constant("MAX_TEXT_BYTES"),
    },
    { ...UI_VALUE_LIMITS },
  );
});

test("row patches: the SDK's op limit and feature name match the player's (host/src/ui/ui_patch.rs)", () => {
  const rust = host("src/ui/ui_patch.rs");
  assert.equal(
    Number(/pub const MAX_PATCH_OPS: usize = (\d+);/.exec(rust)?.[1]),
    PATCH_OPS,
  );
  assert.equal(
    /pub const FEATURE: &str = "([^"]+)";/.exec(rust)?.[1],
    PATCH_FEATURE,
  );
  // The player advertises it in plugin.app's start-up data.
  assert.match(host("src/main.rs"), /"features":\[ui::ui_patch::FEATURE\]/);
});

test("TEXT_LIMITS and LIST_SEPARATORS match the player's (host/src/i18n/text.rs)", () => {
  const rust = host("src/i18n/text.rs");
  const constant = (name: string) =>
    Number(new RegExp(`pub const ${name}: usize = (\\d+);`).exec(rust)?.[1]);
  assert.deepEqual(
    {
      depth: constant("MAX_DEPTH"),
      nodes: constant("MAX_NODES"),
      args: constant("MAX_ARGS"),
      items: constant("MAX_ITEMS"),
      stringBytes: constant("MAX_STRING_BYTES"),
    },
    { ...TEXT_LIMITS },
  );
  const styles = JSON.parse(
    /pub const STYLES: \[&str; 5\] = (\[[^\]]*\]);/.exec(rust)![1]!,
  ) as string[];
  const unescape = (text: string) =>
    text
      .replace(/\\u\{([0-9a-f]+)\}/g, (_, hex: string) =>
        String.fromCodePoint(Number.parseInt(hex, 16)),
      )
      .replaceAll("\\n", "\n");
  const rows = Object.fromEntries(
    [...rust.matchAll(/\("([^"]+)", \[([^\]]*)\]\)/g)].map(
      ([, locale, list]) => [
        locale,
        Object.fromEntries(
          [...list!.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map(([, sep], i) => [
            styles[i]!,
            unescape(sep!),
          ]),
        ),
      ],
    ),
  );
  assert.deepEqual(rows, LIST_SEPARATORS);
});

test("BAR_BUTTON_ICONS and BAR_BUTTON_LIMITS match the player's (host/src/commands.rs)", () => {
  const rust = host("src/commands.rs");
  const table = /pub const BUTTON_ICONS: [^=]+= \[([\s\S]*?)\];/.exec(rust);
  assert.ok(table, "BUTTON_ICONS in commands.rs");
  const icons = [...table[1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(icons, [...BAR_BUTTON_ICONS]);
  const constant = (name: string) =>
    Number(new RegExp(`const ${name}: \\w+ = (\\d+);`).exec(rust)?.[1]);
  assert.deepEqual(
    {
      perPlugin: constant("MAX_BUTTONS_PER_OWNER"),
      total: constant("MAX_BUTTONS"),
      maxOrder: constant("MAX_BUTTON_ORDER"),
    },
    { ...BAR_BUTTON_LIMITS },
  );
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

// host/src/playback/processing.rs runs the same cases against the player's own
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

test("capabilities and apiVersion match the player's manifest rules (host/capabilities.json)", () => {
  const json = capabilityJson();
  assert.deepEqual(
    json.capabilities.map((c) => c.name),
    [...CAPABILITIES],
  );
  assert.equal(json.apiVersion, API_VERSION);
});

test("the installed SDK's copy of host/capabilities.json is current (SDK npm run capabilities:sync)", () => {
  // CAPABILITY_INFO, vestra-plugin check and the SDK's TypeScript plugin
  // are built from this copy: a capability change here needs an SDK release.
  assert.deepEqual(HOST_CAPABILITIES, capabilityJson());
});

test("every file of the Slint library is in sync-sdk-slint.mjs's SLINT_FILES", () => {
  const library = readdirSync(
    new URL("../../host/ui/vestra/", import.meta.url),
  );
  const script = readFileSync(
    new URL("../../scripts/sync-sdk-slint.mjs", import.meta.url),
    "utf8",
  );
  assert.ok(script.includes(`"palette.slint"`), "palette.slint is synced");
  for (const file of library)
    assert.ok(script.includes(`"vestra/${file}"`), `${file} is synced`);
});

advisory("the installed SDK's Slint library copy matches the player's", () => {
  const result = runNode([
    fileURLToPath(new URL("../../scripts/sync-sdk-slint.mjs", import.meta.url)),
    "--strict",
  ]);
  // Exit 1 with the script's own message: files differ (product data). Any
  // other outcome (a crash also exits 1, without that message) is a failure.
  const differs =
    result.status === 1 &&
    /The installed SDK's Slint copy .* differs from host\/ui: /.test(
      result.stderr,
    );
  if (result.status !== 0 && !differs)
    throw new Error(
      `sync-sdk-slint.mjs --strict: exit ${result.status}\n${result.stderr}`,
    );
  assert.equal(result.status, 0, result.stderr);
});

// TestHost resolves `@vestra/…` imports from the SDK's copy and checks the
// values a plugin sets against the library's structs and properties
// (SDK dist/plugin/testing/slint-types.js), so that surface is contract in
// one direction: what the SDK copy declares that the player lacks or
// declares differently would let TestHost accept a value the player
// refuses. What only the player declares (a new property or struct field)
// makes TestHost stricter, and plugins need a new SDK to use it anyway: that
// waits for the next SDK release (advisory). The rest of the copy (bodies,
// defaults, colours) is the byte comparison above.

/** Per SLINT_FILES file, the type surface of host/ui's file and the SDK copy's. */
function slintSurfaces() {
  const script = readFileSync(
    new URL("../../scripts/sync-sdk-slint.mjs", import.meta.url),
    "utf8",
  );
  const list = found(
    /const SLINT_FILES = \[([^\]]*)\]/.exec(script)?.[1],
    "SLINT_FILES in sync-sdk-slint.mjs",
  );
  const files = [...list.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
  found(files.length > 0 && files, "files in SLINT_FILES");
  const sdkSlint = (file: string) =>
    readFileSync(
      new URL(
        `../../node_modules/@vestra-player/sdk/slint/${file}`,
        import.meta.url,
      ),
      "utf8",
    );
  let total = 0;
  const surfaces = files.map((file) => {
    const player = slintSurface(host(`ui/${file}`));
    total += player.length;
    found(player.length > 0 && player, `exported declarations in ${file}`);
    // TestHost takes what it cannot resolve as any value: an empty or
    // unreadable copy would accept everything.
    const sdk = slintSurface(sdkSlint(file));
    found(
      sdk.length > 0 && sdk,
      `exported declarations in the SDK copy of ${file}`,
    );
    return { file, player, sdk };
  });
  // The reader still reads the library (a struct field, a property).
  const markdown = slintSurface(host("ui/vestra/markdown.slint"));
  found(
    markdown.includes("struct MarkdownRow.header: bool"),
    "struct MarkdownRow's fields in markdown.slint",
  );
  found(
    markdown.includes("MarkdownView.blocks: in [MarkdownBlock]"),
    "MarkdownView's blocks property in markdown.slint",
  );
  found(total > 20 && total, "the library's exported declarations");
  return surfaces;
}

/** The declaration an entry of slintSurface is (`struct X`, `component X`, …), or null for a member. */
function slintDeclaration(entry: string): string | null {
  const m =
    /^(struct|enum) ([\w-]+)$/.exec(entry) ??
    /^(component|global) ([\w-]+)(?: inherits \S+)?$/.exec(entry) ??
    /^(export) (\S+)$/.exec(entry);
  return m ? `${m[1]} ${m[2]}` : null;
}

// A struct, enum, component or global the copy lacks is a type TestHost
// cannot resolve, and it takes that as any value: every exported
// declaration of the player's must be in the copy (a new one needs the SDK
// sync before it merges). Members only the player has stay advisory.
test("the SDK's Slint library copy has every exported declaration of the player's (structs, enums, components, globals)", () => {
  for (const { file, player, sdk } of slintSurfaces()) {
    const have = new Set(sdk.map(slintDeclaration));
    assert.deepEqual(
      player
        .map(slintDeclaration)
        .filter(
          (declaration) => declaration !== null && !have.has(declaration),
        ),
      [],
      `${file}: exported in host/ui/${file} but missing from the SDK copy`,
    );
  }
});

test("the SDK's Slint library copy declares nothing the player lacks or declares differently (structs, components, properties)", () => {
  for (const { file, player, sdk } of slintSurfaces()) {
    const have = new Set(player);
    assert.deepEqual(
      sdk.filter((entry) => !have.has(entry)),
      [],
      `${file}: declared so in the SDK copy only (host/ui/${file} lacks it or declares it otherwise)`,
    );
  }
});

advisory(
  "the SDK's Slint library copy declares the player's additions (structs, components, properties)",
  () => {
    // An entry's key: what stands before its declaration.
    const key = (entry: string) => entry.split(": ")[0]!;
    for (const { file, player, sdk } of slintSurfaces()) {
      const keys = new Set(sdk.map(key));
      assert.deepEqual(
        player.filter((entry) => !keys.has(key(entry))),
        [],
        `${file}: declared in host/ui/${file} only`,
      );
    }
  },
);

/** Slint source without comments, with every string literal emptied (`""`). */
function slintCode(text: string): string {
  const n = text.length;
  let i = 0;
  let out = "";
  // From an opening quote to past its closing one; `\{ … }` interpolations
  // may hold braces and strings of their own.
  const skipString = () => {
    i++;
    while (i < n && text[i] !== '"') {
      if (text[i] === "\\" && text[i + 1] === "{") {
        i += 2;
        let depth = 1;
        while (i < n && depth > 0) {
          if (text[i] === '"') {
            skipString();
            continue;
          }
          if (text[i] === "{") depth++;
          else if (text[i] === "}") depth--;
          i++;
        }
        continue;
      }
      if (text[i] === "\\") i++;
      i++;
    }
    i++;
  };
  while (i < n) {
    if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      i = end < 0 ? n : end;
    } else if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? n : end + 2;
      out += " ";
    } else if (text[i] === '"') {
      skipString();
      out += '""';
    } else out += text[i++];
  }
  return out;
}

/** The index of the `}` that closes the `{` at `open` in slintCode output. */
function slintClose(code: string, open: number): number {
  let depth = 0;
  for (let i = open; i < code.length; i++)
    if (code[i] === "{") depth++;
    else if (code[i] === "}" && --depth === 0) return i;
  throw new Error("unbalanced braces in a Slint file");
}

/**
 * The type surface of one Slint library file, one sorted entry each
 * (`<key>` or `<key>: <declaration>`): every exported struct and each of
 * its fields with its type, exported enum and each value, exported
 * component or global (and its base), and each of their non-private
 * property declarations directly in the body (name, direction, type).
 * Names as Slint normalises them (`_` is `-`).
 */
function slintSurface(text: string): string[] {
  const code = slintCode(text);
  const name = (id: string) => id.replace(/^-/, "_").replace(/(?<!^)_/g, "-");
  const type = (text: string) => name(text.replace(/\s+/g, ""));
  const body = (open: number) => code.slice(open + 1, slintClose(code, open));
  // Split at the commas outside brackets (a field may be `{ a: int, … }`).
  const items = (text: string) => {
    const parts = [""];
    let depth = 0;
    for (const c of text) {
      if ("{[(".includes(c)) depth++;
      else if ("}])".includes(c)) depth--;
      if (c === "," && depth === 0) parts.push("");
      else parts[parts.length - 1] += c;
    }
    return parts.map((part) => part.trim()).filter(Boolean);
  };
  const lines: string[] = [];
  for (const m of code.matchAll(/\bexport\s+struct\s+([\w-]+)\s*\{/g)) {
    const struct = name(m[1]!);
    lines.push(`struct ${struct}`);
    for (const field of items(body(m.index + m[0].length - 1))) {
      const colon = found(
        field.indexOf(":") > 0 && field.indexOf(":"),
        `a field of struct ${struct}: ${field}`,
      );
      lines.push(
        `struct ${struct}.${name(field.slice(0, colon).trim())}: ${type(field.slice(colon + 1))}`,
      );
    }
  }
  for (const m of code.matchAll(/\bexport\s+enum\s+([\w-]+)\s*\{/g)) {
    lines.push(`enum ${name(m[1]!)}`);
    for (const value of items(body(m.index + m[0].length - 1)))
      lines.push(`enum ${name(m[1]!)}::${name(value)}`);
  }
  for (const m of code.matchAll(/\bexport\s*\{([^}]*)\}/g))
    for (const item of items(m[1]!)) lines.push(`export ${type(item)}`);
  for (const m of code.matchAll(
    /\bexport\s+(component|global)\s+([\w-]+)(?:\s+inherits\s+([\w-]+))?\s*\{/g,
  )) {
    const element = name(m[2]!);
    lines.push(`${m[1]} ${element}${m[3] ? ` inherits ${name(m[3])}` : ""}`);
    // Only what stands directly in the body: nested `{ … }` blanked.
    let top = "";
    let depth = 0;
    for (const c of body(m.index + m[0].length - 1)) {
      if (c === "{") depth++;
      if (depth === 0) top += c;
      if (c === "}") depth--;
    }
    for (const p of top.matchAll(
      /(?:^|[;\s])(?:(in-out|in_out|in|out|private)\s+)?property\s*(?:<([^>]*)>)?\s*([\w-]+)/g,
    )) {
      const direction = (p[1] ?? "private").replace("in_out", "in-out");
      if (direction === "private") continue;
      lines.push(
        `${element}.${name(p[3]!)}: ${direction} ${p[2] === undefined ? "(two-way)" : type(p[2])}`,
      );
    }
  }
  return lines.sort();
}

test("manifest rules match the player's (host/src/packages/manifest.rs)", () => {
  const rust = host("src/packages/manifest.rs");
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

test("Markdown limits match the player's (host/src/ui/ui_markdown.rs)", () => {
  const text = /pub const MAX_TEXT: usize = ([^;]+);/.exec(
    host("src/ui/ui_markdown.rs"),
  )?.[1];
  assert.ok(text, "MAX_TEXT");
  const bytes = text
    .split("*")
    .map((part) => Number(part.trim()))
    .reduce((a, b) => a * b, 1);
  assert.equal(bytes, MARKDOWN_LIMITS.textBytes);
});

test("step seek parameters are the player's: setStepSeek keyframes, host.run seconds (SDK 1.5.19)", () => {
  // The SDK sends them only when given, under these names.
  const wire = sdkFile("wire.js");
  assert.match(wire, /"player\.setStepSeek",\s*keyframes === undefined/);
  assert.match(wire, /"host\.run",\s*seconds === undefined/);
  // The player reads them under the same names.
  assert.match(
    host("src/capabilities/playback_control.rs"),
    /params\["keyframes"\]/,
  );
  assert.match(
    host("src/capabilities/commands.rs"),
    /run_step\(action, &params\["seconds"\]\)/,
  );
  assert.match(
    host("src/host/actions.rs"),
    /"keyframes":self\.step_keyframes\(\)/,
  );
});

test("a seek control's keyframes flag is the SDK's Scrub.seek option and the player's Control::Seek field (SDK 1.5.22, player 0.5.3)", () => {
  // The player: an optional boolean on the seek control, false when absent
  // (plugins built on older SDKs never send it), mapped to mpv's seek mode.
  const session = host("src/playback/session.rs");
  assert.match(
    session,
    /Seek \{\s*seconds: f64,\s*#\[serde\(default[^\]]*\)\]\s*keyframes: bool,\s*\}/,
  );
  const backend = host("src/playback/source_backend.rs");
  assert.match(
    backend,
    /if keyframes \{\s*"absolute\+keyframes"\s*\} else \{\s*"absolute\+exact"\s*\}/,
  );
  // The SDK: `Scrub.seek(seconds, { keyframes })` sends the field under that
  // name, and only when true, so older players see the intent they always
  // saw. An installed SDK before 1.5.22 has no such option and sends nothing.
  const player = sdkFile("plugin/player.js");
  const version = JSON.parse(
    readFileSync(
      new URL(
        "../../node_modules/@vestra-player/sdk/package.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ).version as string;
  const atLeast = (a: string, b: string) => {
    const x = a.split(".").map(Number);
    const y = b.split(".").map(Number);
    for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i]! > y[i]!;
    return true;
  };
  const sends =
    /kind: "seek",\s*seconds,\s*\.\.\.\(keyframes \? \{ keyframes \} : \{\}\)/.test(
      player,
    );
  if (atLeast(version, "1.5.22"))
    assert.ok(
      sends,
      `SDK ${version}: Scrub.seek sends keyframes only when true`,
    );
  else
    assert.ok(
      !/kind: "seek"[^}]*keyframes/.test(player),
      `SDK ${version}: a seek control carries no keyframes field`,
    );
});

test("IMAGE_LIMITS match the player's (host/src/ui/ui_image.rs)", () => {
  const source = host("src/ui/ui_image.rs");
  const constant = (name: string) => {
    const value = new RegExp(`pub const ${name}: \\w+ = ([^;]+);`).exec(
      source,
    )?.[1];
    assert.ok(value, name);
    return value
      .split("*")
      .map((part) => Number(part.trim()))
      .reduce((a, b) => a * b, 1);
  };
  assert.deepEqual(
    {
      fileBytes: constant("MAX_FILE_BYTES"),
      side: constant("MAX_SIDE"),
      pluginBytes: constant("BUDGET_BYTES"),
      viewBytes: constant("VIEW_BUDGET_BYTES"),
    },
    { ...IMAGE_LIMITS },
  );
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

advisory(
  "the test host's copy of the player's Chinese texts matches locales/zh-Hans.po",
  () => {
    const { entries } = parsePo(host("locales/zh-Hans.po"));
    const player = new Map(entries.map((e) => [e.id, e.strings[0]]));
    for (const [english, chinese] of Object.entries(HOST_ZH_HANS))
      assert.equal(player.get(english), chinese, english);
  },
);

// ------------------------------------------- tables of the SDK's Reference
//
// The SDK's documentation site generates its Reference pages (capabilities,
// limits, commands, theme) from the constants below; each test names the
// drift it finds. Rust is read as text, so a test also fails when the code it
// reads changes shape: adjust the reader then.

const sdkFile = (file: string) =>
  readFileSync(
    new URL(
      `../../node_modules/@vestra-player/sdk/dist/${file}`,
      import.meta.url,
    ),
    "utf8",
  );

// The plugin API declarations: `plugin/types.d.ts` plus, from SDK 1.6.21, the
// `plugin/types/*.d.ts` files it re-exports (one per documentation category).
const sdkTypes = () => {
  const directory = new URL(
    "../../node_modules/@vestra-player/sdk/dist/plugin/types/",
    import.meta.url,
  );
  let parts: string[] = [];
  try {
    parts = readdirSync(directory)
      .filter((name) => name.endsWith(".d.ts"))
      .sort();
  } catch {
    // SDK ≤ 1.6.20: everything is in types.d.ts.
  }
  return [
    sdkFile("plugin/types.d.ts"),
    ...parts.map((name) => sdkFile(`plugin/types/${name}`)),
  ].join("\n");
};

/** The text inside the `{ ... }` that follows `marker` (strings and comments skipped). */
function braces(text: string, marker: string): string {
  const at = text.indexOf(marker);
  found(at >= 0, marker);
  const open = text.indexOf("{", at + marker.length - 1);
  const close = matching(text, open);
  return text.slice(open + 1, close);
}

/** The index of the bracket that closes the one at `open`. */
function matching(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const c = text[i]!;
    if (c === '"') {
      for (i++; i < text.length && text[i] !== '"'; i++)
        if (text[i] === "\\") i++;
    } else if (c === "/" && text[i + 1] === "/") {
      i = text.indexOf("\n", i);
    } else if ("{([".includes(c)) depth++;
    else if ("})]".includes(c) && --depth === 0) return i;
  }
  throw new Error("unbalanced brackets");
}

/** A method's capability in host/capabilities.json (see its `about`). */
type Needs =
  | null
  | string
  | {
      param: string;
      cases: Record<string, string | null>;
      otherwise: string | null;
    }
  | { ifDeclared: string; then: string; otherwise: string };
interface CapabilityJson {
  name: string;
  label: string;
  risky: boolean;
  requires: string[];
  residency: string;
  methods: { name: string; needs: Needs }[];
}

/**
 * The player's capabilities as host/src/capabilities/ generates them into
 * host/capabilities.json (a host test keeps the file current).
 */
function capabilityJson() {
  return JSON.parse(host("capabilities.json")) as {
    apiVersion: string;
    capabilities: CapabilityJson[];
    uncapable: { name: string; needs: Needs }[];
  };
}

/**
 * The capability the player asks for a request from a plugin that declared
 * `declared` (`null`: none; `undefined`: the player does not know the
 * method), read from host/capabilities.json: an exact method name first,
 * then a `prefix.*` entry.
 */
function dispatchCapabilities() {
  const json = capabilityJson();
  const methods = [
    ...json.capabilities.flatMap((c) => c.methods),
    ...json.uncapable,
  ];
  const evaluate = (
    needs: Needs,
    params: Record<string, string>,
    declared: ReadonlySet<string>,
  ): string | null => {
    if (needs === null || typeof needs === "string") return needs;
    if ("param" in needs) {
      const value = params[needs.param];
      return value !== undefined && Object.hasOwn(needs.cases, value)
        ? needs.cases[value]!
        : needs.otherwise;
    }
    return declared.has(needs.ifDeclared) ? needs.then : needs.otherwise;
  };
  const capability = (
    method: string,
    params: Record<string, string> = {},
    declared: ReadonlySet<string> = new Set(),
  ) => {
    const entry =
      methods.find((m) => m.name === method) ??
      methods.find(
        (m) => m.name.endsWith(".*") && method.startsWith(m.name.slice(0, -1)),
      );
    return entry ? evaluate(entry.needs, params, declared) : undefined;
  };
  return {
    capability,
    methods: methods.map((m) => m.name).filter((name) => !name.endsWith("*")),
  };
}

test("capabilities.json's wording and requirements are CAPABILITY_INFO's", () => {
  const json = capabilityJson();
  assert.deepEqual(
    json.capabilities.map((c) => c.name),
    Object.keys(CAPABILITY_INFO),
    "capabilities.json lists other capabilities, or in another order, than CAPABILITY_INFO",
  );
  for (const c of json.capabilities) {
    const info = CAPABILITY_INFO[c.name]!;
    assert.deepEqual(
      { label: c.label, risky: c.risky, requires: c.requires },
      { label: info.label, risky: info.risky, requires: [...info.requires] },
      `${c.name}: capabilities.json vs CAPABILITY_INFO`,
    );
  }
});

// Both are this repository's files, so the store is checked against the
// player directly (CAPABILITY_INFO is compared with capabilities.json above).
test("the store's capability wording is capabilities.json's (plugins/store/labels.ts)", () => {
  const store: Record<string, { text: string; risky?: boolean }> =
    STORE_CAPABILITIES;
  const json = capabilityJson();
  assert.deepEqual(
    Object.keys(store).sort(),
    json.capabilities.map((c) => c.name).sort(),
    "the store labels another set of capabilities than capabilities.json",
  );
  for (const c of json.capabilities) {
    assert.equal(
      store[c.name]!.text,
      c.label,
      `${c.name}: store text vs capabilities.json label`,
    );
    assert.equal(
      store[c.name]!.risky === true,
      c.risky,
      `${c.name}: store risky vs capabilities.json risky`,
    );
  }
});

/**
 * The requests each member in CAPABILITY_INFO.apis sends, read from the
 * SDK's source (`runtime.host.*` calls in src/plugin/*.ts, wire.ts). `event:x`
 * is `events.subscribe` of event `x`; `method:video` is a processing request
 * for that stage; a trailing `?` marks a best-effort call whose failure the
 * member ignores.
 */
const CONTROL = ["source.submit", "source.operation", "source.state?"];
const LAYOUT = ["event:window.state", "layout.state"];
const MEMBER_REQUESTS: Record<string, readonly string[]> = {
  "Player.openStream": [
    "producer.prepare",
    "producer.state",
    "producer.ack",
    "producer.ackInput",
    "producer.release",
    "source.submit",
  ],
  "Player.state": ["source.state"],
  "Player.refresh": ["source.state"],
  "Player.stats": ["playback.stats"],
  "Player.chapters": ["player.chapters"],
  "Player.audioDevices": ["player.audioDevices"],
  "Timeline.marks": ["timeline.marks"],
  "Timeline.ranges": ["timeline.marks"],
  "Player.open": CONTROL,
  "Player.setPaused": CONTROL,
  "Player.seek": CONTROL,
  "Player.setRate": CONTROL,
  "Player.restart": CONTROL,
  "Player.close": CONTROL,
  "Player.scrub": CONTROL,
  "Player.setOutput": ["playback.set", "source.state?"],
  "Player.provideSequence": ["sequence.set"],
  "Player.next": ["sequence.step"],
  "Player.previous": ["sequence.step"],
  "Player.addAudioTrack": ["audio.addTrack"],
  "Player.selectAudioTrack": ["audio.selectTrack"],
  "Player.step": ["player.step"],
  "Player.setStepSeek": ["player.setStepSeek"],
  "Player.setLoop": ["player.setLoop"],
  "Player.setAudioDevice": ["player.setAudioDevice"],
  "Power.preventSleep": ["power.preventSleep"],
  "Resolvers.add": [
    "sourceResolvers.register",
    "sourceResolvers.respond",
    "sourceResolvers.unregister",
  ],
  "Player.onChange": ["event:source.state"],
  "Subtitles.onChange": [
    "event:source.state",
    "event:playback.backend",
    "subtitles.state",
  ],
  "MainWindow.onChange": ["event:window.state"],
  "PanelSlot.onChange": LAYOUT,
  "Panel.onChange": LAYOUT,
  "BackdropSlot.onChange": LAYOUT,
  "Commands.add": [
    "commands.register",
    "commands.update",
    "commands.unregister",
  ],
  // SDK 1.6.7, player 0.7.6: the labels of the plugin's own commands.
  "Commands.keys": ["commands.keys", "event:commands.keys"],
  "Host.run": ["host.run", "source.state?"],
  "Windows.create": ["ui.open", "ui.set", "ui.show", "ui.close"],
  "Panels.mount": [
    "ui.mount",
    "ui.setMount",
    "ui.interaction",
    "ui.menu",
    "ui.unmount",
  ],
  "Panels.bottom": ["layout.state", "layout.set"],
  "Panels.sidebar": ["layout.state", "layout.set"],
  "Panels.backdrop": ["layout.state"],
  "Dialogs.confirm": ["dialogs.confirm"],
  "Dialogs.openFile": ["dialogs.open"],
  "Dialogs.openFiles": ["dialogs.open"],
  "Dialogs.openDirectory": ["dialogs.open"],
  "Dialogs.saveFile": ["dialogs.open"],
  "Packages.validate": [
    "packages.validate",
    "packages.install",
    "packages.cancel",
  ],
  "Packages.validateArchive": [
    "packages.validateArchive",
    "packages.install",
    "packages.cancel",
  ],
  "Packages.setEnabled": ["packages.setEnabled"],
  // SDK 1.5.19, player 0.5.1 (STORE-RELOAD-BUILTIN).
  "Packages.reload": ["packages.reload"],
  "Packages.uninstall": ["packages.uninstall", "packages.cancel"],
  "Plugin.video.filter": [
    "processing.state",
    "processing.replace:video",
    "processing.operation:video",
  ],
  "Plugin.video.view": ["video.view.set", "video.view.release"],
  "Plugin.video.render": ["video.render.set", "video.render.release"],
  "Plugin.video.shaders": ["video.shaders.set"],
  "Plugin.audio.filter": [
    "processing.state",
    "processing.replace:audio",
    "processing.operation:audio",
  ],
  "Subtitles.state": ["subtitles.state"],
  "Subtitles.refresh": ["subtitles.state"],
  "Subtitles.select": ["subtitles.select", "subtitles.operation"],
  "Subtitles.load": [
    "subtitles.upload.begin",
    "subtitles.upload.append",
    "subtitles.upload.discard",
    "subtitles.commit",
    "subtitles.operation",
  ],
  "Subtitles.remove": ["subtitles.remove", "subtitles.operation"],
  "Subtitles.setStyle": ["subtitles.style"],
  "MediaCapture.capture": [
    "snapshot.capture",
    "snapshot.state",
    "snapshot.release",
    "processing.state?",
    "subtitles.state?",
  ],
  "Snapshot.export": [
    "jobs.start",
    "jobs.state",
    "jobs.cancel",
    "jobs.operation",
  ],
  // An export of remembered files' parts: jobs.start with parts (the
  // player also checks media.sources for it, capabilities/media_control.rs).
  "MediaCapture.export": [
    "jobs.start",
    "jobs.state",
    "jobs.cancel",
    "jobs.operation",
  ],
  "MediaCapture.exportCheck": ["jobs.check"],
  "MediaSources.remember": ["media.sources.remember"],
  "MediaSources.get": ["media.sources.get"],
  "MediaSources.locate": ["media.sources.locate"],
  "MediaSources.forget": ["media.sources.forget"],
  "MediaSources.list": ["media.sources.list"],
  // Replaces what is playing, so it needs playback.control as well as
  // media.sources (capabilities/media_sources.rs); followed as player.open
  // is (source.operation then needs playback.control; the refresh after it
  // is best-effort).
  "MediaSources.open": [
    "media.sources.open",
    "source.operation?",
    "source.state?",
  ],
  "Osd.show": ["osd.show"],
  "Osd.panel": ["osd.panel"],
  "Clipboard.readText": ["clipboard.readText"],
  "Clipboard.writeText": ["clipboard.writeText"],
  "Clipboard.writeImage": ["clipboard.writeImage"],
  "MainWindow.setVideoArea": ["window.setVideoArea"],
  "MainWindow.setAlwaysOnTop": ["window.setAlwaysOnTop"],
  "MainWindow.setCursorAutoHide": ["window.setCursorAutoHide"],
  "MainWindow.setPictureInPicture": ["window.setPictureInPicture"],
  "Bindings.list": ["bindings.list"],
  "Bindings.set": ["bindings.set"],
  "Bindings.reset": ["bindings.reset"],
  "Bindings.onChange": ["event:bindings.changed"],
  "Timeline.layer": ["timeline.set", "timeline.setRanges"],
  "Drops.provide": ["drops.provide", "drops.withdraw"],
  "Panel.onDrop": ["drops.listen"],
  "Settings.page": ["settings.register", "settings.set", "settings.unregister"],
  "Settings.show": ["settings.show"],
  "Appearance.set": ["appearance.set"],
  "Appearance.setLanguage": ["appearance.setLanguage"],
  "Secrets.get": ["secrets.get"],
  "Secrets.set": ["secrets.set"],
  "Secrets.delete": ["secrets.delete"],
  "Rendering.setPreference": ["rendering.setPreference"],
  "Rendering.restartPlayer": ["rendering.restart"],
  "VideoUpscaling.state": ["video.upscaling.state"],
  "VideoUpscaling.set": ["video.upscaling.set"],
  "VideoUpscaling.onChange": ["event:video.upscaling.changed"],
  // SDK 1.7.0: plugin.wire.commands.register (register, then update and
  // unregister through the WireCommand it returns).
  "WireCommands.register": [
    "commands.register",
    "commands.update",
    "commands.unregister",
  ],
};

/**
 * KNOWN_UNREACHED: requests the player guards that no SDK member sends on
 * purpose (`producer.declareInput`: kept for a future keyboard input of
 * generated streams, doc/architecture/README.md §3).
 */
const KNOWN_UNREACHED = new Set(["producer.declareInput"]);

test("CAPABILITY_INFO.apis need the capabilities the player checks per request (host/capabilities.json)", () => {
  const { capability: capabilityOf, methods } = dispatchCapabilities();
  const request = (
    call: string,
  ): { method: string; params: Record<string, string> } => {
    const [name, qualifier] = call.replace(/\?$/, "").split(":") as [
      string,
      string?,
    ];
    return name === "event"
      ? { method: "events.subscribe", params: { event: qualifier! } }
      : { method: name, params: qualifier ? { stage: qualifier } : {} };
  };
  const sdk = readdirSync(
    new URL("../../node_modules/@vestra-player/sdk/dist/", import.meta.url),
    { recursive: true },
  )
    .map(String)
    .filter((file) => file.endsWith(".js") && !file.endsWith("testing.js"))
    .map(sdkFile)
    .join("\n");
  const listed = new Map<string, string[]>();
  for (const [name, info] of Object.entries(CAPABILITY_INFO))
    for (const member of info.apis)
      listed.set(member, [...(listed.get(member) ?? []), name]);
  for (const member of listed.keys())
    assert.ok(
      MEMBER_REQUESTS[member],
      `CAPABILITY_INFO lists ${member}: add the requests it sends to MEMBER_REQUESTS`,
    );
  for (const member of Object.keys(MEMBER_REQUESTS))
    assert.ok(
      listed.has(member),
      `MEMBER_REQUESTS has ${member}, which CAPABILITY_INFO no longer lists`,
    );
  for (const [member, names] of listed) {
    const declared = new Set(
      names.flatMap((name) => [name, ...CAPABILITY_INFO[name]!.requires]),
    );
    const needed = new Set<string>();
    for (const call of MEMBER_REQUESTS[member]!) {
      const { method, params } = request(call);
      assert.ok(
        sdk.includes(`"${method}"`),
        `${member}: the installed SDK never sends ${method}; correct MEMBER_REQUESTS`,
      );
      const capability = capabilityOf(method, params, declared);
      assert.notEqual(
        capability,
        undefined,
        `${member}: the player does not know the request ${method}`,
      );
      if (!capability) continue;
      needed.add(capability);
      if (call.endsWith("?") || declared.has(capability)) continue;
      assert.fail(
        `${member} sends ${call}, for which the player checks ${capability}, but CAPABILITY_INFO lists ${member} under ${names.join(", ")} (with requires: ${[...declared].join(", ")})`,
      );
    }
    for (const name of names)
      assert.ok(
        needed.has(name),
        `CAPABILITY_INFO lists ${member} under ${name}, but the player checks ${name} for none of its requests (${MEMBER_REQUESTS[member]!.join(", ")})`,
      );
  }
  // Every request the player names in the dispatch, if it needs a capability,
  // is sent by some listed member.
  const reached = new Set(
    Object.values(MEMBER_REQUESTS)
      .flat()
      .map((call) => request(call).method),
  );
  reached.add("events.unsubscribe"); // a listener's dispose()
  for (const method of new Set(methods)) {
    const capability = capabilityOf(method, {
      event: "source.state",
      stage: "video",
    });
    if (!capability) continue;
    if (KNOWN_UNREACHED.has(method)) {
      assert.ok(
        !reached.has(method),
        `KNOWN_UNREACHED: ${method} is reached now; remove the entry`,
      );
      continue;
    }
    assert.ok(
      reached.has(method),
      `the player checks ${capability} for ${method}, but no member in CAPABILITY_INFO.apis sends it`,
    );
  }
});

test("OSD_LIMITS, SECRET_LIMITS, TIMELINE_LIMITS and MESSAGE_LIMIT are the player's (capabilities/ui_osd.rs, system.rs, secret_index.rs, timeline.rs, process/mod.rs)", () => {
  const constant = (file: string, name: string) => {
    const value = new RegExp(`const ${name}: \\w+ = ([^;]+);`).exec(
      host(file),
    )?.[1];
    assert.ok(value, `${name} in ${file}`);
    return value
      .split("*")
      .map((part) => Number(part.trim().replaceAll("_", "")))
      .reduce((a, b) => a * b, 1);
  };
  assert.deepEqual(
    {
      messageChars: constant("src/capabilities/ui_osd.rs", "OSD_MESSAGE_CHARS"),
      panelLines: constant("src/capabilities/ui_osd.rs", "OSD_PANEL_LINES"),
      panelBytes: constant("src/capabilities/ui_osd.rs", "OSD_PANEL_BYTES"),
      minColumns: constant("src/capabilities/ui_osd.rs", "OSD_MIN_COLUMNS"),
      maxColumns: constant("src/capabilities/ui_osd.rs", "OSD_MAX_COLUMNS"),
      actions: constant("src/capabilities/ui_osd.rs", "OSD_ACTIONS"),
      actionChars: constant("src/capabilities/ui_osd.rs", "OSD_ACTION_CHARS"),
    },
    { ...OSD_LIMITS },
    "OSD_LIMITS differ from the player's OSD_* constants",
  );
  assert.deepEqual(
    {
      nameChars: constant("src/system.rs", "MAX_SECRET_NAME"),
      valueBytes: constant("src/system.rs", "MAX_SECRET_BYTES"),
      names: constant("src/packages/secret_index.rs", "MAX_NAMES"),
    },
    { ...SECRET_LIMITS },
    "SECRET_LIMITS differ from the player's secret limits",
  );
  assert.deepEqual(
    {
      marks: constant("src/timeline.rs", "MAX_MARKS"),
      ranges: constant("src/timeline.rs", "MAX_RANGES"),
      titleChars: constant("src/timeline.rs", "MAX_TITLE"),
    },
    { ...TIMELINE_LIMITS },
    "TIMELINE_LIMITS differ from the player's timeline.rs limits",
  );
  assert.equal(
    constant("src/process/mod.rs", "MAX_MESSAGE_BYTES"),
    MESSAGE_LIMIT,
    "MESSAGE_LIMIT vs process/mod.rs MAX_MESSAGE_BYTES",
  );
});

test("SystemInfo.rendering has the values app.system reports (system.rs rendering_json)", () => {
  // SystemInfo's field (`Plugin.rendering` is the SDK 1.5.35 service).
  const sdk = /readonly rendering\??: ("[^;]+);/.exec(sdkTypes())?.[1];
  assert.ok(sdk, "SystemInfo.rendering in the SDK's plugin types");
  const system = host("src/system.rs");
  const body =
    /pub fn rendering_json\(gpu: bool\) -> Value \{\n([^\n]*)\n\}/.exec(
      system,
    )?.[1];
  assert.ok(body, "rendering_json in system.rs");
  const values = (text: string) =>
    [...text.matchAll(/"([^"]+)"/g)].map((m) => m[1]!).sort();
  assert.deepEqual(values(sdk), values(body));
  assert.ok(
    system.includes(`value["rendering"] = rendering;`),
    "app.system sets rendering",
  );
});

test("WireRenderingState has the fields rendering.state answers with (host/rendering.rs Rendering::state)", () => {
  const wire = /export interface WireRenderingState \{([\s\S]*?)\n\}/.exec(
    sdkFile("wire.d.ts"),
  )?.[1];
  assert.ok(wire, "WireRenderingState in the SDK's wire.d.ts");
  // Top-level fields and the inline `gpu: { … }` object's fields.
  const sdk = [
    ...new Set([...wire.matchAll(/(\w+)\??:/g)].map((m) => m[1]!)),
  ].sort();
  const state = /pub fn state\(&self[\s\S]*?\n {4}\}/.exec(
    host("src/host/rendering.rs"),
  )?.[0];
  assert.ok(state, "Rendering::state in rendering.rs");
  const player = [
    ...new Set([...state.matchAll(/"(\w+)":/g)].map((m) => m[1]!)),
  ].sort();
  assert.deepEqual(sdk, player);
  const reason = /export interface WireRenderingReason \{([\s\S]*?)\n\}/.exec(
    sdkFile("wire.d.ts"),
  )?.[1];
  assert.deepEqual(
    [...(reason ?? "").matchAll(/(\w+)\??:/g)].map((m) => m[1]!).sort(),
    ["code", "message"],
    "WireRenderingReason is Reason::json's { code, message }",
  );
  assert.ok(
    host("src/playback/render_mode.rs").includes(
      `serde_json::json!({"code":self.code(),"message":self.message()})`,
    ),
    "Reason::json",
  );
});

test("WireUpscalingState has the fields video.upscaling answers with (playback/upscaling.rs Upscaling::state and shared_state; SDK 1.5.37 and 1.5.43, player 0.5.16 and 0.5.25)", () => {
  const wire = /export interface WireUpscalingState \{([\s\S]*?)\n\}/.exec(
    sdkFile("wire.d.ts"),
  )?.[1];
  assert.ok(wire, "WireUpscalingState in the SDK's wire.d.ts");
  // Every field name, the nested sizes' included.
  const sdk = [
    ...new Set([...wire.matchAll(/(\w+)\??:/g)].map((m) => m[1]!)),
  ].sort();
  const upscaling = host("src/playback/upscaling.rs");
  const state = /fn state\(&self[\s\S]*?\n {4}\}/.exec(upscaling)?.[0];
  assert.ok(state, "Upscaling::state in upscaling.rs");
  const shared = /fn shared_state\(&self[\s\S]*?\n {4}\}/.exec(upscaling)?.[0];
  assert.ok(shared, "Upscaling::shared_state in upscaling.rs");
  const player = [
    ...new Set([
      ...[...shared.matchAll(/"(\w+)":/g)].map((m) => m[1]!),
      ...[...state.matchAll(/state\["(\w+)"\]/g)].map((m) => m[1]!),
    ]),
  ].sort();
  assert.deepEqual(sdk, player);
  assert.ok(
    upscaling.includes(`json!({"code":self.code(),"message":self.message()})`),
    "upscaling Reason::json is { code, message } apart from decoding",
  );
  // A decoding reason adds `cause`, and with copy-back `plugins` of { id, name }.
  const reason =
    /export interface WireUpscalingReason[^{]*\{([\s\S]*?)\n\}/.exec(
      sdkFile("wire.d.ts"),
    )?.[1];
  assert.ok(reason, "WireUpscalingReason in the SDK's wire.d.ts");
  assert.deepEqual(
    [...new Set([...reason.matchAll(/(\w+)\??:/g)].map((m) => m[1]!))].sort(),
    ["cause", "id", "name", "plugins"],
  );
  for (const field of [
    `"cause":cause.as_str()`,
    `reason["plugins"]`,
    `json!({"id":owner.id,"name":owner.name})`,
  ])
    assert.ok(upscaling.includes(field), `Reason::json sends ${field}`);
  assert.ok(
    upscaling.includes(`pub const CHANGED: &str = "video.upscaling.changed";`),
    "the event's name",
  );
});

test("UpscalingDecodingCause names the player's causes (playback/upscaling.rs Cause::as_str; SDK 1.5.43, player 0.5.25)", () => {
  const sdk = /export type UpscalingDecodingCause = ([^;]+);/.exec(
    sdkTypes(),
  )?.[1];
  assert.ok(sdk, "UpscalingDecodingCause in the SDK's plugin types");
  const own = /pub fn as_str\(self\) -> &'static str \{([\s\S]*?)\n {4}\}/.exec(
    host("src/playback/upscaling.rs"),
  )?.[1];
  assert.ok(own, "Cause::as_str in upscaling.rs");
  const values = (text: string) =>
    [...text.matchAll(/"([^"]+)"/g)].map((m) => m[1]!).sort();
  assert.deepEqual(values(sdk), values(own));
});

test("UpscalingReasonCode names the player's upscaling reason codes (playback/upscaling.rs, platform; SDK 1.5.37, player 0.5.16)", () => {
  const sdk = /export type UpscalingReasonCode = ([^;]+);/.exec(
    sdkTypes(),
  )?.[1];
  assert.ok(sdk, "UpscalingReasonCode in the SDK's plugin types");
  const own = /pub fn code\(&self\) -> &'static str \{([\s\S]*?)\n {4}\}/.exec(
    host("src/playback/upscaling.rs"),
  )?.[1];
  assert.ok(own, "Reason::code in upscaling.rs");
  // The system's refusals: `code: "…"` and `refuse("…", …)` in the
  // platform modules (and the worker's own failed check).
  const refusals = [
    "src/platform/mod.rs",
    "src/platform/windows.rs",
    "src/playback/upscaling.rs",
  ].flatMap((file) =>
    [...host(file).matchAll(/code: "([a-z-]+)"|refuse\(\s*"([a-z-]+)"/g)].map(
      (m) => (m[1] ?? m[2])!,
    ),
  );
  const values = (text: string) =>
    [...text.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
  assert.deepEqual(
    [...new Set(values(sdk))].sort(),
    [...new Set([...values(own), ...refusals])].sort(),
  );
});

test("RenderingReasonCode names the player's reason codes (playback/render_mode.rs Reason::code; SDK 1.5.35, player 0.5.13)", () => {
  const sdk = /export type RenderingReasonCode = ([^;]+);/.exec(
    sdkTypes(),
  )?.[1];
  assert.ok(sdk, "RenderingReasonCode in the SDK's plugin types");
  const body = /pub fn code\(&self\) -> &'static str \{([\s\S]*?)\n {4}\}/.exec(
    host("src/playback/render_mode.rs"),
  )?.[1];
  assert.ok(body, "Reason::code in render_mode.rs");
  const values = (text: string) =>
    [...text.matchAll(/"([^"]+)"/g)].map((m) => m[1]!).sort();
  assert.deepEqual(values(sdk), values(body));
  assert.ok(
    host("src/system.rs").includes(`value["renderingReason"] = reason;`),
    "app.system sets renderingReason",
  );
});

test("SUBTITLE_LIMITS are the player's (playback/subtitle_state.rs, subtitle_upload.rs)", () => {
  // `pub const NAME: usize = …;` inside the Registry of each file.
  const constant = (file: string, name: string) => {
    const value = new RegExp(`pub const ${name}: usize = ([^;]+);`).exec(
      host(file),
    )?.[1];
    assert.ok(value, `${name} in ${file}`);
    return value
      .split("*")
      .map((part) => Number(part.trim()))
      .reduce((a, b) => a * b, 1);
  };
  const state = "src/playback/subtitle_state.rs";
  const upload = "src/playback/subtitle_upload.rs";
  assert.equal(
    constant(upload, "MAX_UPLOAD_BYTES"),
    constant(state, "MAX_CONTENT_BYTES"),
    "an upload and a committed file have the same limit",
  );
  assert.deepEqual(
    {
      fileBytes: constant(state, "MAX_CONTENT_BYTES"),
      tracksPerPlugin: constant(state, "MAX_OWNER_RESOURCES"),
      tracks: constant(state, "MAX_RESOURCES"),
      totalBytes: constant(state, "MAX_TOTAL_BYTES"),
      loadsPerPlugin: constant(state, "MAX_OWNER_KEYS"),
      loads: constant(state, "MAX_KEYS"),
      uploadsPerPlugin: constant(upload, "MAX_OWNER_ACTIVE"),
      uploads: constant(upload, "MAX_ACTIVE"),
    },
    { ...SUBTITLE_LIMITS },
    "SUBTITLE_LIMITS differ from the player's subtitle registries",
  );
});

test("VIDEO_PROCESSING_LIMITS are the player's (playback/effects.rs, plugin_files.rs, video_shaders.rs; SDK 1.5.39, player 0.5.20)", () => {
  // `pub const NAME: <type> = …;`, products of numbers (`16 * 1024`).
  const constant = (file: string, name: string) => {
    const value = new RegExp(`pub const ${name}: \\w+ = ([^;]+);`).exec(
      host(file),
    )?.[1];
    assert.ok(value, `${name} in ${file}`);
    return value
      .split("*")
      .map((part) => Number(part.trim()))
      .reduce((a, b) => a * b, 1);
  };
  const effects = "src/playback/effects.rs";
  const files = "src/playback/plugin_files.rs";
  const shaders = "src/playback/video_shaders.rs";
  assert.deepEqual(
    {
      shaders: constant(shaders, "MAX_SHADERS"),
      shaderBytes: constant(files, "MAX_SHADER_BYTES"),
      shaderIdBytes: constant(shaders, "MAX_ID"),
      lutBytes: constant(files, "MAX_LUT_BYTES"),
      lutSize: constant(files, "MAX_LUT_SIZE"),
      lutPathBytes: constant(effects, "MAX_LUT_PATH"),
      matrixValue: constant(effects, "MATRIX_VALUE"),
      rdiv: constant(effects, "MAX_RDIV"),
      bias: constant(effects, "MAX_BIAS"),
    },
    { ...VIDEO_PROCESSING_LIMITS },
    "VIDEO_PROCESSING_LIMITS differ from the player's",
  );
});

test("MEDIA_SOURCE_LIMITS are the player's (playback/media_sources.rs, capabilities/media_sources.rs; SDK 1.7.8)", () => {
  // `[pub ]const NAME: <type> = …;` (the request share and the Locate
  // interval are private), products of numbers; `86400.` reads as 86400.
  const constant = (file: string, name: string) => {
    const value = new RegExp(`(?:pub )?const ${name}: [\\w:]+ = ([^;]+);`).exec(
      host(file),
    )?.[1];
    assert.ok(value, `${name} in ${file}`);
    const seconds = /Duration::from_secs\((\d+)\)$/.exec(value)?.[1];
    if (seconds !== undefined) return Number(seconds);
    return value
      .split("*")
      .map((part) => Number(part.trim()))
      .reduce((a, b) => a * b, 1);
  };
  const store = "src/playback/media_sources.rs";
  const capability = "src/capabilities/media_sources.rs";
  assert.deepEqual(
    {
      references: constant(store, "MAX_PER_PLUGIN"),
      parts: constant(store, "MAX_PARTS"),
      partEnd: constant(store, "MAX_PART_END"),
      totalSeconds: constant(store, "MAX_TOTAL_SECONDS"),
      requests: constant(store, "MAX_IN_FLIGHT_PER_PLUGIN"),
      locateInterval: constant(capability, "LOCATE_INTERVAL"),
    },
    { ...MEDIA_SOURCE_LIMITS },
    "MEDIA_SOURCE_LIMITS differ from the player's",
  );
});

test("BINDING_INPUTS are the names the player's binding parser accepts (bindings.rs)", () => {
  const rust = host("src/bindings.rs");
  const list = (name: string) => {
    const body = new RegExp(
      `const ${name}: &\\[&str\\] = &\\[([^\\]]*)\\];`,
    ).exec(rust)?.[1];
    assert.ok(body, `${name} in bindings.rs`);
    return [...body.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
  };
  const parse = braces(rust, "pub fn parse(text: &str)");
  const modifiers = [...parse.matchAll(/"(\w+)" => \d/g)].map((m) => m[1]!);
  const range = (from: string, to: string) =>
    Array.from({ length: to.charCodeAt(0) - from.charCodeAt(0) + 1 }, (_, i) =>
      String.fromCharCode(from.charCodeAt(0) + i),
    );
  const letters = /'(\w)'\.\.='(\w)' => Kind::Letter/.exec(rust);
  const digits = /'(\d)'\.\.='(\d)' => Kind::Digit/.exec(rust);
  const functions = /\.filter\(\|n\| \((\d+)\.\.=(\d+)\)\.contains\(n\)/.exec(
    rust,
  );
  assert.ok(
    letters && digits && functions,
    "letter, digit and function key ranges in bindings.rs kind()",
  );
  assert.deepEqual(
    {
      modifiers,
      letters: range(letters[1]!, letters[2]!),
      digits: range(digits[1]!, digits[2]!),
      functionKeys: Array.from(
        { length: Number(functions[2]) - Number(functions[1]) + 1 },
        (_, i) => `F${Number(functions[1]) + i}`,
      ),
      named: list("NAMED"),
      mouse: list("MOUSE").map((name) => `Mouse:${name}`),
    },
    JSON.parse(JSON.stringify(BINDING_INPUTS)),
    "BINDING_INPUTS differ from bindings.rs",
  );
});

/** A palette of theme.rs (`VESTRA`, `VESTRA_LIGHT`): token → `#rrggbb`. */
function themePalette(name: string): Record<string, string> {
  const body = found(
    new RegExp(
      `const ${name}: \\[\\(&str, u32\\); \\d+\\] = \\[([\\s\\S]*?)\\];`,
    ).exec(host("src/ui/theme.rs"))?.[1],
    `${name} in theme.rs`,
  );
  const palette = Object.fromEntries(
    [...body.matchAll(/\("([^"]+)", 0x([0-9a-f]{6})\)/gi)].map((m) => [
      m[1]!,
      `#${m[2]!.toLowerCase()}`,
    ]),
  );
  found(Object.keys(palette).length > 0, `the colours of ${name}`);
  return palette;
}

test("THEME_PALETTES names the player's themes and no token the player lacks (theme.rs THEMES)", () => {
  const themes = /pub const THEMES: \[&str; \d+\] = \[([^\]]*)\];/.exec(
    host("src/ui/theme.rs"),
  )?.[1];
  assert.ok(themes, "THEMES in theme.rs");
  assert.deepEqual(
    Object.keys(THEME_PALETTES),
    [...themes.matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    "THEME_PALETTES has other palettes than theme.rs THEMES",
  );
  // A token only the SDK names is contract (a plugin's test could rely on
  // a colour the player does not set); a new token of the player's waits
  // for the next SDK release (the colours test below).
  for (const [theme, rust] of [
    ["vestra", "VESTRA"],
    ["vestra-light", "VESTRA_LIGHT"],
  ] as const) {
    const player = themePalette(rust);
    assert.deepEqual(
      Object.keys(THEME_PALETTES[theme]!)
        .filter((token) => !Object.hasOwn(player, token))
        .sort(),
      [],
      `THEME_PALETTES['${theme}'] tokens that theme.rs ${rust} lacks`,
    );
  }
});

advisory("THEME_PALETTES' colours are the player's palettes (theme.rs)", () => {
  const palette = themePalette;
  assert.deepEqual(
    { ...THEME_PALETTES.vestra },
    palette("VESTRA"),
    "THEME_PALETTES.vestra vs theme.rs VESTRA",
  );
  assert.deepEqual(
    { ...THEME_PALETTES["vestra-light"] },
    palette("VESTRA_LIGHT"),
    "THEME_PALETTES['vestra-light'] vs theme.rs VESTRA_LIGHT",
  );
});

test("HostAction ids are the player's core actions (core_actions.rs ACTIONS)", () => {
  const sdk = /export type HostAction = ([^;]+);/.exec(sdkTypes())?.[1];
  assert.ok(sdk, "HostAction in the SDK's plugin types");
  const actions = /pub const ACTIONS: \[&str; \d+\] = \[([^\]]*)\];/.exec(
    host("src/core_actions.rs"),
  )?.[1];
  assert.ok(actions, "ACTIONS in core_actions.rs");
  const ids = (text: string) =>
    [...text.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
  assert.deepEqual(
    ids(sdk),
    ids(actions),
    "HostAction differs from core_actions.rs ACTIONS",
  );
});

/**
 * The `PluginError` codes are the table in the SDK's TSDoc. Codes the player
 * returns that the table does not list yet (reported; each must still occur
 * in the host, so the entry goes once the table lists the code or the host
 * stops returning it).
 */
const KNOWN_UNDOCUMENTED_CODES = new Set<string>([]);

/**
 * Codes the player returns only for a wire parameter the SDK never sends, so
 * no plugin sees them and the `PluginError` table leaves them out (SDK 1.5.33).
 * Each entry names the request and parameter; the test checks that the SDK's
 * wire client still does not send it.
 */
const SDK_UNREACHABLE_CODES = new Map<
  string,
  { readonly method: string; readonly param: string }
>([
  // packages.validate { digest }: the SDK sends only directory and apiVersion.
  ["staleDigest", { method: "packages.validate", param: "digest" }],
]);

test("the error codes the player returns are PluginError codes", () => {
  const types = sdkTypes();
  const table = types.slice(
    types.indexOf("| code | meaning |"),
    types.indexOf("export type PluginError"),
  );
  const documented = new Set(
    [...table.matchAll(/^\s*\* \| (`[^|]+`) \|/gm)].flatMap((m) =>
      [...m[1]!.matchAll(/`([^`]+)`/g)].map((code) => code[1]!),
    ),
  );
  assert.ok(
    documented.has("capability"),
    "the PluginError table in the SDK's plugin types",
  );
  const root = new URL("../../host/src/", import.meta.url);
  const returned = new Set(["invalid"]); // RpcError::invalid
  for (const file of readdirSync(root, { recursive: true }).map(String))
    if (file.endsWith(".rs"))
      for (const m of readFileSync(new URL(file, root), "utf8").matchAll(
        /RpcError::new\(\s*"([^"]+)"/g,
      ))
        returned.add(m[1]!);
  const wire = sdkFile("wire.js");
  for (const [code, { method, param }] of SDK_UNREACHABLE_CODES) {
    assert.ok(
      returned.has(code),
      `SDK_UNREACHABLE_CODES: the player no longer returns "${code}"; remove the entry`,
    );
    assert.ok(
      !documented.has(code),
      `SDK_UNREACHABLE_CODES: PluginError lists "${code}" now; remove the entry`,
    );
    const call = wire.slice(wire.indexOf(`"${method}"`));
    assert.ok(call.length < wire.length, `${method} in the SDK's wire.js`);
    assert.ok(
      !call.slice(0, call.indexOf("})")).includes(param),
      `SDK_UNREACHABLE_CODES: the SDK now sends ${method} { ${param} }, so "${code}" reaches plugins; document it`,
    );
  }
  for (const code of returned)
    assert.ok(
      documented.has(code) ||
        KNOWN_UNDOCUMENTED_CODES.has(code) ||
        SDK_UNREACHABLE_CODES.has(code),
      `the player returns code "${code}", which the PluginError table does not list`,
    );
  for (const code of KNOWN_UNDOCUMENTED_CODES) {
    assert.ok(
      !documented.has(code),
      `KNOWN_UNDOCUMENTED_CODES: PluginError lists "${code}" now; remove the entry`,
    );
    assert.ok(
      returned.has(code),
      `KNOWN_UNDOCUMENTED_CODES: the player no longer returns "${code}"; remove the entry`,
    );
  }
});

type Row = {
  readonly owner: string;
  readonly id: string;
  readonly path: readonly string[];
  readonly title: string;
  readonly bindings: readonly string[];
  readonly focus: "panel" | null;
};
const line = (row: Row) =>
  `${row.owner} | ${row.id} | ${row.path.join(" ▸ ") || `(${row.title})`} | ${row.bindings.join(", ")}${row.focus ? " | panel" : ""}`;
function assertRows(
  actual: readonly Row[],
  expected: readonly Row[],
  what: string,
) {
  const have = new Set(actual.map(line));
  const want = new Set(expected.map(line));
  const missing = [...want].filter((row) => !have.has(row));
  const extra = [...have].filter((row) => !want.has(row));
  assert.ok(
    !missing.length && !extra.length,
    `${what}:\n  in BUILTIN_COMMANDS only:\n    ${missing.join("\n    ")}\n  registered only:\n    ${extra.join("\n    ")}`,
  );
}

/** The player's own commands as host/src registers them (host/actions.rs, core_actions.rs). */
function hostCommands(): Row[] {
  const literals = (text: string) =>
    [...text.matchAll(/"([^"]*)"/g)].map((m) => m[1]!);
  const rows: Row[] = [];
  // host_declarations: `let mut x = Declaration::new("Title", &["Path"])`, `x.bindings = vec![..]`.
  const own = braces(host("src/host/actions.rs"), "fn host_declarations()");
  for (const [, id, variable] of own.matchAll(
    /\("([a-z-]+)", (\w+), state\(/g,
  )) {
    const made = found(
      new RegExp(
        `let mut ${variable} =\\s*commands::Declaration::new\\("([^"]+)", &\\[([^\\]]*)\\]\\)`,
      ).exec(own),
      `host_declarations: the declaration of ${id}`,
    );
    const bindings =
      new RegExp(`${variable}\\.bindings = vec!\\[([^\\]]*)\\]`).exec(
        own,
      )?.[1] ?? "";
    rows.push({
      owner: "host",
      id: id!,
      path: literals(made[2]!),
      title: made[1]!,
      bindings: literals(bindings),
      focus: null,
    });
  }
  // core_actions::declarations: `make(&[path], shown, order, group, &[bindings], repeat)`.
  const rust = host("src/core_actions.rs");
  const body = braces(rust, "pub fn declarations(step: f64)");
  const step = Number(
    /pub const DEFAULT_STEP: f64 = ([\d.]+);/.exec(rust)?.[1],
  );
  const computed: Record<string, string> = {
    backward: `Back ${step} s`,
    forward: `Forward ${step} s`,
  };
  const make = (at: number) => {
    const args = body.slice(at, matching(body, at));
    const arrays = [...args.matchAll(/&\[([^\]]*)\]/g)].map((m) => m[1]!);
    const path = arrays[0]!.split(",").map((part) => {
      part = part.trim();
      return part.startsWith("&")
        ? computed[part.slice(1)]!
        : part.replace(/^"|"$/g, "");
    });
    return { path, bindings: literals(arrays[1]!) };
  };
  // `palette(category, title, shown, &[bindings], repeat)`: out of the
  // menu (menu redesign step 4), its title alone.
  const palette = (at: number) => {
    const args = body.slice(at + 1, matching(body, at));
    const title = /^\s*"[^"]*",\s*(&\w+|"[^"]*")/.exec(args)![1]!;
    const bindings = /&\[([^\]]*)\]/.exec(args)![1]!;
    return {
      path: [] as string[],
      title: title.startsWith("&")
        ? computed[title.slice(1)]!
        : title.replace(/^"|"$/g, ""),
      bindings: literals(bindings),
    };
  };
  for (const m of body.matchAll(
    /\(\s*"([a-z-]+)",\s*(make\(|palette\(|(\w+)\))/g,
  )) {
    const made = m[3]
      ? /let mut \w+ = (make|palette)\(/.exec(
          body.slice(body.indexOf(`let mut ${m[3]} = `)),
        )![1]
      : m[2]!.slice(0, -1);
    const at = m[3]
      ? body.indexOf("(", body.indexOf(`let mut ${m[3]} = `))
      : m.index + m[0].length - 1;
    if (made === "palette") {
      const { path, title, bindings } = palette(at);
      rows.push({
        owner: "host",
        id: m[1]!,
        path,
        title,
        bindings,
        focus: null,
      });
      continue;
    }
    const { path, bindings } = make(at);
    rows.push({
      owner: "host",
      id: m[1]!,
      path,
      title: path.at(-1)!,
      bindings,
      focus: null,
    });
  }
  return rows;
}

// TestHost refuses a plugin command on or under one of the player's own
// menu paths (`conflict`) and counts the player's own commands in
// COMMAND_LIMITS.total (`limit`), taking both from BUILTIN_COMMANDS' rows
// owned by "host" (SDK dist/plugin/testing/commands.js). Contract in the
// direction where TestHost would accept what the player refuses: a menu path
// of the player's that BUILTIN_COMMANDS lacks (TestHost lets a plugin take
// it), and fewer host rows than the player has commands (TestHost reaches
// the total later). The other direction (an SDK-only path or row) only makes
// TestHost stricter; with titles, keys, the built-in plugins' rows and
// BUILTIN_SUBMENUS it is product data (below).
test("BUILTIN_COMMANDS has every menu path of the player's own and as many of its commands (TestHost conflicts and limits)", () => {
  const player = hostCommands();
  found(player.length > 0 && player, "the player's own commands");
  const sdk = BUILTIN_COMMANDS.filter((row) => row.owner === "host");
  const paths = new Set(sdk.map((row) => row.path.join(" ▸ ")));
  assert.deepEqual(
    player
      .filter((row) => row.path.length > 0)
      .map((row) => row.path.join(" ▸ "))
      .filter((path) => !paths.has(path))
      .sort(),
    [],
    "menu paths of the player's own commands (host/actions.rs host_declarations, core_actions.rs) that BUILTIN_COMMANDS lacks",
  );
  assert.ok(
    sdk.length >= player.length,
    `BUILTIN_COMMANDS has ${sdk.length} host rows, the player ${player.length} own commands`,
  );
});

advisory(
  "BUILTIN_COMMANDS and BUILTIN_SUBMENUS are what the player and its built-in plugins register",
  async () => {
    assertRows(
      hostCommands(),
      BUILTIN_COMMANDS.filter((row) => row.owner === "host"),
      "the player's own commands (host/actions.rs host_declarations, core_actions.rs) differ from BUILTIN_COMMANDS",
    );
    const root = new URL("../../plugins/", import.meta.url);
    const rows: Row[] = [];
    for (const name of readdirSync(root).sort()) {
      const directory = new URL(`${name}/`, root);
      const manifest = JSON.parse(
        readFileSync(new URL("manifest.json", directory), "utf8"),
      ) as { id: string };
      const { default: activate } = (await import(
        new URL("main.ts", directory).href
      )) as {
        default: Parameters<typeof startPlugin>[0];
      };
      const data = mkdtempSync(join(tmpdir(), "vestra-agreement-"));
      const { host: fake, stop } = await startPlugin(activate, {
        directory,
        dataDirectory: data,
        locale: "en",
      });
      for (const [id, command] of fake.commands)
        rows.push({
          owner: manifest.id,
          id,
          path: [...command.display.path],
          title: command.display.title,
          bindings: [...(command.declaration.bindings ?? [])],
          focus: command.declaration.focus === "panel" ? "panel" : null,
        });
      await stop();
      rmSync(data, { recursive: true, force: true });
    }
    assertRows(
      rows,
      BUILTIN_COMMANDS.filter((row) => row.owner !== "host"),
      "the built-in plugins' commands (started in the test host) differ from BUILTIN_COMMANDS",
    );
    // Submenus: every inner path of a plugin command below the top level of a
    // built-in menu, or a top-level menu of its own; not the player's own
    // groups (`host_group_label` in commands.rs: Playback ▸ Seek, Playback ▸
    // Speed), whoever fills them (the player's seek and speed actions left them
    // for the command palette, menu redesign step 4).
    const builtIn = new Set(Object.values(MENU_KEYS));
    const hosts = new Set(
      [
        ...braces(host("src/commands.rs"), "pub fn host_group_label(").matchAll(
          /\[((?:"[^"]+",?\s*)+)\] =>/g,
        ),
      ]
        .map((m) => [...m[1]!.matchAll(/"([^"]*)"/g)].map((s) => s[1]!))
        .filter((path) => path.length > 1)
        .map((path) => path.join(" ▸ ")),
    );
    found(hosts.has("Playback ▸ Seek"), "host_group_label in commands.rs");
    const submenus = new Map<string, Set<string>>();
    for (const row of rows)
      for (let i = 1; i < row.path.length; i++) {
        const path = row.path.slice(0, i);
        const key = path.join(" ▸ ");
        if ((i === 1 && builtIn.has(path[0]!)) || hosts.has(key)) continue;
        submenus.set(key, (submenus.get(key) ?? new Set()).add(row.owner));
      }
    const shown = (
      menus: Iterable<{ path: readonly string[]; plugins: Iterable<string> }>,
    ) =>
      [...menus]
        .map(
          (menu) =>
            `${menu.path.join(" ▸ ")} | ${[...menu.plugins].sort().join(", ")}`,
        )
        .sort();
    assert.deepEqual(
      shown(
        [...submenus].map(([key, plugins]) => ({
          path: key.split(" ▸ "),
          plugins,
        })),
      ),
      shown(BUILTIN_SUBMENUS),
      "the built-in plugins' submenus differ from BUILTIN_SUBMENUS",
    );
  },
);

test("COMMAND_LIMITS' declaration limits are the player's (commands.rs Registry::register; SDK 1.5.48)", () => {
  const rust = host("src/commands.rs");
  const text = (label: string) =>
    Number(new RegExp(`text\\([^,]+, "${label}", (\\d+)\\)`).exec(rust)?.[1]);
  const order = /order\.unsigned_abs\(\) > ([\d_]+)/.exec(rust)?.[1];
  assert.deepEqual(
    {
      depth: Number(/const MAX_DEPTH: usize = (\d+);/.exec(rust)?.[1]),
      idBytes: text("localId"),
      titleBytes: text("title"),
      segmentBytes: text("path segment"),
      groupBytes: text("group"),
      categoryBytes: text("category"),
      reasonBytes: text("reason"),
      order: Number(order?.replaceAll("_", "")),
    },
    {
      depth: COMMAND_LIMITS.depth,
      idBytes: COMMAND_LIMITS.idBytes,
      titleBytes: COMMAND_LIMITS.titleBytes,
      segmentBytes: COMMAND_LIMITS.segmentBytes,
      groupBytes: COMMAND_LIMITS.groupBytes,
      categoryBytes: COMMAND_LIMITS.categoryBytes,
      reasonBytes: COMMAND_LIMITS.reasonBytes,
      order: COMMAND_LIMITS.order,
    },
  );
});

test("TestHost refuses a menu text with the player's message (commands.rs text; SDK 1.5.48)", async () => {
  const rust = host("src/commands.rs");
  const rule = /"\{label\} requires 1\.\.=\{limit\} bytes, ([^"]+)"/.exec(
    rust,
  )?.[1];
  assert.ok(rule, "text() message in commands.rs");
  const prefix =
    /Self::Invalid\(message\) => write!\(f, "([^"{]*)\{message\}"\)/.exec(
      rust,
    )?.[1];
  assert.ok(prefix, "CommandError::Invalid's Display in commands.rs");
  await assert.rejects(
    startPlugin(
      (plugin) => {
        plugin.commands.add({ id: "a", path: ["Tools ", "A"], run() {} });
      },
      { capabilities: ["commands"] },
    ),
    {
      code: "invalid",
      message: `${prefix}path segment requires 1..=${COMMAND_LIMITS.segmentBytes} bytes, ${rule}`,
    },
  );
});

test("TestHost refuses a plugin item on or under each of the player's own menu items (SDK 1.5.48)", async () => {
  const paths = hostCommands()
    .map((row) => row.path)
    .filter((path) => path.length > 0);
  // Toggle Fullscreen and Quit (host_declarations) and the core actions.
  assert.ok(paths.some((path) => path.join() === "Quit"));
  for (const taken of [...paths, ...paths.map((path) => [...path, "Item"])])
    await assert.rejects(
      startPlugin(
        (plugin) => {
          plugin.commands.add({ id: "a", path: [...taken], run() {} });
        },
        { capabilities: ["commands"] },
      ),
      { code: "conflict" },
      `a plugin item at ${taken.join(" ▸ ")}`,
    );
});

test("SESSION_STORAGE_LIMITS are the player's (session_storage.rs; SDK 1.5.25, player 0.5.5)", async (t) => {
  const runtime: { SESSION_STORAGE_LIMITS?: object } =
    await import("@vestra-player/sdk/runtime");
  if (!runtime.SESSION_STORAGE_LIMITS)
    return t.skip("the installed SDK is older than 1.5.25");
  const source = host("src/session_storage.rs");
  const constant = (name: string) => {
    const value = new RegExp(`const ${name}: \\w+ = ([^;]+);`).exec(
      source,
    )?.[1];
    assert.ok(value, `${name} in session_storage.rs`);
    return value
      .split("*")
      .map((part) => Number(part.trim().replaceAll("_", "")))
      .reduce((a, b) => a * b, 1);
  };
  assert.deepEqual(
    {
      keyBytes: constant("MAX_KEY_BYTES"),
      bytes: constant("MAX_SESSION_BYTES"),
      entryBytes: constant("ENTRY_BYTES"),
    },
    { ...runtime.SESSION_STORAGE_LIMITS },
    "SESSION_STORAGE_LIMITS differ from the player's session storage limits",
  );
});

test("the SDK's wake events are the player's (capabilities/events.rs WAKE_EVENTS; commands.keys from SDK 1.6.7)", async () => {
  const rust = host("src/capabilities/events.rs");
  const block =
    /pub\(crate\) const WAKE_EVENTS: &\[&str\] = &\[([\s\S]*?)\];/.exec(
      rust,
    )?.[1];
  assert.ok(block, "WAKE_EVENTS in capabilities/events.rs");
  const player = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  // Internal to the SDK: TestHost and `vestra-plugin test` use it to say
  // what keeps a plugin running while idle.
  const runtime = new URL(
    "../../node_modules/@vestra-player/sdk/dist/plugin/runtime.js",
    import.meta.url,
  );
  const { WAKE_EVENTS } = (await import(runtime.href)) as {
    WAKE_EVENTS: readonly string[];
  };
  assert.deepEqual([...WAKE_EVENTS].sort(), [...player].sort());
});
