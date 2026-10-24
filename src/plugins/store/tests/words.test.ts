import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { CAPABILITIES } from "../labels.ts";
import { HOST, PART, WINDOW } from "../text.ts";
import { BUILTIN, FakeNet, OFFICIAL, openStore, props, publish, rows, start, until } from "./helpers.ts";

const words = readFileSync(new URL("../ui/words.slint", import.meta.url), "utf8");

/** `if <var>.id == "…" { return @tr("…", args…); }` lines of one function. */
function entries(name: string, variable: string): Map<string, number> {
  const from = words.indexOf(`function ${name}(`);
  const body = words.slice(from, words.indexOf(`return ${variable}.id;`, from));
  const found = new Map<string, number>();
  const line = new RegExp(`if ${variable}\\.id == ("(?:[^"\\\\]|\\\\.)*") \\{ return @tr\\(("(?:[^"\\\\]|\\\\.)*")(.*)\\); \\}`, "g");
  for (const match of body.matchAll(line)) {
    assert.equal(match[1], match[2], "the id is the @tr text");
    found.set(JSON.parse(match[1]!) as string, match[3]!.split(",").length - 1);
  }
  return found;
}

/** How many arguments a template takes. */
function count(template: string): number {
  const numbered = [...template.matchAll(/\{(\d+)\}/g)].map((m) => Number(m[1]));
  return numbered.length ? Math.max(...numbered) + 1 : [...template.matchAll(/\{\}/g)].length;
}

test("words.slint knows every window template of text.ts, with the right number of arguments", () => {
  const windows = entries("msg", "m");
  const parts = entries("arg", "x");
  assert.deepEqual([...windows.keys()].sort(), [...WINDOW].sort());
  assert.deepEqual([...parts.keys()].sort(), [...PART].sort());
  for (const [template, args] of [...windows, ...parts]) assert.equal(args, count(template), template);
  // The templates of the three tables are distinct.
  assert.equal(new Set([...WINDOW, ...PART, ...HOST]).size, WINDOW.length + PART.length + HOST.length);
});

test("words.slint says every capability in the same words as labels.ts", () => {
  for (const [id, { text }] of Object.entries(CAPABILITIES))
    assert.ok(words.includes(`if id == "${id}" { return @tr(${JSON.stringify(text)}); }`), id);
});

test("English by default: the menu, the list and the OSD", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]);
  const { host, stop } = await start({ net, packages: [BUILTIN], locale: "en" });
  try {
    assert.deepEqual(host.command("open").display.path, ["Plugin Store…"]);
    await openStore(host);
    await until(host, () => rows(host).length === 2);
    assert.deepEqual(
      rows(host).map((r) => [r.name, r.badge, (r as { meta?: string }).meta]),
      [
        ["Alpha", "", "Someone · Official repository"],
        ["vestra.video", "Built-in", "Unknown author · Built-in"],
      ],
    );
    host.window("store").emit("openLink", "javascript:alert(1)");
    await host.settle(20);
    assert.ok(host.osd.includes("This kind of link cannot be opened"));
    assert.equal(props(host)["status-text"], "This kind of link cannot be opened");
  } finally {
    await stop();
  }
});
