// vendor/sdk is the SDK copy that generated candidates type-check against (it is
// copied into each candidate as types/). It must stay byte-identical to src/sdk:
//   cp ../../sdk/*.ts vendor/sdk/ && cp ../../sdk/plugin/*.ts vendor/sdk/plugin/ && rm vendor/sdk/plugin/testing.ts
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

const source = new URL("../../../sdk/", import.meta.url);
const vendor = new URL("../vendor/sdk/", import.meta.url);
const list = async (root, sub, skip = []) =>
  (await fs.readdir(new URL(sub, root)))
    .filter((name) => name.endsWith(".ts") && !skip.includes(name))
    .map((name) => sub + name)
    .sort();

test("vendor/sdk mirrors the repository SDK (minus the test host)", async () => {
  const expected = [...(await list(source, "")), ...(await list(source, "plugin/", ["testing.ts"]))];
  const actual = [...(await list(vendor, "")), ...(await list(vendor, "plugin/"))];
  assert.deepEqual(actual, expected);
  for (const file of expected)
    assert.equal(
      await fs.readFile(new URL(file, vendor), "utf8"),
      await fs.readFile(new URL(file, source), "utf8"),
      `vendor/sdk/${file} differs from src/sdk/${file}`,
    );
});
