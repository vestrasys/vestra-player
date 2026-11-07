// The command palette (player 0.7.6) names a command "Category: Title": every
// command of the workshop has a category, translated in every language.
import assert from "node:assert/strict";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

const EXPECTED = { en: "Plugins", "zh-Hans": "插件", ja: "プラグイン", ko: "플러그인", de: "Plugins", fr: "Plugins", es: "Complementos" };

test("palette: every command has a category in every language, the player's word for plugins", async () => {
  for (const [locale, category] of Object.entries(EXPECTED)) {
    const host = new TestHost();
    const { stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url), host, locale });
    try {
      assert.deepEqual([...host.commands.keys()], ["workshop.open"]);
      for (const [id, { declaration, display }] of host.commands) {
        assert.ok(typeof display.category === "string" && display.category.trim() !== "", `${locale}: ${id} has no category`);
        assert.equal(display.category, category, `${locale}: ${id}`);
        assert.equal(declaration.home, undefined, `${locale}: ${id} opens its own window`);
      }
    } finally {
      await stop();
    }
  }
});
