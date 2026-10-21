// The SDK package repeats a few of the player's constants; they must agree.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  API_VERSION,
  CAPABILITIES,
  COMMAND_LIMITS,
} from "@vestra-player/sdk/runtime";

const host = (file: string) =>
  readFileSync(new URL(`../../host/${file}`, import.meta.url), "utf8");

test("COMMAND_LIMITS matches the player's registry", () => {
  const rust = host("src/commands.rs");
  const constant = (name: string) =>
    Number(new RegExp(`const ${name}: usize = (\\d+);`).exec(rust)?.[1]);
  assert.equal(constant("MAX_PER_OWNER"), COMMAND_LIMITS.perPlugin);
  assert.equal(constant("MAX_COMMANDS"), COMMAND_LIMITS.total);
});

test("capabilities and apiVersion match the player's manifest rules", () => {
  const rust = host("src/plugins.rs");
  const list = /pub const CAPABILITIES: &\[&str\] = &\[([^\]]*)\];/.exec(rust);
  assert.ok(list, "CAPABILITIES in plugins.rs");
  const names = [...list[1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(names, [...CAPABILITIES]);
  assert.equal(/pub const API: &str = "([^"]+)";/.exec(rust)?.[1], API_VERSION);
});

test("the SDK's Slint library copy matches the player's", () => {
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
