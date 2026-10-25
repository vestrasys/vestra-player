import type { I18n, Scope, Text } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { type CopyKind, copyTarget } from "./copy.ts";
import type { Keyframes } from "./keyframes.ts";
import type { OpenUrl } from "./open-url.ts";
import type { Prompt } from "./prompt.ts";
import type { Reopener } from "./reopen.ts";
import type { SeekSteps, SettingsStore } from "./settings.ts";
import type { MenuText, Reason, State } from "./texts.ts";
import type { Thumbnails } from "./thumbs.ts";
import { stepText, type TimeJump } from "./time-jump.ts";

/** The 播放控制 chapter's features (doc/product/feature-requirements.md). */
export type Features = {
  openUrl: OpenUrl;
  reopener: Reopener;
  keyframes: Keyframes;
  timeJump: TimeJump;
  /** B29: pass the switch to the host. */
  sleep: () => void;
  settings: SettingsStore;
  copy: (kind: CopyKind) => Promise<void>;
  jumpToStart: () => Promise<void>;
  thumbnails: Thumbnails;
};

type Entry = {
  id: string;
  path: (MenuText | Text)[];
  /** Menu position: doc/product/menus.md, and the 播放控制 chapter's 菜单与快捷键 table for new items. */
  order: number;
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
 * Tiers 2–4 of the relative seeks (B06). Tier 1 (← →) is the core's
 * `seek-backward` / `seek-forward`, given its step with `player.setStepSeek`.
 */
function seekEntries(p: Playback, steps: SeekSteps, { text }: I18n): Entry[] {
  const tiers: [number, string, string][] = [
    [2, "Ctrl+ArrowLeft", "Ctrl+ArrowRight"],
    [3, "Shift+ArrowLeft", "Shift+ArrowRight"],
    [4, "Ctrl+Alt+ArrowLeft", "Ctrl+Alt+ArrowRight"],
  ];
  return tiers.flatMap(([tier, back, forward]) => {
    const seconds = steps[tier - 1]!;
    const order = 210 + (tier - 1) * 2;
    const seek = (id: string, direction: "backward" | "forward", at: number, delta: number, binding: string): Entry => ({
      id,
      path: ["@playback", "Seek", direction === "backward" ? text("Back {} s", [stepText(seconds)]) : text("Forward {} s", [stepText(seconds)])],
      order: at,
      group: "step",
      bindings: [binding],
      action: () => p.seekBy(delta),
      state: () => ready(p.canStep()),
    });
    return [
      seek(`backward-${tier}`, "backward", order, -seconds, back),
      seek(`forward-${tier}`, "forward", order + 1, seconds, forward),
    ];
  });
}

function entries(p: Playback, jump: Prompt, customRate: Prompt, f: Features): Entry[] {
  const rate = (id: string, title: MenuText, order: number, value: number): Entry => ({
    id,
    path: ["@playback", "Speed", title],
    order,
    radio: true,
    action: () => p.setRate(value),
    state: () => ready(p.can("rate"), p.state.rate === value, p.rateBusy),
  });
  const keyframe = (id: string, title: MenuText, order: number, direction: "backward" | "forward", binding: string): Entry => ({
    id,
    path: ["@playback", "Seek", title],
    order,
    group: "frame",
    bindings: [binding],
    repeat: true,
    action: () => f.keyframes.run(direction),
    state: () => f.keyframes.state(),
  });
  const copy = (id: string, title: MenuText, order: number, kind: CopyKind): Entry => ({
    id,
    path: ["Copy to Clipboard", title],
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
    path: ["@playback", title],
    order,
    group: "playback-options",
    action: toggle,
    state: () => ready(true, value()),
  });
  return [
    {
      id: "open",
      path: ["Open File…"],
      order: 100,
      group: "open",
      bindings: ["Ctrl+O", "F3"],
      action: () => p.open(),
      state: () => ready(!p.selecting, null, p.selecting),
    },
    {
      id: "open-url",
      path: ["@open", "Open URL…"],
      order: 110,
      group: "open",
      bindings: ["Ctrl+U"],
      action: () => f.openUrl.show(),
      state: () => ready(),
    },
    // A10: the former Reopen (only after a failed load), renamed and widened.
    {
      id: "reopen",
      path: ["Reopen Current/Last File"],
      order: 121,
      group: "open",
      bindings: ["Ctrl+Y"],
      action: () => f.reopener.run(),
      state: () => ready(f.reopener.available(), null, f.reopener.busy, "No file to reopen"),
    },
    copy("copy-name", "File Name", 125, "name"),
    copy("copy-folder", "Folder Path", 126, "folder"),
    copy("copy-path", "Full Path", 127, "path"),
    keyframe("keyframe-back", "Previous Keyframe", 206, "backward", "Ctrl+Shift+ArrowLeft"),
    keyframe("keyframe-forward", "Next Keyframe", 207, "forward", "Ctrl+Shift+ArrowRight"),
    {
      id: "jump",
      path: ["@playback", "Jump to Time…"],
      order: 218,
      bindings: ["G"],
      action: () => jump.show(),
      state: () => ready(p.canSeek()),
    },
    {
      id: "jump-start",
      path: ["@playback", "Seek", "Jump to Start"],
      order: 219,
      group: "points",
      bindings: ["Backspace"],
      action: () => f.jumpToStart(),
      state: () => ready(p.canRestart(), null, p.restartBusy),
    },
    {
      id: "time-jump",
      path: ["@playback", "Seek", "Seek Steps…"],
      order: 223,
      group: "jump-settings",
      action: () => f.timeJump.show(),
      state: () => ready(),
    },
    rate("rate-05", "0.5×", 223, 0.5),
    rate("rate-15", "1.5×", 224, 1.5),
    rate("rate-20", "2×", 225, 2),
    {
      id: "rate-custom",
      path: ["@playback", "Speed", "Custom…"],
      order: 226,
      action: () => customRate.show(),
      state: () => ready(p.can("rate"), null, p.rateBusy),
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
      path: ["@playback", "Show Thumbnail on Seek Bar Hover"],
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
function register(scope: Scope, table: Entry[]) {
  for (const { id, path, order, group, radio, bindings, repeat, state, action } of table)
    scope.commands.add({
      id,
      path,
      order,
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
 * Register the menu commands. The tier 2–4 seeks live in a child scope: a title
 * cannot change in place, so `setSteps` disposes them and adds them again with
 * the new seconds (same ids, orders and bindings).
 */
export function addCommands(scope: Scope, i18n: I18n, playback: Playback, jump: Prompt, customRate: Prompt, features: Features) {
  register(scope, entries(playback, jump, customRate, features));
  let seeks = scope.scope();
  register(seeks, seekEntries(playback, features.settings.value.seekSteps, i18n));
  return {
    async setSteps(steps: SeekSteps) {
      await seeks.dispose();
      seeks = scope.scope();
      register(seeks, seekEntries(playback, steps, i18n));
    },
  };
}
