// Candidates type-check against the plugin API of the player that runs the
// workshop (found from its runner), not against a copy inside the workshop.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { playerSdk } from "../engine.mjs";

test("the player's SDK package is found and the workshop vendors no copy", async () => {
  const sdk = playerSdk();
  const pkg = JSON.parse(await fs.readFile(path.join(sdk, "package.json"), "utf8"));
  assert.equal(pkg.name, "@vestra-player/sdk");
  assert.equal(pkg.exports["."].types, "./dist/index.d.ts");
  await fs.access(path.join(sdk, "dist/index.d.ts"));
  await assert.rejects(fs.access(new URL("../vendor/sdk", import.meta.url)), { code: "ENOENT" });
});
