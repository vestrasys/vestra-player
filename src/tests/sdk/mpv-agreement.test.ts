// The mpv pass-through (area `mpv`, SDK 1.8.0): the SDK repeats the
// player's closed refusal rules and limits (`MPV_RULES`, `MPV_LIMITS`), refuses
// early with them and models them in TestHost. They are contract data: they
// must be the player's (src/host/mpv-rules.json, generated from
// src/host/src/playback/mpv_rules.rs; a host test keeps the file current).
// With an SDK older than 1.8.0 the tests skip.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

type Rules = {
  categories: Record<string, string>;
  commands: Record<string, string>;
  properties: Record<string, string>;
  propertyPrefixes: Record<string, string>;
  reads: readonly string[];
  writers: readonly string[];
  filterCommands: Record<string, readonly string[]>;
  filterMessages: readonly string[];
  reservedLabel: string;
  prefixes: readonly string[];
  reservedLabels: readonly string[];
};

// Typed loosely: an SDK before 1.8.0 has neither.
const runtime = (await import("@vestra-player/sdk/runtime")) as unknown as {
  MPV_LIMITS?: Record<string, number>;
  MPV_RULES?: Rules;
};
const OLDER = "the installed SDK is older than 1.8.0";

/** Whether the installed SDK is older than `major.minor.patch`. */
function sdkBefore(major: number, minor: number, patch: number): boolean {
  const version = JSON.parse(
    readFileSync(
      new URL(
        "../../node_modules/@vestra-player/sdk/package.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ).version as string;
  const [a, b, c] = version.split(".").map(Number);
  return a! * 1e6 + b! * 1e3 + c! < major * 1e6 + minor * 1e3 + patch;
}

const player = JSON.parse(host("mpv-rules.json")) as {
  limits: Record<string, number>;
  rules: Record<string, unknown>;
};

test("MPV_LIMITS are the player's (host/mpv-rules.json)", (t) => {
  if (!runtime.MPV_LIMITS) return t.skip(OLDER);
  assert.deepEqual(
    { ...runtime.MPV_LIMITS },
    player.limits,
    "MPV_LIMITS differ from the player's limits",
  );
});

test("MPV_RULES are the player's (host/mpv-rules.json)", (t) => {
  if (!runtime.MPV_RULES) return t.skip(OLDER);
  // SDK 1.11.0 words the clipboard refusal as the player now does.
  if (sdkBefore(1, 11, 0))
    return t.skip(
      "the installed SDK is older than 1.11.0 (the clipboard wording)",
    );
  const sdk = runtime.MPV_RULES;
  // Each table on its own, so a difference names its table. The prefixes
  // are a list in the player's file: their order decides.
  const tables: Record<string, unknown> = {
    categories: { ...sdk.categories },
    commands: { ...sdk.commands },
    properties: { ...sdk.properties },
    propertyPrefixes: Object.entries(sdk.propertyPrefixes),
    reads: [...sdk.reads],
    writers: [...sdk.writers],
    filterCommands: Object.fromEntries(
      Object.entries(sdk.filterCommands).map(([stage, operations]) => [
        stage,
        [...operations],
      ]),
    ),
    filterMessages: [...sdk.filterMessages],
    prefixes: [...(sdk.prefixes ?? [])],
    reservedLabel: sdk.reservedLabel,
    reservedLabels: [...(sdk.reservedLabels ?? [])],
  };
  assert.deepEqual(
    Object.keys(tables).sort(),
    Object.keys(player.rules).sort(),
    "MPV_RULES has other tables than the player's rules",
  );
  for (const [name, table] of Object.entries(tables))
    assert.deepEqual(
      table,
      player.rules[name],
      `MPV_RULES.${name} differs from the player's (src/host/mpv-rules.json)`,
    );
});

test("the mpv group serves the pass-through's five methods, open to every plugin (capabilities/mpv.rs)", () => {
  const rust = host("src/capabilities/mpv.rs");
  const methods = /static METHODS: &\[&str\] = &\[([^\]]*)\]/.exec(rust)?.[1];
  assert.ok(methods, "METHODS in capabilities/mpv.rs");
  assert.deepEqual(
    [...methods.matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    ["command", "get", "set", "observe", "unobserve"].map(
      (name) => `mpv.${name}`,
    ),
  );
  assert.match(rust, /name: "mpv",\s*builtin_only: false,/);
  // No other area lists an mpv.* method.
  const root = new URL("../../host/src/capabilities/", import.meta.url);
  for (const file of readdirSync(root).filter((name) => name.endsWith(".rs"))) {
    if (file === "mpv.rs") continue;
    const list = /static METHODS: &\[&str\] = &\[([^\]]*)\]/.exec(
      readFileSync(new URL(file, root), "utf8"),
    )?.[1];
    assert.doesNotMatch(list ?? "", /"mpv\./, file);
  }
});
