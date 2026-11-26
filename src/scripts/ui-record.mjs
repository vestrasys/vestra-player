/**
 * Preloaded into plugin tests (`node --import`) when VESTRA_UI_RECORD or
 * VESTRA_UI_STATES names a directory (`scripts/plugin.mjs test` adds it).
 *
 * VESTRA_UI_RECORD: every property value a plugin sends to a window, panel
 * or settings page on the SDK test host, in its wire form, as JSON lines in
 * that directory. The host test `recorded_plugin_props_match_their_slint`
 * (src/host/src/ui/ui_types.rs) sets each one on the compiled Slint component
 * with the real interpreter: the test host does not know the Slint
 * declarations, so a value whose type does not match (a struct field the
 * struct does not declare, a number for a string) would otherwise pass every
 * plugin test and fail only in the player ("wrong type").
 *
 * One line: {plugin, test, file, component?, global?, property, value}.
 * `plugin` is the plugin directory (the test's working directory), `file`
 * relative to it; `global` is set for panels and settings pages.
 *
 * VESTRA_UI_STATES (the screenshot tool, scripts/ui-shots.mjs): every view
 * event in order, without removing repeats, so the tool can rebuild the
 * state each view had. One line:
 * {plugin, test, case, host, view, kind, event, file, component?, global?,
 * title?, category?, property?, value?}: `host` numbers the test hosts of the
 * process, `view` the views of one host (a reopened window is a new view),
 * `kind` is window, panel or page, `event` is open, set or close, and `case`
 * the name of the test that was running (node:test).
 */
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const out = process.env.VESTRA_UI_RECORD;
const states = process.env.VESTRA_UI_STATES;
if (out || states) {
  const { TestHost } = await import("@vestra-player/sdk/testing");
  const testFile = process.argv[1] ?? "";
  const target = out && join(out, `${process.pid}.jsonl`);
  const stateTarget = states && join(states, `${process.pid}.jsonl`);
  for (const dir of [out, states]) if (dir) mkdirSync(dir, { recursive: true });
  const seen = new Set();
  /** Per test host: window id → file, settings page id → view, slot → panel. */
  const views = new WeakMap();
  const record = (view, property, value) => {
    if (!target) return;
    const line = JSON.stringify({
      plugin: process.cwd(),
      test: testFile,
      ...view,
      property,
      value,
    });
    const key = JSON.stringify([view, property, value]);
    if (seen.has(key)) return;
    seen.add(key);
    appendFileSync(target, `${line}\n`);
  };

  // The test running when a request came (plugin requests arrive from the
  // plugin's own process, so the call stack does not name it): a root
  // `beforeEach`, only in a test file's process (NODE_OPTIONS also reaches
  // the test runner's own process).
  let current = "";
  if (stateTarget && /\.test\.[cm]?[jt]s$/.test(testFile)) {
    const { beforeEach } = await import("node:test");
    beforeEach((t) => {
      current = t.fullName ?? t.name;
    });
  }
  const caseName = () => current;
  let hosts = 0;
  const event = (known, view, kind, name, extra) => {
    if (!stateTarget) return;
    appendFileSync(
      stateTarget,
      `${JSON.stringify({
        plugin: process.cwd(),
        test: testFile,
        case: caseName(),
        host: known.host,
        view: view.serial,
        kind,
        event: name,
        file: view.file,
        component: view.component,
        global: view.global,
        ...extra,
      })}\n`,
    );
  };
  const stateful = (known, view, kind, properties, extra = {}) => {
    view.serial = ++known.serials;
    event(known, view, kind, "open", extra);
    for (const [property, value] of Object.entries(properties ?? {}))
      event(known, view, kind, "set", { property, value });
  };
  // `serial` is not part of the recorded view (VESTRA_UI_RECORD's lines).
  const plain = ({ serial: _serial, ...view }) => view;

  const request = TestHost.prototype.request;
  TestHost.prototype.request = async function (method, params = {}) {
    const result = await request.call(this, method, params);
    if (!views.has(this))
      views.set(this, {
        windows: new Map(),
        pages: new Map(),
        panels: new Map(),
        host: ++hosts,
        serials: 0,
      });
    const known = views.get(this);
    const p = params ?? {};
    // A row patch (SDK 1.5.44, player 0.6.0) carries operations, not the
    // value: record the whole value the property shows after it.
    const shownValue = (props) =>
      p.patch ? props?.[String(p.property)] : p.value;
    // A plugin holds one panel per slot (SDK 1.6.15); a request names its
    // slot, or (one panel, older SDKs) means the only one.
    const panelOf = () => {
      if (p.slot != null) return known.panels.get(String(p.slot)) ?? null;
      return known.panels.size === 1 ? [...known.panels.values()][0] : null;
    };
    const panelProps = () =>
      (p.slot != null ? this.panels?.get(String(p.slot)) : this.panel)?.props;
    const each = (view, properties) => {
      for (const [name, value] of Object.entries(properties ?? {}))
        record(plain(view), name, value);
    };
    switch (method) {
      case "ui.open": {
        const view = { file: String(p.file) };
        known.windows.set(String(p.id), view);
        each(view, p.properties);
        stateful(known, view, "window", p.properties);
        break;
      }
      case "ui.set": {
        const view = known.windows.get(String(p.id));
        if (view) {
          const value = shownValue(this.windows.get(String(p.id))?.props);
          record(plain(view), String(p.property), value);
          event(known, view, "window", "set", {
            property: String(p.property),
            value,
          });
        }
        break;
      }
      case "ui.close": {
        const view = known.windows.get(String(p.id));
        if (view) event(known, view, "window", "close");
        known.windows.delete(String(p.id));
        break;
      }
      case "ui.mount": {
        const view = {
          file: String(p.file),
          component: String(p.component),
          global: String(p.global),
        };
        known.panels.set(String(p.slot ?? "bottom"), view);
        each(view, p.properties);
        stateful(known, view, "panel", p.properties);
        break;
      }
      case "ui.setMount": {
        const view = panelOf();
        if (view) {
          const value = shownValue(panelProps());
          record(plain(view), String(p.property), value);
          event(known, view, "panel", "set", {
            property: String(p.property),
            value,
          });
        }
        break;
      }
      case "ui.unmount": {
        const gone =
          p.slot != null ? [String(p.slot)] : [...known.panels.keys()];
        for (const slot of gone) {
          const view = known.panels.get(slot);
          if (view) event(known, view, "panel", "close");
          known.panels.delete(slot);
        }
        break;
      }
      case "settings.register": {
        const view = {
          file: String(p.file),
          component: String(p.component),
          global: String(p.global),
        };
        known.pages.set(String(p.id), view);
        each(view, p.properties);
        stateful(known, view, "page", p.properties, {
          title: p.title,
          category: p.category,
        });
        break;
      }
      case "settings.set": {
        const view = known.pages.get(String(p.id));
        if (view) {
          const value = shownValue(
            this.settings.pages.get(String(p.id))?.props,
          );
          record(plain(view), String(p.property), value);
          event(known, view, "page", "set", {
            property: String(p.property),
            value,
          });
        }
        break;
      }
      case "settings.unregister": {
        const view = known.pages.get(String(p.id));
        if (view) event(known, view, "page", "close");
        known.pages.delete(String(p.id));
        break;
      }
    }
    return result;
  };
}
