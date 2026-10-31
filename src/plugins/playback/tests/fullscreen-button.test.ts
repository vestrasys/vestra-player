import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { time } from "../format.ts";
import activate from "../main.ts";

/**
 * 0.2.4, UI-BAR-05 / UI-BAR-10: the bottom bar's Fullscreen button. A click
 * runs the core action `fullscreen` (the same toggle as Enter / F11); the
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

test("UI-BAR-05 a click enters fullscreen, a second click leaves it, through host.run like Enter", async () => {
  const { host, stop } = await start(withMedia());
  try {
    const bar = host.mounted();
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
    const bar = host.mounted();
    bar.emit("action", "fullscreen");
    await until(() => host.mainWindow.fullscreen && bar.props["fullscreen"] === true);
  } finally {
    await stop();
  }
});

test("UI-BAR-05 the icon follows the window when fullscreen changes elsewhere (key, Escape)", async () => {
  const { host, stop } = await start(withMedia());
  try {
    const bar = host.mounted();
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
    const bar = host.mounted();
    host.emit("bindings.changed", null);
    await until(() => bar.props["key-open"] === "Ctrl+O");
    // The test host's binding table has no host/fullscreen command (the
    // player's does, Enter by default): no key here.
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

/**
 * The fit rule exactly as `bar.slint` states it: `timeWidth(elapsed, widest,
 * total)` from the texts' preferred widths, `fits(width, timeWidth)` for the
 * bar's width (all px), so the test follows the source.
 */
