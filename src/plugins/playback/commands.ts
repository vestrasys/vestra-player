import type { Command, Condition, I18n, Scope, Text, When } from "@vestra-player/sdk";
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

/** The 播放控制 chapter's features. */
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
  /** Menu position: the item's fixed order in its menu. */
  order?: number;
  group?: string;
  radio?: boolean;
  bindings?: string[];
  repeat?: boolean;
  /** For `run`, and the state of a command without `gate`. */
  state: () => State;
  /**
   * The command's state as conditions the player evaluates (`plugin.when`), given again
   * when what they are built from changes (`regate`); what has no condition comes from
   * the plugin's own facts. `busy` is the plugin's own state.
   */
  gate?: () => Gate;
  busy?: () => boolean;
  action: () => unknown;
};

/** A gated command's state: one of the shared conditions (`Conditions`), so a change is a new object. */
export type Gate = { enabled: Condition; reason: Reason | null; checked?: Condition };

/** The conditions the commands use, made once (a gate picks one of them). */
export function conditions(when: When) {
  const seekable = when.can("seek").and(when.phase("ready", "ended"));
  const rates = new Map<number, Condition>();
  return {
    always: when.ready.or(when.ready.not()),
    never: when.ready.and(when.ready.not()),
    /** `Playback.canStep` (tiers 2–4); a missing position is checked again at the run. */
    seekable,
    /** `Playback.canSeek`: and a known length above 0 (`duration > 0`; the bounds are `min ≤ x`). */
    jumpable: seekable.and(when.duration({ min: 0.001 })),
    /** `Playback.can("rate")`. */
    rateable: when.can("rate").and(when.phase("ready")),
    /** `Playback.canRestart`. */
    restartable: when.phase("ready").and(when.can("seek")).or(when.phase("ended").and(when.can("restart"))),
    /** Keyframe steps: a seekable file, ready or ended. */
    keyframeable: seekable.and(when.stream.not()),
    /** The rate is `value` (the bounds are `min ≤ rate < max`: a range 0.001 wide holds exactly the rates the player sets, which have at most two decimals). */
    rate(value: number) {
      let made = rates.get(value);
      if (!made) rates.set(value, (made = when.rate({ min: value, max: value + 0.001 })));
      return made;
    },
    looping: when.looping,
  };
}
export type Conditions = ReturnType<typeof conditions>;

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
function seekEntries(p: Playback, steps: SeekSteps, { text }: I18n, seekBy: (delta: number) => unknown, c: Conditions): Entry[] {
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
      // `canStep`: a seekable source with a position; without one the key shows the reason.
      gate: () => ({ enabled: p.state.position === null ? c.never : c.seekable, reason: UNSUPPORTED }),
    });
    return [seek(`backward-${tier}`, "backward", -seconds, back), seek(`forward-${tier}`, "forward", seconds, forward)];
  });
}

