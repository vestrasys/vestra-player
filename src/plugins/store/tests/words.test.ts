import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { CAPABILITIES } from "../labels.ts";
import { HOST, LABELS, PART, WINDOW } from "../text.ts";
import { BUILTIN, FakeNet, OFFICIAL, openStore, props, publish, rows, start, until } from "./helpers.ts";

const pot = readFileSync(new URL("../locales/messages.pot", import.meta.url), "utf8");
const po = readFileSync(new URL("../locales/zh-Hans.po", import.meta.url), "utf8");

/** Only placeholders and punctuation: the player formats it as it is (not in the catalog). */
const formatOnly = (template: string) => !/\p{L}/u.test(template.replace(/\{[^{}]*\}/g, ""));
const quoted = (text: string) => `msgid ${JSON.stringify(text)}`;

test("every template and capability is in messages.pot and translated in zh-Hans.po", () => {
  // The templates of the three tables are distinct.
  assert.equal(new Set([...WINDOW, ...PART, ...HOST]).size, WINDOW.length + PART.length + HOST.length);
  const all = [...WINDOW, ...PART, ...HOST, ...LABELS, ...Object.values(CAPABILITIES).map((c) => c.text)];
  for (const text of all.filter((each) => !formatOnly(each) && !each.includes("\n"))) {
    assert.ok(pot.includes(quoted(text)), `messages.pot: ${text}`);
    assert.ok(po.includes(quoted(text)), `zh-Hans.po: ${text}`);
  }
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
