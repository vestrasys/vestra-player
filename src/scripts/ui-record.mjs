/**
 * Preloaded into plugin tests (`node --import`) when VESTRA_UI_RECORD names a
 * directory (`scripts/plugin.mjs test` adds it): every property value a
 * plugin sends to a window, panel or settings page on the SDK test host, in
 * its wire form, as JSON lines in that directory. The host test
 * `recorded_plugin_props_match_their_slint` (src/host/src/ui_types.rs) sets
 * each one on the compiled Slint component with the real interpreter: the
 * test host does not know the Slint declarations, so a value whose type does
 * not match (a struct field the struct does not declare, a number for a
 * string) would otherwise pass every plugin test and fail only in the player
 * ("wrong type").
 *
 * One line: {plugin, test, file, component?, global?, property, value}.
 * `plugin` is the plugin directory (the test's working directory), `file`
 * relative to it; `global` is set for panels and settings pages.
 */
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const out = process.env.VESTRA_UI_RECORD;
if (out) {
  const { TestHost } = await import("@vestra-player/sdk/testing");
  mkdirSync(out, { recursive: true });
  const target = join(out, `${process.pid}.jsonl`);
  const seen = new Set();
  /** Per test host: window id → file, settings page id → view, the panel. */
  const views = new WeakMap();
  const record = (view, property, value) => {
    const line = JSON.stringify({
      plugin: process.cwd(),
      test: process.argv[1] ?? "",
      ...view,
      property,
      value,
    });
    const key = JSON.stringify([view, property, value]);
    if (seen.has(key)) return;
    seen.add(key);
    appendFileSync(target, `${line}\n`);
  };
  const request = TestHost.prototype.request;
  TestHost.prototype.request = async function (method, params = {}) {
    const result = await request.call(this, method, params);
    if (!views.has(this))
      views.set(this, { windows: new Map(), pages: new Map(), panel: null });
    const known = views.get(this);
    const p = params ?? {};
    const each = (view, properties) => {
      for (const [name, value] of Object.entries(properties ?? {}))
        record(view, name, value);
    };
    switch (method) {
      case "ui.open": {
        const view = { file: String(p.file) };
        known.windows.set(String(p.id), view);
        each(view, p.properties);
        break;
      }
      case "ui.set": {
        const view = known.windows.get(String(p.id));
        if (view) record(view, String(p.property), p.value);
        break;
      }
      case "ui.mount":
        known.panel = {
          file: String(p.file),
          component: String(p.component),
          global: String(p.global),
        };
        each(known.panel, p.properties);
        break;
      case "ui.setMount":
        if (known.panel) record(known.panel, String(p.property), p.value);
        break;
      case "settings.register": {
        const view = {
          file: String(p.file),
          component: String(p.component),
          global: String(p.global),
        };
        known.pages.set(String(p.id), view);
        each(view, p.properties);
        break;
      }
      case "settings.set": {
        const view = known.pages.get(String(p.id));
        if (view) record(view, String(p.property), p.value);
        break;
      }
    }
    return result;
  };
}