function entries(p: Playback, f: Features, c: Conditions): Entry[] {
  const rate = (id: string, title: MenuText, value: number): Entry => ({
    id,
    title,
    category: "Speed",
    home: "panel",
    radio: true,
    action: () => p.setRate(value),
    state: () => ready(p.can("rate"), p.state.rate === value, p.ui.rateBusy),
    gate: () => ({ enabled: c.rateable, reason: UNSUPPORTED, checked: c.rate(value) }),
    busy: () => p.ui.rateBusy,
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
    gate: () => f.keyframes.gate(c),
    busy: () => f.keyframes.busy,
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
    // What can be copied depends on the source's path: decided here, given again on a change.
    gate: () => {
      const target = copyTarget(p.state, kind);
      return "reason" in target ? { enabled: c.never, reason: target.reason } : { enabled: c.always, reason: null };
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
      path: ["@open", "Open File…"],
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
      gate: () => ({ enabled: f.reopener.available() ? c.always : c.never, reason: "No file to reopen" }),
      busy: () => f.reopener.busy,
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
      gate: () => ({ enabled: c.jumpable, reason: UNSUPPORTED }),
    },
    {
      id: "jump-start",
      title: "Jump to Start",
      category: "Seek",
      home: "panel",
      bindings: ["Backspace"],
      action: () => f.jumpToStart(),
      state: () => ready(p.canRestart(), null, p.ui.restartBusy),
      gate: () => ({ enabled: c.restartable, reason: UNSUPPORTED }),
      busy: () => p.ui.restartBusy,
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
      gate: () => ({ enabled: c.rateable, reason: UNSUPPORTED }),
      busy: () => p.ui.rateBusy,
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

/** The gated commands of one scope and the state they were last given. */
type Gated = { command: Command; gate: () => Gate; last: Gate }[];

/**
 * `gate` with this plugin's `busy`: busy implies disabled (`ready`), as before. Its reason is
 * `state()`'s, as `ready` gave it: none while the source could do it (so a key passes on), the
 * underlying reason while it could not.
 */
const withBusy = (gate: () => Gate, busy: (() => boolean) | undefined, never: Condition, state: () => State) =>
  busy ? () => (busy() ? { ...gate(), enabled: never, reason: state().reason } : gate()) : gate;

/** Give each gated command its conditions again where they changed. */
function regateAll(gated: Gated) {
  for (const entry of gated) {
    const next = entry.gate();
    const { last } = entry;
    if (next.enabled === last.enabled && next.reason === last.reason && next.checked === last.checked) continue;
    entry.last = next;
    entry.command.set({ enabled: next.enabled, reason: next.reason, ...(next.checked ? { checked: next.checked } : {}) });
  }
}

/**
 * Register `table` in `scope`; the host checks `enabled`, `run` checks it again for late runs.
 * A gated command's state is conditions (`gate`), the rest is read by the SDK's update pass.
 */
function register(scope: Scope, { text }: I18n, table: Entry[], gated: Gated, never: Condition) {
  for (const { id, path, title, category, home, order, group, radio, bindings, repeat, state, gate: own, busy, action } of table) {
    const gate = own && withBusy(own, busy, never, state);
    const first = gate?.();
    const command = scope.commands.add({
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
      ...(first
        ? {
            enabled: first.enabled,
            ...(first.checked ? { checked: first.checked } : {}),
            busy: busy ?? false,
            reason: first.reason,
          }
        : {
            enabled: () => state().enabled,
            checked: () => state().checked,
            busy: () => state().busy,
            reason: () => state().reason,
          }),
      run: async () => {
        if (!state().enabled) return;
        // A run sets its busy flag at once and clears it at the end: the conditions follow both.
        const done = action();
        regateAll(gated);
        try {
          await done;
        } finally {
          regateAll(gated);
        }
      },
    });
    if (gate && first) gated.push({ command, gate, last: first });
  }
}

/**
 * Register the commands. The tier 2–4 seeks live in a child scope: a title
 * cannot change in place, so `setSteps` disposes them and adds them again with
 * the new seconds (same ids and bindings).
 */
export function addCommands(scope: Scope, i18n: I18n, playback: Playback, features: Features) {
  const c = conditions(scope.when);
  const gated: Gated = [];
  register(scope, i18n, entries(playback, features, c), gated, c.never);
  let seeks = scope.scope();
  let seekGated: Gated = [];
  register(seeks, i18n, seekEntries(playback, features.settings.value.seekSteps, i18n, features.seekBy, c), seekGated, c.never);
  // One retitle at a time (steps typed one after another): each waits for the previous one,
  // so a scope is never disposed twice nor its ids added while the old ones still hold them.
  let queue = Promise.resolve();
  return {
    setSteps(steps: SeekSteps) {
      queue = queue.catch(() => {}).then(async () => {
        await seeks.dispose();
        seeks = scope.scope();
        seekGated = [];
        register(seeks, i18n, seekEntries(playback, steps, i18n, features.seekBy, c), seekGated, c.never);
      });
      return queue;
    },
    /** After a change of what the gates are built from (the stream, a keyframe answer). */
    regate() {
      regateAll(gated);
      regateAll(seekGated);
    },
  };
}
