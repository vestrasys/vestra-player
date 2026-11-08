import type { I18n, Scope, Text } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { type CopyKind, copyTarget } from "./copy.ts";
import type { Keyframes } from "./keyframes.ts";
import type { OpenUrl } from "./open-url.ts";
import type { Reopener } from "./reopen.ts";
import { type SeekSteps, type SettingsStore, stepText } from "./settings.ts";
import type { Anchor, Category, MenuText, Reason, State } from "./texts.ts";
import type { Thumbnails } from "./thumbs.ts";

/** What the commands do with the Playback panel (panel.ts). */
export type PanelActions = {
  /** Playback… (Alt+4): the adjust column at this panel, or closed when the panel is in front. */
  toggle: () => Promise<void>;
  /** The adjust column is shown with this panel in front (Playback… is checked). */
  shown: () => boolean;
  /** Jump to Time… (G): the panel in front, its time field focused. */
  focusJump: () => Promise<void>;
  /** Custom… speed: the panel in front, its speed open for typing. */
  typeRate: () => Promise<void>;
  /** Seek Steps…: the panel in front at its Seek steps section. */
  revealSteps: () => Promise<void>;
};

/** The 播放控制 chapter's features (doc/product/feature-requirements.md). */
export type Features = {
  openUrl: OpenUrl;
  reopener: Reopener;
  keyframes: Keyframes;
  panel: PanelActions;
  /** B29: pass the switch to the host. */
  sleep: () => void;
  settings: SettingsStore;
  copy: (kind: CopyKind) => Promise<void>;
  jumpToStart: () => Promise<void>;
  thumbnails: Thumbnails;
  /** B06 tiers 2–4: one relative seek of `delta` seconds (keyframe or exact, as set at the press). */
  seekBy: (delta: number) => unknown;
};

type Entry = {
  id: string;
  /** In the menu; without one the command is in the command palette only (with its key). */
  path?: (Anchor | MenuText | Text)[];
  /** The title of a command outside the menu. */
  title?: MenuText | Text;
  /** The command palette's "Category: Title" (translated like the title). */
  category: Category;
  /** The command that opens the place where this command's control lives (`panel`: the Playback panel). */
  home?: "panel";
  /** Menu position: doc/product/menus.md, and the 播放控制 chapter's 菜单与快捷键 table for new items. */
  order?: number;
  group?: string;
  radio?: boolean;
  bindings?: string[];
  repeat?: boolean;
  state: () => State;
  action: () => unknown;
};

const UNSUPPORTED = "Not supported by the current source";

/** Busy implies disabled; the reason is shown only when it cannot be done. */
const ready = (enabled = true, checked: boolean | null = null, busy = false, reason: Reason = UNSUPPORTED): State => ({
  enabled: enabled && !busy,
  checked,
  busy,
  reason: enabled ? null : reason,
});

/**
 * Tiers 2–4 of the relative seeks (B06), in the command palette with their keys
 * (menu redesign: the Playback panel's Seek steps replace Playback ▸ Seek). Tier 1
 * (← →) is the core's `seek-backward` / `seek-forward`, given its step with
 * `player.setStepSeek`.
 */
function seekEntries(p: Playback, steps: SeekSteps, { text }: I18n, seekBy: (delta: number) => unknown): Entry[] {
  const tiers: [number, string, string][] = [
    [2, "Ctrl+ArrowLeft", "Ctrl+ArrowRight"],
    [3, "Shift+ArrowLeft", "Shift+ArrowRight"],
    [4, "Ctrl+Alt+ArrowLeft", "Ctrl+Alt+ArrowRight"],
  ];
  return tiers.flatMap(([tier, back, forward]) => {
    const seconds = steps[tier - 1]!;
    const seek = (id: string, direction: "backward" | "forward", delta: number, binding: string): Entry => ({
      id,
      title: direction === "backward" ? text("Back {} s", [stepText(seconds)]) : text("Forward {} s", [stepText(seconds)]),
      category: "Seek",
      home: "panel",
      bindings: [binding],
      action: () => seekBy(delta),
      state: () => ready(p.canStep()),
    });
    return [seek(`backward-${tier}`, "backward", -seconds, back), seek(`forward-${tier}`, "forward", seconds, forward)];
  });
}

