import type { CommandState, Scope } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { type CopyKind, copyTarget } from "./copy.ts";
import type { Keyframes } from "./keyframes.ts";
import type { OpenUrl } from "./open-url.ts";
import type { Prompt } from "./prompt.ts";
import type { Reopener } from "./reopen.ts";
import type { SeekSteps, SettingsStore } from "./settings.ts";
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
  path: string[];
  /** Menu position: doc/product/menus.md, and the 播放控制 chapter's 菜单与快捷键 table for new items. */
  order: number;
  group?: string;
  radio?: boolean;
  bindings?: string[];
  repeat?: boolean;
  state: () => CommandState;
  action: () => unknown;
};

const UNSUPPORTED = "当前来源不支持此操作";

/** Busy implies disabled; the reason is shown only when it cannot be done. */
const ready = (enabled = true, checked: boolean | null = null, busy = false, reason = UNSUPPORTED): CommandState => ({
  enabled: enabled && !busy,
  checked,
  busy,
  reason: enabled ? null : reason,
});

/**
 * Tiers 2–4 of the relative seeks (B06). Tier 1 (← →) is the core's
 * `seek-backward` / `seek-forward`, given its step with `player.setStepSeek`.
 */
function seekEntries(p: Playback, steps: SeekSteps): Entry[] {
  const tiers: [number, string, string][] = [
    [2, "Ctrl+ArrowLeft", "Ctrl+ArrowRight"],
    [3, "Shift+ArrowLeft", "Shift+ArrowRight"],
    [4, "Ctrl+Alt+ArrowLeft", "Ctrl+Alt+ArrowRight"],
  ];
  return tiers.flatMap(([tier, back, forward]) => {
    const seconds = steps[tier - 1]!;
    const order = 210 + (tier - 1) * 2;
    const seek = (id: string, title: string, at: number, delta: number, binding: string): Entry => ({
      id,
      path: ["播放", "跳转", `${title}${stepText(seconds)}秒`],
      order: at,
      group: "step",
      bindings: [binding],
      action: () => p.seekBy(delta),
      state: () => ready(p.canStep()),
    });
    return [
      seek(`backward-${tier}`, "后退", order, -seconds, back),
      seek(`forward-${tier}`, "前进", order + 1, seconds, forward),
    ];
  });
}

function entries(p: Playback, jump: Prompt, customRate: Prompt, f: Features): Entry[] {
  const rate = (id: string, title: string, order: number, value: number): Entry => ({
    id,
    path: ["播放", "速度", title],
    order,
    radio: true,
    action: () => p.setRate(value),
    state: () => ready(p.can("rate"), p.state.rate === value, p.rateBusy),
  });
  const keyframe = (id: string, title: string, order: number, direction: "backward" | "forward", binding: string): Entry => ({
    id,
    path: ["播放", "跳转", title],
    order,
    group: "frame",
    bindings: [binding],
    repeat: true,
    action: () => f.keyframes.run(direction),
    state: () => f.keyframes.state(),
  });
  const copy = (id: string, title: string, order: number, kind: CopyKind): Entry => ({
    id,
    path: ["复制到剪贴板", title],
    order,
    group: "open",
    action: () => f.copy(kind),
    state: () => {
      const target = copyTarget(p.state, kind);
      return "reason" in target ? ready(false, null, false, target.reason) : ready();
    },
  });
  const option = (id: string, title: string, order: number, value: () => boolean, toggle: () => void): Entry => ({
    id,
    path: ["播放", title],
    order,
    group: "playback-options",
    action: toggle,
    state: () => ready(true, value()),
  });
  return [
    {
      id: "open",
      path: ["打开文件…"],
      order: 100,
      group: "open",
      bindings: ["Ctrl+O", "F3"],
      action: () => p.open(),
      state: () => ready(!p.selecting, null, p.selecting),
    },
    {
      id: "open-url",
      path: ["打开", "打开链接…"],
      order: 110,
      group: "open",
      bindings: ["Ctrl+U"],
      action: () => f.openUrl.show(),
      state: () => ready(),
    },
    // A10: the former 重新打开 (only after a failed load), renamed and widened.
    {
      id: "reopen",
      path: ["重开当前/最后文件"],
      order: 121,
      group: "open",
      bindings: ["Ctrl+Y"],
      action: () => f.reopener.run(),
      state: () => ready(f.reopener.available(), null, f.reopener.busy, "没有可重开的文件"),
    },
    copy("copy-name", "文件名", 125, "name"),
    copy("copy-folder", "文件夹路径", 126, "folder"),
    copy("copy-path", "完整路径", 127, "path"),
    keyframe("keyframe-back", "上一关键帧", 206, "backward", "Ctrl+Shift+ArrowLeft"),
    keyframe("keyframe-forward", "下一关键帧", 207, "forward", "Ctrl+Shift+ArrowRight"),
    {
      id: "jump",
      path: ["播放", "跳转到…"],
      order: 218,
      bindings: ["G"],
      action: () => jump.show(),
      state: () => ready(p.canSeek()),
    },
    {
      id: "jump-start",
      path: ["播放", "跳转", "跳到开头"],
      order: 219,
      group: "points",
      bindings: ["Backspace"],
      action: () => f.jumpToStart(),
      state: () => ready(p.canRestart(), null, p.restartBusy),
    },
    {
      id: "time-jump",
      path: ["播放", "跳转", "时间跨度设置…"],
      order: 223,
      group: "jump-settings",
      action: () => f.timeJump.show(),
      state: () => ready(),
    },
    rate("rate-05", "0.5倍", 223, 0.5),
    rate("rate-15", "1.5倍", 224, 1.5),
    rate("rate-20", "2倍", 225, 2),
    {
      id: "rate-custom",
      path: ["播放", "速度", "自定义…"],
      order: 226,
      action: () => customRate.show(),
      state: () => ready(p.can("rate"), null, p.rateBusy),
    },
    option("prevent-sleep", "播放时禁止屏保", 240, () => f.settings.value.preventSleep, () => {
      f.settings.update((s) => {
        s.preventSleep = !s.preventSleep;
      });
      f.sleep();
      p.notify(`播放时禁止屏保: ${f.settings.value.preventSleep ? "开" : "关"}`);
    }),
    option("hover-time", "进度条悬停显示时间", 242, () => f.settings.value.hoverTime, () => {
      f.settings.update((s) => {
        s.hoverTime = !s.hoverTime;
      });
      p.notify(`进度条悬停显示时间: ${f.settings.value.hoverTime ? "开" : "关"}`);
    }),
    // I07: greyed out with 「需要 ffmpeg」 when ffmpeg cannot be found.
    {
      id: "hover-thumbnail",
      path: ["播放", "进度条悬停显示缩略图"],
      order: 243,
      group: "playback-options",
      action: () => {
        f.settings.update((s) => {
          s.hoverThumbnail = !s.hoverThumbnail;
        });
        p.notify(`进度条悬停显示缩略图: ${f.settings.value.hoverThumbnail ? "开" : "关"}`);
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
export function addCommands(scope: Scope, playback: Playback, jump: Prompt, customRate: Prompt, features: Features) {
  register(scope, entries(playback, jump, customRate, features));
  let seeks = scope.scope();
  register(seeks, seekEntries(playback, features.settings.value.seekSteps));
  return {
    async setSteps(steps: SeekSteps) {
      await seeks.dispose();
      seeks = scope.scope();
      register(seeks, seekEntries(playback, steps));
    },
  };
}
