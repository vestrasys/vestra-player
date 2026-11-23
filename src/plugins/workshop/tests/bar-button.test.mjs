// The bottom bar's AI button (player 0.5.10): left of the Plugin Store's
// (order 200), and a click opens the window exactly like the menu item.
import assert from "node:assert/strict";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { until } from "./fixture.mjs";

test("the bottom bar's AI button: icon, order, tooltip, and a click opens the window like the menu", async (t) => {
  const host = new TestHost();
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host, builtin: true });
  t.after(() => started.stop());
  assert.equal(host.barButtons.length, 1);
  const [button] = host.barButtons;
  assert.equal(button.command, "workshop.open");
  assert.equal(button.icon, "ai");
  assert.equal(button.image, false);
  assert.equal(button.order, 100);
  assert.equal(button.enabled, true);
  assert.equal(button.checked, false);
  assert.equal(button.tooltip, "Vestra Player AI");
  // The button sits beside a menu entry: one with no path would be refused.
  assert.deepEqual(host.command("workshop.open").declaration.path, ["@tools", "Vestra Player AI…"]);
  assert.deepEqual(host.command("workshop.open").declaration.button, { icon: "ai", order: 100 });

  assert.ok(!host.windows.has("ai") || !host.window("ai").visible, "the window starts closed");
  await button.click();
  await until(host, () => host.windows.has("ai") && host.window("ai").visible, "window shown");
  assert.equal(host.window("ai").file, "ai.slint");
  await until(host, () => host.window("ai").props.banner?.code === "not-configured", "banner");
});
