import type { Plugin } from "@vestra-player/sdk";
import type { AbLoop } from "./loop.ts";
import { COUNTS } from "./settings.ts";

/**
 * Command palette categories of the A-B Loop commands (player 0.7.6): the names
 * of the former Playback ▸ A-B Loop submenus, kept from the marks plugin with
 * their translations.
 */
export const AB_CATEGORY = "A-B Loop";
export const COUNT_CATEGORY = "Loop Count";

/** What an A-B command needs besides a usable source, and the reason shown without it. */
const MISSING = {
  a: "Start point not set",
  b: "End point not set",
  range: "A-B range not set",
} as const;

/** A-B Loop: Loop Count (B23), in `COUNTS` order; the panel's Count list shows the same texts. */
export const COUNT_TITLES = ["Endless", "Once", "2 Times", "3 Times", "5 Times", "10 Times"] as const;

/**
 * A-B Loop (B23), in the command palette with their keys, the Playback panel's
 * A-B Loop section as their place (`home`). Moved here from the marks plugin
 * with the same local ids, titles, keys, orders, groups and categories.
 */
export function addLoopCommands(plugin: Plugin, loop: AbLoop) {
  const usable = () => loop.blocked();
  const hasA = () => loop.a !== null;
  const hasB = () => loop.b !== null;
  const has = { a: hasA, b: hasB, range: () => hasA() && hasB() };
  const items = [
    { id: "ab.set-a", title: "Set Start Point", order: 270, group: "ab-a", binding: "[", needs: null, run: () => loop.setA() },
    { id: "ab.clear-a", title: "Clear Start Point", order: 271, group: "ab-a", binding: "{", needs: "a", run: () => loop.clearA() },
    { id: "ab.a-back", title: "Start Point Back 0.1 s", order: 272, group: "ab-a", binding: "Ctrl+[", needs: "a", repeat: true, run: () => loop.nudgeA(-0.1) },
    { id: "ab.a-forward", title: "Start Point Forward 0.1 s", order: 273, group: "ab-a", binding: "Alt+[", needs: "a", repeat: true, run: () => loop.nudgeA(0.1) },
    { id: "ab.set-b", title: "Set End Point", order: 274, group: "ab-b", binding: "]", needs: null, run: () => loop.setB() },
    { id: "ab.clear-b", title: "Clear End Point", order: 275, group: "ab-b", binding: "}", needs: "b", run: () => loop.clearB() },
    { id: "ab.b-back", title: "End Point Back 0.1 s", order: 276, group: "ab-b", binding: "Ctrl+]", needs: "b", repeat: true, run: () => loop.nudgeB(-0.1) },
    { id: "ab.b-forward", title: "End Point Forward 0.1 s", order: 277, group: "ab-b", binding: "Alt+]", needs: "b", repeat: true, run: () => loop.nudgeB(0.1) },
    { id: "ab.back", title: "Move Range Back 0.1 s", order: 278, group: "ab-both", binding: "Ctrl+Alt+[", needs: "range", repeat: true, run: () => loop.shift(-0.1) },
    { id: "ab.forward", title: "Move Range Forward 0.1 s", order: 279, group: "ab-both", binding: "Ctrl+Alt+]", needs: "range", repeat: true, run: () => loop.shift(0.1) },
    { id: "ab.segment", title: "Loop Current Chapter or Bookmark Range", order: 280, group: "ab-toggle", binding: "Alt+\\", needs: null, run: () => loop.loopSegment() },
  ] as const;
  for (const item of items) {
    const ok = item.needs === null ? () => true : has[item.needs];
    plugin.commands.add({
      id: item.id,
      title: item.title,
      category: AB_CATEGORY,
      home: "panel",
      order: item.order,
      group: item.group,
      bindings: [item.binding],
      ...("repeat" in item ? { repeat: item.repeat } : {}),
      enabled: () => !usable() && ok(),
      reason: () => usable() ?? (ok() || item.needs === null ? null : MISSING[item.needs]),
      run: item.run,
    });
  }
  // `\`: on while any loop runs (another plugin's too), so it can always stop one.
  plugin.commands.add({
    id: "ab.toggle",
    title: "A-B Loop On/Off",
    category: AB_CATEGORY,
    home: "panel",
    order: 281,
    group: "ab-toggle",
    bindings: ["\\"],
    enabled: () => !usable() && loop.toggleable(),
    reason: () => usable() ?? (loop.toggleable() ? null : MISSING.range),
    checked: () => !!plugin.player.state.loop,
    run: () => loop.toggle(),
  });
  COUNTS.forEach((count, i) =>
    plugin.commands.add({
      id: `ab.count.${count ?? "endless"}`,
      title: COUNT_TITLES[i]!,
      category: COUNT_CATEGORY,
      home: "panel",
      order: 282 + i,
      group: "ab-count",
      radio: true,
      checked: () => loop.count === count,
      run: () => loop.setCount(count),
    }),
  );
}
