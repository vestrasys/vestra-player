// The SDK package (installed from the vestra-player-sdk repository) repeats a
// few of the player's constants and its Slint library; they must agree with
// this host. A mismatch means: change the SDK repository, release it and
// upgrade the dependency here (agentic-workflow.md §1.6).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
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
  MENU_IDS,
  MENU_KEYS,
  MESSAGE_LIMIT,
  OSD_LIMITS,
  parsePo,
  rewriteSlint,
  scanSlint,
  SECRET_LIMITS,
  THEME_PALETTES,
  UI_VALUE_LIMITS,
  LIST_SEPARATORS,
  TEXT_LIMITS,
} from "@vestra-player/sdk/runtime";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { CAPABILITIES as STORE_CAPABILITIES } from "../../plugins/store/labels.ts";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

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

test("the test host's copy of the player's Chinese texts matches locales/zh-Hans.po", () => {
  const { entries } = parsePo(host("locales/zh-Hans.po"));
  const player = new Map(entries.map((e) => [e.id, e.strings[0]]));
  for (const [english, chinese] of Object.entries(HOST_ZH_HANS))
    assert.equal(player.get(english), chinese, english);
});

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

/** The text inside the `{ ... }` that follows `marker` (strings and comments skipped). */
function braces(text: string, marker: string): string {
  const at = text.indexOf(marker);
  assert.ok(at >= 0, `${marker} not found`);
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

test("CAPABILITY_INFO's store wording is the store's (plugins/store/labels.ts)", () => {
  const store: Record<string, { text: string; risky?: boolean }> =
    STORE_CAPABILITIES;
  assert.deepEqual(
    Object.keys(store).sort(),
    Object.keys(CAPABILITY_INFO).sort(),
    "the store labels another set of capabilities than CAPABILITY_INFO",
  );
  for (const [name, info] of Object.entries(CAPABILITY_INFO)) {
    assert.equal(
      store[name]!.text,
      info.label,
      `${name}: store text vs CAPABILITY_INFO.label`,
    );
    assert.equal(
      store[name]!.risky === true,
      info.risky,
      `${name}: store risky vs CAPABILITY_INFO.risky`,
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
  "Timeline.onChange": ["event:timeline.changed"],
  "MainWindow.onChange": ["event:window.state"],
  "Appearance.onChange": ["event:appearance.changed"],
  "PanelSlot.onChange": LAYOUT,
  "Panel.onChange": LAYOUT,
  "BackdropSlot.onChange": LAYOUT,
  "Resolvers.onFileTypesChange": ["event:resolvers.fileTypes"],
  "Commands.add": [
    "commands.register",
    "commands.update",
    "commands.unregister",
  ],
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
  "Timeline.layer": ["timeline.set"],
  "Drops.provide": ["drops.provide", "drops.withdraw"],
  "Panel.onDrop": ["drops.listen"],
  "Settings.page": ["settings.register", "settings.set", "settings.unregister"],
  "Settings.show": ["settings.show"],
  "Appearance.set": ["appearance.set"],
  "Appearance.setLanguage": ["appearance.setLanguage"],
  "Secrets.get": ["secrets.get"],
  "Secrets.set": ["secrets.set"],
  "Secrets.delete": ["secrets.delete"],
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

test("OSD_LIMITS, SECRET_LIMITS and MESSAGE_LIMIT are the player's (capabilities/ui_osd.rs, system.rs, secret_index.rs, process/mod.rs)", () => {
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
  assert.equal(
    constant("src/process/mod.rs", "MAX_MESSAGE_BYTES"),
    MESSAGE_LIMIT,
    "MESSAGE_LIMIT vs process/mod.rs MAX_MESSAGE_BYTES",
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

test("THEME_PALETTES are the player's palettes (theme.rs)", () => {
  const rust = host("src/ui/theme.rs");
  const themes = /pub const THEMES: \[&str; \d+\] = \[([^\]]*)\];/.exec(
    rust,
  )?.[1];
  assert.ok(themes, "THEMES in theme.rs");
  const palette = (name: string) => {
    const body = new RegExp(
      `const ${name}: \\[\\(&str, u32\\); \\d+\\] = \\[([\\s\\S]*?)\\];`,
    ).exec(rust)?.[1];
    assert.ok(body, `${name} in theme.rs`);
    return Object.fromEntries(
      [...body.matchAll(/\("([^"]+)", 0x([0-9a-f]{6})\)/gi)].map((m) => [
        m[1]!,
        `#${m[2]!.toLowerCase()}`,
      ]),
    );
  };
  assert.deepEqual(
    Object.keys(THEME_PALETTES),
    [...themes.matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    "THEME_PALETTES has other palettes than theme.rs THEMES",
  );
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
  const sdk = /export type HostAction = ([^;]+);/.exec(
    sdkFile("plugin/types.d.ts"),
  )?.[1];
  assert.ok(sdk, "HostAction in the SDK's types.d.ts");
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

test("the error codes the player returns are PluginError codes", () => {
  const types = sdkFile("plugin/types.d.ts");
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
    "the PluginError table in types.d.ts",
  );
  const root = new URL("../../host/src/", import.meta.url);
  const returned = new Set(["invalid"]); // RpcError::invalid
  for (const file of readdirSync(root, { recursive: true }).map(String))
    if (file.endsWith(".rs"))
      for (const m of readFileSync(new URL(file, root), "utf8").matchAll(
        /RpcError::new\(\s*"([^"]+)"/g,
      ))
        returned.add(m[1]!);
  for (const code of returned)
    assert.ok(
      documented.has(code) || KNOWN_UNDOCUMENTED_CODES.has(code),
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
  // register_host_commands: `let mut x = Declaration::new("Title", &["Path"])`, `x.bindings = vec![..]`.
  const own = braces(
    host("src/host/actions.rs"),
    "fn register_host_commands(&mut self)",
  );
  for (const [, id, variable] of own.matchAll(
    /\("([a-z-]+)", (\w+), state\(/g,
  )) {
    const made = new RegExp(
      `let mut ${variable} =\\s*commands::Declaration::new\\("([^"]+)", &\\[([^\\]]*)\\]\\)`,
    ).exec(own);
    assert.ok(made, `register_host_commands: the declaration of ${id}`);
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
  for (const m of body.matchAll(/\(\s*"([a-z-]+)",\s*(make\(|(\w+)\))/g)) {
    const at = m[3]
      ? body.indexOf("(", body.indexOf(`let mut ${m[3]} = make`))
      : m.index + m[0].length - 1;
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

test("BUILTIN_COMMANDS and BUILTIN_SUBMENUS are what the player and its built-in plugins register", async () => {
  assertRows(
    hostCommands(),
    BUILTIN_COMMANDS.filter((row) => row.owner === "host"),
    "the player's own commands (host/actions.rs register_host_commands, core_actions.rs) differ from BUILTIN_COMMANDS",
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
  // built-in menu, or a top-level menu of its own; not those the player's
  // own commands open (Playback ▸ Seek, Playback ▸ Speed).
  const builtIn = new Set(Object.values(MENU_KEYS));
  const hosts = new Set(
    hostCommands().flatMap((row) =>
      row.path.slice(0, -1).map((_, i) => row.path.slice(0, i + 1).join(" ▸ ")),
    ),
  );
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