function rules() {
  const time = /property <length> time-width: ([^;]+);/.exec(slint);
  const fits = /property <bool> fullscreen-fits: ([^;]+);/.exec(slint);
  const left = /property <length> left-width: ([^;]+);/.exec(slint);
  const room = /strip := BarButtons \{[^}]*?\broom: ([^;]+);/.exec(slint);
  assert.ok(time && fits && left && room, "bar.slint states the rule");
  // The widest elapsed text is "-" + the total (the remaining time), in the body font.
  assert.match(
    slint,
    /widest-elapsed := Text \{\s+visible: false;\s+text: "-" \+ PlaybackBridge\.total-label;\s+font-size: Theme\.font-body;/,
  );
  const timeJs = toJs(time[1], [
    ["widest-elapsed.preferred-width", "widest"],
    ["elapsed.preferred-width", "elapsed"],
    ["total.preferred-width", "total"],
  ]);
  const fitsJs = toJs(fits[1], [
    ["root.width", "width"],
    ["row.time-width", "timeWidth"],
  ]);
  const leftJs = toJs(left[1], [
    ["row.fullscreen-fits", "fits"],
    ["row.time-width", "timeWidth"],
  ]);
  const roomJs = toJs(room[1], [
    ["root.width", "width"],
    ["row.left-width", "leftWidth"],
  ]);
  const fitsFn = new Function("width", "timeWidth", `return ${fitsJs};`) as (width: number, timeWidth: number) => boolean;
  const leftFn = new Function("fits", "timeWidth", `return ${leftJs};`) as (fits: boolean, timeWidth: number) => number;
  const roomFn = new Function("width", "leftWidth", `return ${roomJs};`) as (width: number, leftWidth: number) => number;
  return {
    timeWidth: new Function("elapsed", "widest", "total", `return ${timeJs};`) as (
      elapsed: number,
      widest: number,
      total: number,
    ) => number,
    fits: fitsFn,
    /** What the bar gives the BarButtons strip (`room`). */
    room: (width: number, timeWidth: number) => roomFn(width, leftFn(fitsFn(width, timeWidth), timeWidth)),
  };
}

/** A proportional 11px body font, roughly: digits 6.2px, `:` and space 3px, `-` and `/` 4px. */
const textWidth = (text: string) =>
  [...text].reduce((sum, ch) => sum + (/\d/.test(ch) ? 6.2 : ch === "-" || ch === "/" ? 4 : 3), 0);

test("UI-BAR-10 the fullscreen button hides before the sidebar toggle: below 6×41 + 10 + time + 8 + 41 + 4 px", () => {
  const { fits } = rules();
  // About 85px for `0:00:00 / 0:00:00` in the 11px body font: from 394px.
  assert.equal(fits(394, 85), true);
  assert.equal(fits(393, 85), false);
  // The threshold moves with the measured time, it is not a constant.
  assert.equal(fits(394, 100), false);
  assert.equal(fits(409, 100), true);
  assert.equal(fits(374, 65), true);

  for (const timeWidth of [40, 65, 85, 100, 140]) {
    let threshold = 0;
    while (!fits(threshold, timeWidth)) threshold++;
    // At the last width that hides the button, the five other buttons, the
    // 10px gap, the whole time and the 8px spare still fit left of the
    // toggle's 41px cell and its 4px margin: the time is never cut while the
    // fullscreen button shows, and the button goes before the time does.
    const hiddenAt = threshold - 1;
    assert.ok(5 * 41 + 10 + timeWidth + 8 + 41 + 4 <= hiddenAt, `time ${timeWidth}: hidden at ${hiddenAt}`);
    // With the button shown, the row needs exactly the threshold.
    assert.equal(threshold, 6 * 41 + 10 + timeWidth + 8 + 41 + 4);
  }
});

test("UI-BAR-10 the BarButtons strip narrows first, never reaches the time, and gets nothing below 320px", () => {
  const { fits, room } = rules();
  // What BarButtons shows for a room: whole 41px cells (bar-buttons.slint).
  const cells = (width: number, timeWidth: number) => Math.max(0, Math.floor(room(width, timeWidth) / 41));
  for (const timeWidth of [40, 65, 85, 100, 140]) {
    const fullscreenAt = 6 * 41 + 10 + timeWidth + 8 + 41 + 4;
    let previous = Infinity;
    for (let width = 1200; width >= 0; width--) {
      const shown = cells(width, timeWidth);
      const left = (fits(width, timeWidth) ? 6 : 5) * 41 + 10 + timeWidth + 8;
      // The strip (x = width - cells × 41 - 4) never overlaps the buttons, the time or its 8px spare.
      if (shown > 0) assert.ok(width - shown * 41 - 4 >= left, `time ${timeWidth}, width ${width}`);
      // Narrowing never brings a cell back.
      assert.ok(shown <= previous, `time ${timeWidth}, width ${width}`);
      previous = shown;
      // While Fullscreen shows, the strip has at least the toggle's cell.
      if (fits(width, timeWidth)) assert.ok(shown >= 1, `time ${timeWidth}, width ${width}`);
      if (width < 320) assert.equal(shown, 0, `width ${width}`);
    }
    // The strip loses all its plugin cells before Fullscreen goes: at the
    // threshold only the toggle's cell is left; below it Fullscreen hides and
    // the toggle stays until five buttons, the time and its cell no longer fit.
    assert.equal(cells(fullscreenAt, timeWidth), 1);
    assert.equal(cells(fullscreenAt + 40, timeWidth), 1);
    assert.equal(cells(fullscreenAt + 41, timeWidth), 2);
    const toggleAt = 5 * 41 + 10 + timeWidth + 8 + 41 + 4;
    assert.equal(cells(fullscreenAt - 1, timeWidth), 1);
    if (toggleAt >= 320) {
      assert.equal(cells(toggleAt, timeWidth), 1);
      assert.equal(cells(toggleAt - 1, timeWidth), 0);
    }
  }
  // Three cells (two plugins' buttons and the toggle) next to Fullscreen and an 85px time: from 476px.
  assert.equal(cells(476, 85), 3);
  assert.equal(cells(475, 85), 2);
});

test("UI-BAR-10 bar.slint places the player's BarButtons at the right end and draws no sidebar toggle of its own", () => {
  assert.ok(slint.includes('import { BarButtons } from "@vestra/bar-buttons.slint";'));
  assert.match(slint, /strip := BarButtons \{\s+x: root\.width - self\.width - 4px;\s+y: 0px;\s+room: /);
  // A persistent element: the player sees it placed only if it exists.
  assert.doesNotMatch(slint, /if [^\n]*:\s*BarButtons/);
  assert.doesNotMatch(slint, /SidebarToggle|has-sidebar|sidebar-on|sidebar-hidden|key-sidebar|tip-sidebar|toggle-sidebar/);
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
