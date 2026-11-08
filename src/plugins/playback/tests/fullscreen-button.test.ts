import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { time } from "../format.ts";
import activate from "../main.ts";

/**
 * 0.2.4, UI-BAR-05 / UI-BAR-10: the bottom bar's Fullscreen button. A click
 * runs the core action `fullscreen` (the same toggle as the Toggle Fullscreen key, Escape by default); the
 * icon follows `window.state.fullscreen`; the button is the first thing the
 * bar hides when it is too narrow, from the time label's measured width.
 */
const directory = new URL("..", import.meta.url);
const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
const start = (host = new TestHost(), locale: "en" | "zh-Hans" = "en") =>
  startPlugin(activate, { directory, host, locale });

function withMedia(host = new TestHost()) {
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 600, position: 12 });
  return host;
}
async function until(predicate: () => boolean) {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
/** The icon `bar.slint` draws from the bar's data (the test host does not run Slint). */
const icon = (props: Record<string, unknown>) => (props["fullscreen"] ? "exit-fullscreen" : "fullscreen");

test("UI-BAR-05 a click enters fullscreen, a second click leaves it, through host.run like the Toggle Fullscreen key", async () => {
  const { host, stop } = await start(withMedia());
  try {
    const bar = host.mounted("bottom");
    assert.equal(host.mainWindow.fullscreen, false);
    assert.equal(bar.props["fullscreen"], false);
    assert.equal(icon(bar.props), "fullscreen");

    bar.emit("action", "fullscreen");
    await until(() => host.mainWindow.fullscreen && bar.props["fullscreen"] === true);
    assert.equal(icon(bar.props), "exit-fullscreen");

    bar.emit("action", "fullscreen");
    await until(() => !host.mainWindow.fullscreen && bar.props["fullscreen"] === false);
    assert.equal(icon(bar.props), "fullscreen");
    assert.deepEqual(
      host.coreRuns.filter((id) => id === "fullscreen"),
      ["fullscreen", "fullscreen"],
    );
  } finally {
    await stop();
  }
});

test("UI-BAR-05 works without media too (the window is not playback state)", async () => {
  const { host, stop } = await start();
  try {
    const bar = host.mounted("bottom");
    bar.emit("action", "fullscreen");
    await until(() => host.mainWindow.fullscreen && bar.props["fullscreen"] === true);
  } finally {
    await stop();
  }
});

test("UI-BAR-05 the icon follows the window when fullscreen changes elsewhere (key, Escape)", async () => {
  const { host, stop } = await start(withMedia());
  try {
    const bar = host.mounted("bottom");
    host.setMainWindow({ fullscreen: true });
    await until(() => bar.props["fullscreen"] === true);
    assert.equal(icon(bar.props), "exit-fullscreen");
    host.setMainWindow({ fullscreen: false });
    await until(() => bar.props["fullscreen"] === false);
    assert.equal(icon(bar.props), "fullscreen");
  } finally {
    await stop();
  }
});

test("UI-BAR-05 bar.slint draws the button right of Open, left of the time, as a BarButton", () => {
  const open = slint.indexOf('clicked => { PlaybackBridge.action("open"); }');
  const button = slint.indexOf('clicked => { PlaybackBridge.action("fullscreen"); }');
  const time = slint.indexOf(" elapsed := Text");
  assert.ok(open > 0 && open < button && button < time, "Open, Fullscreen, time");
  assert.match(
    slint,
    /BarButton \{\s+visible: row\.fullscreen-fits;\s+\/\/[^\n]*\s+enabled: row\.fullscreen-fits;\s+width: row\.fullscreen-fits \? 41px : 0px;\s+icon: PlaybackBridge\.fullscreen \? "exit-fullscreen" : "fullscreen";\s+tip: PlaybackBridge\.tip-fullscreen;\s+clicked => \{ PlaybackBridge\.action\("fullscreen"\); \}/,
  );
  // A persistent element: no `if` creates or destroys it on resize or state.
  assert.doesNotMatch(slint, /if [^\n]*(fullscreen)[^\n]*:/);
  assert.ok(
    slint.includes(
      'tip-fullscreen: with-key(fullscreen ? @tr("Leave fullscreen") : @tr("Fullscreen"), key-fullscreen);',
    ),
  );
});

test("UI-BAR-05 the tooltip is translated and names the Toggle Fullscreen key", async () => {
  const { host, stop } = await start(withMedia(), "zh-Hans");
  try {
    const bar = host.mounted("bottom");
    host.emit("bindings.changed", null);
    await until(() => bar.props["key-open"] === "Ctrl+O");
    // The test host's binding table has no host/fullscreen command (the
    // player's does, Escape by default): no key here.
    assert.equal(bar.props["key-fullscreen"], "");
    assert.equal(host.translate("Fullscreen"), "全屏");
    assert.equal(host.translate("Leave fullscreen"), "退出全屏");
  } finally {
    await stop();
  }
});

// ------------------------------------------------------------------ UI-BAR-10

/** Replace the `px` units and the named widths of a Slint expression, so JS can evaluate it. */
function toJs(expression: string, names: [string, string][]) {
  let js = expression.replace(/\bmax\(/g, "Math.max(").replace(/(\d+(?:\.\d+)?)px/g, "$1");
  for (const [slint, name] of names) js = js.split(slint).join(name);
  assert.match(js, /^[\w\s.+*>=(),?:-]+$/, js);
  const allowed = new Set(["Math", "max", ...names.map(([, name]) => name)]);
  for (const word of js.match(/[A-Za-z_][\w-]*/g) ?? []) assert.ok(allowed.has(word), `unexpected ${word} in ${js}`);
  return js;
}

/** What the row shows at a bar width, as `bar.slint` works it out. */
interface Row {
  fullscreen: boolean;
  stop: boolean;
  skip: boolean;
  open: boolean;
  /** How many of the bar's buttons show (`row.buttons`). */
  buttons: number;
  /** What the bar reports to the player (`content-min-width`). */
  content: number;
}

/**
 * The fit rules exactly as `bar.slint` states them: `timeWidth(elapsed,
 * widest, total)` from the texts' preferred widths, `row(width, timeWidth,
 * fixed)` for the bar's width and the player's own cells
 * (`BarButtonsHost.fixed`), all px, so the test follows the source.
 */
function rules() {
  const property = (name: string, type: string) => {
    const match = new RegExp(`property <${type}> ${name}: ([^;]+);`).exec(slint);
    assert.ok(match, `bar.slint states ${name}`);
    return match[1]!;
  };
  // The widest elapsed text is "-" + the total (the remaining time), in the body font.
  assert.match(
    slint,
    /widest-elapsed := Text \{\s+visible: false;\s+text: "-" \+ PlaybackBridge\.total-label;\s+font-size: Theme\.font-body;/,
  );
  const timeJs = toJs(property("time-width", "length"), [
    ["widest-elapsed.preferred-width", "widest"],
    ["elapsed.preferred-width", "elapsed"],
    ["total.preferred-width", "total"],
  ]);
  const fitNames: [string, string][] = [
    ["root.width", "width"],
    ["row.time-width", "timeWidth"],
    ["BarButtonsHost.fixed", "fixed"],
  ];
  const fit = (name: string) =>
    new Function("width", "timeWidth", "fixed", `return ${toJs(property(name, "bool"), fitNames)};`) as (
      width: number,
      timeWidth: number,
      fixed: number,
    ) => boolean;
  const fullscreenFits = fit("fullscreen-fits");
  const stopFits = fit("stop-fits");
  const skipFits = fit("skip-fits");
  const openFits = fit("open-fits");
  const flags: [string, string][] = [
    ["row.fullscreen-fits", "fullscreen"],
    ["row.stop-fits", "stop"],
    ["row.skip-fits", "skip"],
    ["row.open-fits", "open"],
  ];
  const buttonsFn = new Function(
    "fullscreen",
    "stop",
    "skip",
    "open",
    `return ${toJs(property("buttons", "int"), flags)};`,
  ) as (fullscreen: boolean, stop: boolean, skip: boolean, open: boolean) => number;
  const leftFn = new Function(
    "buttons",
    "timeWidth",
    `return ${toJs(property("left-width", "length"), [
      ["row.buttons", "buttons"],
      ["row.time-width", "timeWidth"],
    ])};`,
  ) as (buttons: number, timeWidth: number) => number;
  // The bar reports its row's width, never the strip's (no loop through the player).
  assert.equal(property("content-min-width", "length"), "row.left-width");
  return {
    timeWidth: new Function("elapsed", "widest", "total", `return ${timeJs};`) as (
      elapsed: number,
      widest: number,
      total: number,
    ) => number,
    row(width: number, timeWidth: number, fixed = 2): Row {
      const fullscreen = fullscreenFits(width, timeWidth, fixed);
      const stop = stopFits(width, timeWidth, fixed);
      const skip = skipFits(width, timeWidth, fixed);
      const open = openFits(width, timeWidth, fixed);
      const buttons = buttonsFn(fullscreen, stop, skip, open);
      return { fullscreen, stop, skip, open, buttons, content: leftFn(buttons, timeWidth) };
    },
  };
}

/**
 * The player's strip for a bar (bar-buttons.slint, `BarButtonsHost`): the
 * room is the bar's width less `content-min-width` and 4px; the player's
 * own cells always show, plugin buttons take the cells left, `»` one of
 * them when some do not fit. Returns the strip's width.
 */
function strip(width: number, content: number, fixed: number, plugins: number) {
  const room = Math.max(0, width - content - 4);
  const free = Math.max(0, Math.floor(room / 41) - fixed);
  const shown = plugins <= free ? plugins : Math.max(0, Math.min(plugins, free - 1));
  const more = shown < plugins;
  return (shown + (more ? 1 : 0) + fixed) * 41;
}

/** A proportional 11px body font, roughly: digits 6.2px, `:` and space 3px, `-` and `/` 4px. */
const textWidth = (text: string) =>
  [...text].reduce((sum, ch) => sum + (/\d/.test(ch) ? 6.2 : ch === "-" || ch === "/" ? 4 : 3), 0);

test("UI-BAR-10 Fullscreen shows from 6×41 + 10 + time + 8 + fixed×41 + 4 px (about 435px)", () => {
  const { row } = rules();
  // About 85px for `0:00:00 / 0:00:00` in the 11px body font, 🔍 and Sidebar: from 435px.
  assert.equal(row(435, 85).fullscreen, true);
  assert.equal(row(434, 85).fullscreen, false);
  // The threshold moves with the measured time and the player's own cells, it is not a constant.
  assert.equal(row(435, 100).fullscreen, false);
  assert.equal(row(450, 100).fullscreen, true);
  assert.equal(row(415, 65).fullscreen, true);
  assert.equal(row(394, 85, 1).fullscreen, true);
  assert.equal(row(476, 85, 3).fullscreen, true);
  assert.equal(row(475, 85, 3).fullscreen, false);
  for (const fixed of [0, 1, 2, 3]) {
    for (const timeWidth of [40, 65, 85, 100, 140]) {
      let threshold = 0;
      while (!row(threshold, timeWidth, fixed).fullscreen) threshold++;
      assert.equal(threshold, 6 * 41 + 10 + timeWidth + 8 + fixed * 41 + 4, `time ${timeWidth}, fixed ${fixed}`);
      // With it the row needs all six buttons; one px narrower it needs five.
      assert.equal(row(threshold, timeWidth, fixed).content, 6 * 41 + 10 + timeWidth + 8);
      assert.equal(row(threshold - 1, timeWidth, fixed).content, 5 * 41 + 10 + timeWidth + 8);
    }
  }
});

test("UI-BAR-10 narrowing drops Fullscreen, then Stop, then Previous / Next, then Open; Play / Pause stays", () => {
  const { row } = rules();
  const at = (width: number) => {
    const r = row(width, 85);
    return [r.fullscreen, r.stop, r.skip, r.open, r.buttons];
  };
  // An 85px time with 🔍 and Sidebar: each step at its threshold and one px below.
  assert.deepEqual(at(435), [true, true, true, true, 6]);
  assert.deepEqual(at(434), [false, true, true, true, 5]);
  assert.deepEqual(at(394), [false, true, true, true, 5]);
  assert.deepEqual(at(393), [false, false, true, true, 4]);
  assert.deepEqual(at(353), [false, false, true, true, 4]);
  assert.deepEqual(at(352), [false, false, false, true, 2]);
  assert.deepEqual(at(320), [false, false, false, true, 2]);
  assert.deepEqual(at(271), [false, false, false, true, 2]);
  assert.deepEqual(at(270), [false, false, false, false, 1]);
  assert.deepEqual(at(0), [false, false, false, false, 1]);
  for (const fixed of [0, 1, 2, 3]) {
    for (const timeWidth of [40, 65, 85, 100, 140]) {
      const spare = 10 + timeWidth + 8 + fixed * 41 + 4;
      const label = `time ${timeWidth}, fixed ${fixed}`;
      // The thresholds: Stop at five buttons, Previous / Next at four, Open at two.
      for (const [step, count] of [
        ["stop", 5],
        ["skip", 4],
        ["open", 2],
      ] as const) {
        assert.equal(row(count * 41 + spare, timeWidth, fixed)[step], true, `${step}, ${label}`);
        assert.equal(row(count * 41 + spare - 1, timeWidth, fixed)[step], false, `${step}, ${label}`);
      }
      let previous = row(1200, timeWidth, fixed);
      for (let width = 1199; width >= 0; width--) {
        const r = row(width, timeWidth, fixed);
        // In that order: a control never shows while a less important one is gone.
        if (r.skip) assert.ok(r.open, `${label}, width ${width}`);
        if (r.stop) assert.ok(r.skip && r.open, `${label}, width ${width}`);
        if (r.fullscreen) assert.ok(r.stop && r.skip && r.open, `${label}, width ${width}`);
        // The count is the controls that show, Play / Pause always one of them.
        assert.equal(r.buttons, 1 + +r.stop + 2 * +r.skip + +r.open + +r.fullscreen, `${label}, width ${width}`);
        // Narrowing never brings a control back, and `content-min-width` only shrinks.
        assert.ok(r.buttons <= previous.buttons && r.content <= previous.content, `${label}, width ${width}`);
        assert.equal(r.content, r.buttons * 41 + 10 + timeWidth + 8, `${label}, width ${width}`);
        previous = r;
      }
    }
  }
});

test("UI-BAR-10 the player's strip never reaches the time while the bar's controls can make room", () => {
  const { row } = rules();
  for (const fixed of [0, 1, 2, 3]) {
    for (const timeWidth of [40, 65, 85, 100, 140]) {
      // Below this even Play / Pause, the time and the player's own cells do not fit.
      const floor = 41 + 10 + timeWidth + 8 + fixed * 41 + 4;
      for (let width = 1200; width >= floor; width--) {
        const r = row(width, timeWidth, fixed);
        // The player's own cells (no plugin buttons) start right of the row.
        assert.ok(width - strip(width, r.content, fixed, 0) - 4 >= r.content, `time ${timeWidth}, width ${width}`);
        // Plugin buttons get only what the row leaves: they never push the row's controls out.
        for (const plugins of [1, 2, 8]) {
          const room = width - r.content - 4;
          const cells = Math.floor(room / 41);
          if (cells > fixed) assert.ok(strip(width, r.content, fixed, plugins) <= room, `plugins ${plugins}, width ${width}`);
        }
      }
    }
  }
  // An 85px time with 🔍, Sidebar and Fullscreen: a plugin's button fits beside them from 476px.
  assert.equal(Math.floor((476 - row(476, 85).content - 4) / 41), 3);
  assert.equal(strip(476, row(476, 85).content, 2, 1), 3 * 41);
  assert.equal(Math.floor((475 - row(475, 85).content - 4) / 41), 2);
});

test("UI-BAR-10 bar.slint leaves the strip to the player and draws no sidebar toggle of its own", () => {
  assert.ok(slint.includes('import { BarButtonsHost } from "@vestra/bar-buttons.slint";'));
  assert.doesNotMatch(slint, /\bBarButtons\s*\{|import \{[^}]*\bBarButtons\b[^}]*\}/);
  assert.match(slint, /out property <length> content-min-width: row\.left-width;/);
  // `content-min-width` never reads the strip's width (the player sizes the strip from it).
  assert.doesNotMatch(slint.replace(/\/\/.*$/gm, ""), /BarButtonsHost\.(width|room|shown|more)/);
  assert.doesNotMatch(slint, /SidebarToggle|has-sidebar|sidebar-on|sidebar-hidden|key-sidebar|tip-sidebar|toggle-sidebar/);
  // A dropped control is a persistent element with no width, not actionable either.
  for (const [icon, flag] of [
    ["stop", "stop-fits"],
    ["previous", "skip-fits"],
    ["next", "skip-fits"],
    ["open", "open-fits"],
  ]) {
    const button = new RegExp(
      `BarButton \\{\\s+visible: row\\.${flag};\\s+width: row\\.${flag} \\? 41px : 0px;\\s+icon: "${icon}";\\s+tip: [^;]+;\\s+enabled: row\\.${flag} && `,
    );
    assert.match(slint, button, icon);
  }
  assert.doesNotMatch(slint, /if [^\n]*:\s*BarButton\b/);
});

test("UI-BAR-10 the threshold stays put while the time runs and when it shows the remaining time", () => {
  const { timeWidth } = rules();
  for (const duration of [45, 600, 3599, 5400, 36000]) {
    const total = ` / ${time(duration)}`;
    const widest = textWidth(`-${time(duration)}`);
    const seen = new Set<number>();
    // Every elapsed and remaining text of the file, across 9:59 → 10:00 and 59:59 → 1:00:00.
    for (let position = 0; position <= duration; position++) {
      for (const elapsed of [time(position), `-${time(duration - position)}`])
        seen.add(timeWidth(textWidth(elapsed), widest, textWidth(total)));
    }
    assert.deepEqual([...seen], [widest + textWidth(total)], `duration ${duration}`);
  }
  // Without a duration ("--:--") a long elapsed text still gets its room.
  const unknown = textWidth(" / --:--");
  assert.equal(timeWidth(textWidth("1:00:00"), textWidth("---:--"), unknown), textWidth("1:00:00") + unknown);
});

/**
 * 0.2.7: the bar keeps keyboard focus out of its sliders. The library Slider
 * takes Tab focus unless `focusable: false`; with Tab unbound (or the default
 * shortcuts ignored) Tab would land in it and the arrows would change it
 * instead of seeking. The seek line is the bar's own SeekBar, with no
 * FocusScope, so it never takes focus.
 */
test("0.2.7 bar.slint: every library Slider is focusable: false; the SeekBar has no FocusScope", () => {
  const code = slint.replace(/\/\/.*$/gm, "");
  const bodies: string[] = [];
  for (const match of code.matchAll(/\bSlider\s*\{/g)) {
    let depth = 0;
    let end = match.index;
    for (let i = match.index + match[0].length - 1; i < code.length; i++) {
      if (code[i] === "{") depth++;
      else if (code[i] === "}" && --depth === 0) {
        end = i;
        break;
      }
    }
    bodies.push(code.slice(match.index, end));
  }
  assert.ok(bodies.length >= 1, "the volume slider is a library Slider");
  for (const body of bodies) assert.match(body, /^\s*focusable:\s*false;/m, body);
  const seekBar = code.slice(code.indexOf("component SeekBar"), code.indexOf("component MuteButton"));
  assert.ok(seekBar.length > 0);
  assert.doesNotMatch(seekBar, /FocusScope|forward-focus/);
  assert.doesNotMatch(code, /FocusScope/, "the bar has no focus scope of its own");
});

test("0.2.9 the fallback key for Toggle Fullscreen is Escape (player 0.6.0 default)", async () => {
  const { DEFAULT_KEYS } = await import("../tips.ts");
  assert.equal(DEFAULT_KEYS.fullscreen, "Escape");
});
