// The vendored pi SDK is exactly what tests/vendor-pi.mjs builds from the
// locked package versions, small enough, and its licenses ship with it.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";
import { PROVIDERS, differences } from "./vendor-pi.mjs";

const read = async (file) => JSON.parse(await fs.readFile(new URL(`../${file}`, import.meta.url), "utf8"));

test("vendor/pi and licenses/ match a fresh build (run `npm run vendor:pi` after a pi upgrade)", async () => {
  assert.deepEqual(await differences(), []);
});

test("pi is pinned to 1.4.0 and the bundle stays under 5 MB", async () => {
  const pkg = await read("package.json");
  for (const name of ["@earendil-works/pi-coding-agent", "@earendil-works/pi-ai", "@earendil-works/pi-agent-core"]) assert.equal(pkg.devDependencies[name], "1.4.0");
  assert.equal((await read("vendor/pi/package.json")).version, "1.4.0");
  const { size } = await fs.stat(new URL("../vendor/pi/pi.mjs", import.meta.url));
  assert.ok(size < 5 * 1024 * 1024, `pi.mjs is ${size} bytes`);
  const models = await read("vendor/pi/models.json");
  for (const provider of PROVIDERS) assert.ok(models[provider].length > 0, provider);
});

test("every bundled package has its license; pi's logo has its source", async () => {
  const inventory = await read("dependency-inventory.json");
  for (const item of inventory) await fs.access(new URL(`../licenses/${item.name.replace("/", "__")}/${item.licenseFiles[0]}`, import.meta.url));
  assert.ok(inventory.some((item) => item.name === "@earendil-works/pi-coding-agent"));
  const notice = await fs.readFile(new URL("../licenses/pi-logo/NOTICE.md", import.meta.url), "utf8");
  assert.match(notice, /https:\/\/pi\.dev\/logo-auto\.svg/);
  assert.match(notice, /github\.com\/earendil-works\/pi/);
});
