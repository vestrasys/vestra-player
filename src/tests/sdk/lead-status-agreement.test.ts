// The palette lead's status additions of SDK 1.8.0: an end state's text up
// to 160 characters and the working-only `status.strip`. The SDK repeats the
// player's limits (`PALETTE_LEAD_LIMITS`) and its TestHost refuses what the
// player refuses, in the player's words. Contract data: they must agree with
// src/host/src/capabilities/palette_lead.rs. With an SDK older than 1.8.0
// the tests skip.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";

const rust = readFileSync(
  new URL("../../host/src/capabilities/palette_lead.rs", import.meta.url),
  "utf8",
);

// Typed loosely: an SDK before 1.8.0 has no PALETTE_LEAD_LIMITS.
const runtime = (await import("@vestra-player/sdk/runtime")) as unknown as {
  PALETTE_LEAD_LIMITS?: Record<string, number>;
};
const OLDER = "the installed SDK is older than 1.8.0";

function constant(name: string): number {
  const found = new RegExp(`pub const ${name}: usize = (\\d+);`).exec(rust);
  if (!found)
    throw new Error(
      `not found: ${name} in capabilities/palette_lead.rs (adjust the reader)`,
    );
  return Number(found[1]);
}

/** The player's limits, under the SDK's names. */
const player = {
  title: constant("TITLE_CHARS"),
  placeholder: constant("PLACEHOLDER_CHARS"),
  empty: constant("EMPTY_CHARS"),
  note: constant("NOTE_CHARS"),
  statusText: constant("STATUS_TEXT_CHARS"),
  statusEndText: constant("STATUS_END_TEXT_CHARS"),
  statusNote: constant("STATUS_NOTE_CHARS"),
  statusStrip: constant("STATUS_STRIP_CHARS"),
};

/** The player's messages, as its source words them. */
const FOR_WORKING = "status.strip is for a working status";
const tooLong = (name: string, max: number) =>
  `${name} must be 1–${max} characters on one line`;

test("the player's palette lead limits and messages are where this test reads them", () => {
  assert.deepEqual(player, {
    title: 40,
    placeholder: 60,
    empty: 80,
    note: 40,
    statusText: 80,
    statusEndText: 160,
    statusNote: 40,
    statusStrip: 80,
  });
  assert.ok(rust.includes(`"${FOR_WORKING}"`), FOR_WORKING);
  assert.ok(
    rust.includes('"{key} must be {min}–{max} characters on one line"'),
    "the length message in capabilities/palette_lead.rs",
  );
});

test("PALETTE_LEAD_LIMITS are the player's (palette_lead.rs *_CHARS; SDK 1.8.0)", (t) => {
  if (!runtime.PALETTE_LEAD_LIMITS) return t.skip(OLDER);
  assert.deepEqual(
    { ...runtime.PALETTE_LEAD_LIMITS },
    player,
    "PALETTE_LEAD_LIMITS differ from the player's *_CHARS constants",
  );
});

test("TestHost takes and refuses a status's text and strip as the player does (SDK 1.8.0)", async (t) => {
  if (!runtime.PALETTE_LEAD_LIMITS) return t.skip(OLDER);
  const fake = new TestHost();
  let lead:
    | { ready: Promise<unknown>; update(options: unknown): Promise<unknown> }
    | undefined;
  const { stop } = await startPlugin(
    (plugin) => {
      lead = plugin.palette.lead({ title: "Ask", icon: "ai" });
    },
    {
      capabilities: ["palette.lead"],
      host: fake,
      builtin: true,
      id: "vestra.agreement",
    },
  );
  await lead!.ready;
  const set = (status: object) =>
    lead!.update({ title: "Ask", icon: "ai", status });
  const refused =
    (message: string) => (error: { code?: string; message?: string }) =>
      error.code === "invalid" && error.message === message;
  // A working text is one line (80).
  await set({ text: "x".repeat(player.statusText) });
  await assert.rejects(
    set({ text: "x".repeat(player.statusText + 1) }),
    refused(tooLong("status.text", player.statusText)),
  );
  // An end state's may be twice that; it has no strip text of its own.
  for (const state of ["done", "failed", "note", "stopped"]) {
    await set({ text: "x".repeat(player.statusEndText), state });
    await assert.rejects(
      set({ text: "x".repeat(player.statusEndText + 1), state }),
      refused(tooLong("status.text", player.statusEndText)),
    );
    await assert.rejects(
      set({ text: "t", state, strip: "s" }),
      refused(FOR_WORKING),
    );
  }
  // The strip's own text, for a working status.
  await set({ text: "Vestra is working · Planning…", strip: "Planning…" });
  await set({ text: "t", strip: "s".repeat(player.statusStrip) });
  await assert.rejects(
    set({ text: "t", strip: "s".repeat(player.statusStrip + 1) }),
    refused(tooLong("status.strip", player.statusStrip)),
  );
  await stop();
});

test("PALETTE_LEAD_STATES are the player's, stopped among them (palette_lead.rs StatusState::NAMES; SDK 1.8.0)", async (t) => {
  const states = (
    (await import("@vestra-player/sdk/runtime")) as unknown as {
      PALETTE_LEAD_STATES?: readonly string[];
    }
  ).PALETTE_LEAD_STATES;
  const names = /pub const NAMES: \[&'static str; \d+\] = \[([^\]]*)\];/.exec(
    rust,
  )?.[1];
  if (names === undefined)
    throw new Error(
      "not found: StatusState::NAMES in capabilities/palette_lead.rs (adjust the reader)",
    );
  const player = [...names.matchAll(/"([^"]+)"/g)].map((m) => m[1]!);
  assert.deepEqual(player, ["working", "done", "failed", "note", "stopped"]);
  if (!states) return t.skip(OLDER);
  assert.deepEqual([...states], player);
});