function entries(p: Playback, f: Features): Entry[] {
  const rate = (id: string, title: MenuText, value: number): Entry => ({
    id,
    title,
    category: "Speed",
    home: "panel",
    radio: true,
    action: () => p.setRate(value),
    state: () => ready(p.can("rate"), p.state.rate === value, p.ui.rateBusy),
  });
  const keyframe = (id: string, title: MenuText, direction: "backward" | "forward", binding: string): Entry => ({
    id,
    title,
    category: "Seek",
    home: "panel",
    bindings: [binding],
    repeat: true,
    action: () => f.keyframes.run(direction),
    state: () => f.keyframes.state(),
  });
  // Menu sweep (B5): Copy to Clipboard ▸ and the Playback options are in the command palette only.
  const copy = (id: string, title: MenuText, order: number, kind: CopyKind): Entry => ({
    id,
    title,
    category: "Copy to Clipboard",
    order,
    group: "open",
    action: () => f.copy(kind),
    state: () => {
      const target = copyTarget(p.state, kind);
      return "reason" in target ? ready(false, null, false, target.reason) : ready();
    },
  });
  const option = (id: string, title: MenuText, order: number, value: () => boolean, toggle: () => void): Entry => ({
    id,
    title,
    category: "Playback",
    order,
    group: "playback-options",
    action: toggle,
    state: () => ready(true, value()),
  });
  return [
    {
      id: "open",
      path: ["Open File…"],
      category: "File",
      order: 100,
      group: "open",
      bindings: ["Ctrl+O", "F3"],
      action: () => p.open(),
      state: () => ready(!p.ui.selecting, null, p.ui.selecting),
    },
    {
      id: "open-url",
      path: ["@open", "Open URL…"],
      category: "File",
      order: 110,
      group: "open",
      bindings: ["Ctrl+U"],
      action: () => f.openUrl.show(),
      state: () => ready(),
    },
    // A10: the former Reopen (only after a failed load), renamed and widened.
    {
      id: "reopen",
      title: "Reopen Current/Last File",
      category: "File",
      order: 121,
      group: "open",
      bindings: ["Ctrl+Y"],
      action: () => f.reopener.run(),
      state: () => ready(f.reopener.available(), null, f.reopener.busy, "No file to reopen"),
    },
    copy("copy-name", "File Name", 125, "name"),
    copy("copy-folder", "Folder Path", 126, "folder"),
    copy("copy-path", "Full Path", 127, "path"),
    // Menu redesign (step 7): Playback… (Alt+4) opens the Playback panel in the adjust column;
    // again closes the column. The panel replaces Playback ▸ Speed and Seek, Jump to Time…,
    // the custom speed window and Seek Steps…: those commands are in the command palette
    // ("Category: Title") with their ids and keys, and name the panel as their home.
    // Palette only (user ruling 2026-11-08): the menu's Adjustments and Effects (the player's)
    // opens the column at its last tab.
    {
      id: "panel",
      title: "Playback…",
      category: "Playback",
      bindings: ["Alt+4"],
      action: () => f.panel.toggle(),
      state: () => ready(true, f.panel.shown()),
    },
    keyframe("keyframe-back", "Previous Keyframe", "backward", "Ctrl+Shift+ArrowLeft"),
    keyframe("keyframe-forward", "Next Keyframe", "forward", "Ctrl+Shift+ArrowRight"),
    {
      id: "jump",
      title: "Jump to Time…",
      category: "Seek",
      home: "panel",
      bindings: ["G"],
      action: () => f.panel.focusJump(),
      state: () => ready(p.canSeek()),
    },
    {
      id: "jump-start",
      title: "Jump to Start",
      category: "Seek",
      home: "panel",
      bindings: ["Backspace"],
      action: () => f.jumpToStart(),
      state: () => ready(p.canRestart(), null, p.ui.restartBusy),
    },
    {
      id: "time-jump",
      title: "Seek Steps…",
      category: "Seek",
      home: "panel",
      action: () => f.panel.revealSteps(),
      state: () => ready(),
    },
    rate("rate-05", "0.5×", 0.5),
    rate("rate-15", "1.5×", 1.5),
    rate("rate-20", "2×", 2),
    {
      id: "rate-custom",
      title: "Custom…",
      category: "Speed",
      home: "panel",
      action: () => f.panel.typeRate(),
      state: () => ready(p.can("rate"), null, p.ui.rateBusy),
    },
    option("prevent-sleep", "Disable Screen Saver While Playing", 240, () => f.settings.value.preventSleep, () => {
      f.settings.update((s) => {
        s.preventSleep = !s.preventSleep;
      });
      f.sleep();
      p.notify(
        f.settings.value.preventSleep
          ? "Disable screen saver while playing: On"
          : "Disable screen saver while playing: Off",
      );
    }),
    option("hover-time", "Show Time on Seek Bar Hover", 242, () => f.settings.value.hoverTime, () => {
      f.settings.update((s) => {
        s.hoverTime = !s.hoverTime;
      });
      p.notify(f.settings.value.hoverTime ? "Show time on seek bar hover: On" : "Show time on seek bar hover: Off");
    }),
    // I07: greyed out with "Needs ffmpeg" when ffmpeg cannot be found.
    {
      id: "hover-thumbnail",
      title: "Show Thumbnail on Seek Bar Hover",
      category: "Playback",
      order: 243,
      group: "playback-options",
      action: () => {
        f.settings.update((s) => {
          s.hoverThumbnail = !s.hoverThumbnail;
        });
        p.notify(
          f.settings.value.hoverThumbnail
            ? "Show thumbnail on seek bar hover: On"
            : "Show thumbnail on seek bar hover: Off",
        );
      },
      state: () => f.thumbnails.command(),
    },
  ];
}

