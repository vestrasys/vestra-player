// node --test packaging/tests/ (npm run test:packaging)
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import {
  COMMITTED_ICO,
  COMMITTED_STRIP_ICON,
  decodePng,
  encodePng,
  icns,
  ico,
  iconContents,
  iconDifference,
  macosMaster,
  pngDifference,
  resize,
  stripIcon,
} from "../icon.mjs";
import { source } from "../lib.mjs";

const logo = decodePng(await readFile(join(source, "assets/brand/logo.png")));

test("the committed vestra.ico matches the logo", async () => {
  assert.equal(iconDifference(await readFile(COMMITTED_ICO), ico(logo)), null);
});

test("the committed title strip icon matches the logo", async () => {
  const committed = await readFile(COMMITTED_STRIP_ICON);
  assert.equal(pngDifference(committed, stripIcon(logo, { level: 1 })), null);
  assert.match(
    pngDifference(committed, encodePng(resize(logo, 32, 32))),
    /width 64 vs 32/,
  );
});

test("other PNG compression is the same icon", () => {
  const [fast, best] = [ico(logo, { level: 1 }), ico(logo, { level: 9 })];
  assert.notDeepEqual(fast, best);
  assert.equal(iconDifference(fast, best), null);
  const small = resize(macosMaster(logo), 64, 64);
  const master = { ...small, width: 64, height: 64 };
  const [a, b] = [icns(master, { level: 1 }), icns(master, { level: 9 })];
  assert.notDeepEqual(a, b);
  assert.equal(iconDifference(a, b), null);
});

test("a changed pixel is a different icon", () => {
  const changed = { ...logo, data: Float32Array.from(logo.data) };
  // An opaque pixel in the middle of the V's left arm.
  const at = (500 * logo.width + 300) * 4;
  changed.data[at] = changed.data[at] > 0.5 ? 0 : changed.data[at + 3];
  assert.match(iconDifference(ico(logo), ico(changed)), /pixels differ/);
});

test("a changed directory field is a different icon", () => {
  const bytes = Buffer.from(ico(logo));
  bytes.writeUInt16LE(24, 6 + 6); // first entry: 24 bits per pixel
  assert.match(iconDifference(ico(logo), bytes), /bits 32 vs 24/);
});

test("PNG entries are decoded, DIB entries compared as stored", () => {
  const entries = iconContents(ico(logo));
  assert.equal(entries.length, 9);
  assert.deepEqual(
    entries.map((entry) => entry.format),
    [...Array(8).fill("raw"), "png"],
  );
  const png = entries.at(-1);
  assert.equal(png.width, 256);
  assert.equal(png.pixels.length, 256 * 256 * 4);
  assert.ok(encodePng(resize(logo, 4, 4)).length > 0);
});
