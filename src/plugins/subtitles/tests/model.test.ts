import assert from "node:assert/strict";
import test from "node:test";
import { parse, parseSync, serialize } from "../model.ts";
import { zh } from "./status.ts";

test("settings parse leniently and keep fields a later version adds", () => {
  assert.deepEqual(parse(null), {
    syncStepMs: 500, autoReload: false, autoLoad: true, fallbackEncoding: "gb18030", secondaryPosition: "top", extra: {},
  });
  assert.deepEqual(parse({ syncStepMs: 100, autoReload: true, fallbackEncoding: "big5" }), {
    syncStepMs: 100, autoReload: true, autoLoad: true, fallbackEncoding: "big5", secondaryPosition: "top", extra: {},
  });
  assert.equal(parse({ fallbackEncoding: "utf-7" }).fallbackEncoding, "gb18030");
  assert.equal(parse({ syncStepMs: 123 }).syncStepMs, 500, "only the listed steps");
  assert.equal(parse({ autoReload: "yes" }).autoReload, false);
  assert.equal(parse({ autoLoad: false }).autoLoad, false, "same-name subtitles: off only when saved off");
  assert.equal(parse({ autoLoad: "no" }).autoLoad, true);
  assert.deepEqual(serialize(parse({ version: 1, secondary: "top", syncStepMs: 50 })), {
    secondary: "top",
    version: 1,
    syncStepMs: 50,
    autoReload: false,
    autoLoad: true,
    fallbackEncoding: "gb18030",
    secondaryPosition: "top",
  });
  assert.equal(parse({ secondaryPosition: "aboveMain" }).secondaryPosition, "aboveMain", "A08");
  assert.equal(parse({ secondaryPosition: "middle" }).secondaryPosition, "top");
});

test("typed sync values are seconds (or ms with the unit) within the host range", () => {
  assert.deepEqual(parseSync("1.25"), { value: 1250 });
  assert.deepEqual(parseSync(" +0.04 "), { value: 40 });
  assert.deepEqual(parseSync("−0.3"), { value: -300 }, "U+2212 minus");
  assert.deepEqual(parseSync("-0,5 s"), { value: -500 }, "a decimal comma and the unit");
  assert.deepEqual(parseSync("2"), { value: 2000 });
  assert.deepEqual(parseSync("250 ms"), { value: 250 });
  assert.deepEqual(parseSync("-0"), { value: 0 });
  assert.deepEqual(parseSync("-600"), { value: -600000 });
  assert.deepEqual(parseSync("90"), { value: 90000 }, "片头差异：一分半");
  for (const bad of ["", "abc", "1.2345", "1e3", "--1", "1.5 ms", ".5", "1 min"]) assert.ok("error" in parseSync(bad), bad);
  const refused = parseSync("600.001");
  assert.ok("error" in refused);
  assert.deepEqual(refused.error, { id: "syncRange", args: [600, 600], notes: [] });
  assert.equal(zh(refused.error), "同步值须在 −600 至 600 秒之间");
  const bad = parseSync("abc");
  assert.ok("error" in bad && zh(bad.error) === "请输入秒数，例如 1.25 或 -0.5。");
});
