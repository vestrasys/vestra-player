// The mpv pass-through (capability `mpv`, SDK 1.8.0): the SDK repeats the
// player's closed refusal rules and limits (`MPV_RULES`, `MPV_LIMITS`), refuses
// early with them and models them in TestHost. They are contract data: they
// must be the player's (src/host/mpv-rules.json, generated from
// src/host/src/playback/mpv_rules.rs; a host test keeps the file current).
// With an SDK older than 1.8.0 the tests skip.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

test("the mpv capability is one risky capability with the pass-through's five methods (host/capabilities.json)", () => {
  const capabilities = (
    JSON.parse(host("capabilities.json")) as {
      capabilities: {
        name: string;
        risky: boolean;
        builtinOnly: boolean;
        requires: string[];
        residency: string;
        methods: { name: string; needs: unknown }[];
      }[];
    }
  ).capabilities;
  const mpv = capabilities.filter((c) =>
    c.methods.some((m) => m.name.startsWith("mpv.")),
  );
  assert.deepEqual(
    mpv.map((c) => c.name),
    ["mpv"],
  );
  const [group] = mpv;
  assert.deepEqual(
    {
      risky: group!.risky,
      builtinOnly: group!.builtinOnly,
      requires: group!.requires,
      residency: group!.residency,
    },
    { risky: true, builtinOnly: false, requires: [], residency: "live" },
  );
  assert.deepEqual(
    group!.methods,
    ["command", "get", "set", "observe", "unobserve"].map((name) => ({
      name: `mpv.${name}`,
      needs: "mpv",
    })),
  );
  // Added last: the capabilities before it keep their manifest order.
  assert.equal(capabilities.at(-1)!.name, "mpv");
});
