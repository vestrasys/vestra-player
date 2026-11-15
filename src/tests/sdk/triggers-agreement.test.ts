// Triggers (SDK 1.10.0, player 0.10.0; doc/architecture/events-and-triggers.md
// §4.3, §4.11): the SDK and the player evaluate the `when` language alike.
// Contract data: the agreement table "condition × facts → truth" is one table
// in two copies, the player's src/host/condition-table.json (evaluated by
// src/host/src/conditions_tests.rs) and the SDK's CONDITION_TABLE (evaluated
// by the SDK's tests and here by its `evaluateTruth`); they must be equal.
// The limits are the player's constants (src/host/src/triggers.rs,
// timeline.rs). With an SDK older than 1.10.0 the tests skip.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

// Typed loosely: an SDK before 1.10.0 has none of these.
const runtime = (await import("@vestra-player/sdk/runtime")) as unknown as {
  CONDITION_TABLE?: {
    defaults: Record<string, unknown>;
    rows: {
      when: unknown;
      facts: Record<string, unknown>;
      holds: boolean;
      range?: number;
    }[];
  };
  TRIGGER_LIMITS?: Record<string, number>;
  TIMELINE_LIMITS: Record<string, number>;
  evaluateTruth?: (
    when: unknown,
    facts: Record<string, unknown>,
  ) => { holds: boolean; range: number | null };
};
const OLDER = "the installed SDK is older than 1.10.0";

function constant(file: string, name: string): number {
  const found = new RegExp(`pub const ${name}: usize = (\\d+);`).exec(
    host(file),
  );
  if (!found)
    throw new Error(`not found: ${name} in ${file} (adjust the reader)`);
  return Number(found[1]);
}

test("the agreement table is the player's (condition-table.json; SDK 1.10.0)", (t) => {
  if (!runtime.CONDITION_TABLE) return t.skip(OLDER);
  assert.deepEqual(
    JSON.parse(JSON.stringify(runtime.CONDITION_TABLE)),
    JSON.parse(host("condition-table.json")),
    "CONDITION_TABLE differs from src/host/condition-table.json",
  );
});

test("the SDK's evaluator gives every row of the player's table (SDK 1.10.0)", (t) => {
  if (!runtime.evaluateTruth) return t.skip(OLDER);
  const table = JSON.parse(host("condition-table.json")) as NonNullable<
    typeof runtime.CONDITION_TABLE
  >;
  for (const row of table.rows) {
    const facts = { ...table.defaults, ...row.facts };
    const can = Object.fromEntries(
      (facts.can as string[]).map((c) => [c, true]),
    );
    const truth = runtime.evaluateTruth(row.when, { ...facts, can });
    assert.equal(truth.holds, row.holds, JSON.stringify(row));
    assert.equal(truth.range, row.range ?? null, JSON.stringify(row));
  }
});

test("TRIGGER_LIMITS are the player's (triggers.rs MAX_TRIGGERS, MAX_NAME; SDK 1.10.0)", (t) => {
  if (!runtime.TRIGGER_LIMITS) return t.skip(OLDER);
  assert.deepEqual(
    { ...runtime.TRIGGER_LIMITS },
    {
      triggers: constant("src/triggers.rs", "MAX_TRIGGERS"),
      nameChars: constant("src/triggers.rs", "MAX_NAME"),
    },
  );
  // A position list holds as many ranges as a timeline layer: one constant.
  assert.equal(
    runtime.TIMELINE_LIMITS.ranges,
    constant("src/timeline.rs", "MAX_RANGES"),
  );
  assert.ok(
    host("src/conditions.rs").includes(
      "pub const MAX_RANGES: usize = crate::timeline::MAX_RANGES;",
    ),
    "conditions.rs takes the range limit from timeline.rs",
  );
});
