import assert from "node:assert/strict";
import test from "node:test";
import { parse, parseSync, serialize } from "../model.ts";
import { zh } from "./status.ts";

test("settings parse leniently and keep fields a later version adds", () => {
  assert.deepEqual(parse(null), { syncStepMs: 500, autoReload: false, fallbackEncoding: "gb18030", secondaryPosition: "top", extra: {} });
  assert.deepEqual(parse({ syncStepMs: 100, autoReload: true, fallbackEncoding: "big5" }), {
    syncStepMs: 100, autoReload: true, fallbackEncoding: "big5", secondaryPosition: "top", extra: {},
  });
  assert.equal(parse({ fallbackEncoding: "utf-7" }).fallbackEncoding, "gb18030");
  assert.equal(parse({ syncStepMs: 123 }).syncStepMs, 500, "only the listed steps");
  assert.equal(parse({ autoReload: "yes" }).autoReload, false);
  assert.deepEqual(serialize(parse({ version: 1, secondary: "top", syncStepMs: 50 })), {
    secondary: "top",
    version: 1,
    syncStepMs: 50,
    autoReload: false,
    fallbackEncoding: "gb18030",
    secondaryPosition: "top",
  });
  assert.equal(parse({ secondaryPosition: "aboveMain" }).secondaryPosition, "aboveMain", "A08");
  assert.equal(parse({ secondaryPosition: "middle" }).secondaryPosition, "top");
});

test("typed sync values are whole milliseconds within the host range", () => {
  assert.deepEqual(parseSync("1250"), { value: 1250 });
  assert.deepEqual(parseSync(" +40 "), { value: 40 });
  assert.deepEqual(parseSync("−300"), { value: -300 });
  assert.deepEqual(parseSync("-600000"), { value: -600000 });
  assert.deepEqual(parseSync("90000"), { value: 90000 }, "片头差异：一分半");
  for (const bad of ["", "abc", "1.5", "1e3", "--1"]) assert.ok("error" in parseSync(bad), bad);
  const refused = parseSync("600001");
  assert.ok("error" in refused);
  assert.deepEqual(refused.error, { id: "syncRange", args: [600000, 600000], notes: [] });
  assert.equal(zh(refused.error), "同步值须在 −600000 至 600000 毫秒之间");
  const bad = parseSync("abc");
  assert.ok("error" in bad && zh(bad.error) === "请输入整数毫秒，例如 1250 或 -500。");
});