/** Register `table` in `scope`; the host checks `enabled`, `run` checks it again for late runs. */
function register(scope: Scope, { text }: I18n, table: Entry[]) {
  for (const { id, path, title, category, home, order, group, radio, bindings, repeat, state, action } of table)
    scope.commands.add({
      id,
      path,
      title,
      category: text(category),
      ...(home ? { home } : {}),
      ...(order !== undefined ? { order } : {}),
      ...(group ? { group } : {}),
      ...(radio ? { radio } : {}),
      ...(bindings ? { bindings } : {}),
      ...(repeat ? { repeat } : {}),
      enabled: () => state().enabled,
      checked: () => state().checked,
      busy: () => state().busy,
      reason: () => state().reason,
      run: async () => {
        if (state().enabled) await action();
      },
    });
}

/**
 * Register the commands. The tier 2–4 seeks live in a child scope: a title
 * cannot change in place, so `setSteps` disposes them and adds them again with
 * the new seconds (same ids and bindings).
 */
export function addCommands(scope: Scope, i18n: I18n, playback: Playback, features: Features) {
  register(scope, i18n, entries(playback, features));
  let seeks = scope.scope();
  register(seeks, i18n, seekEntries(playback, features.settings.value.seekSteps, i18n, features.seekBy));
  // One retitle at a time (steps typed one after another): each waits for the previous one,
  // so a scope is never disposed twice nor its ids added while the old ones still hold them.
  let queue = Promise.resolve();
  return {
    setSteps(steps: SeekSteps) {
      queue = queue.catch(() => {}).then(async () => {
        await seeks.dispose();
        seeks = scope.scope();
        register(seeks, i18n, seekEntries(playback, steps, i18n, features.seekBy));
      });
      return queue;
    },
  };
}
