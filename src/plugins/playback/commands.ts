import type { CommandState, Scope } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { type CopyKind, copyTarget } from "./copy.ts";
import type { OpenUrl } from "./open-url.ts";
import type { Prompt } from "./prompt.ts";
import type { Reopener } from "./reopen.ts";
import type { SettingsStore } from "./settings.ts";
import type { SleepGuard } from "./sleep.ts";

/** The round-1 features (doc/product/feature-requirements.md, 播放控制). */
export type Features = {
  openUrl: OpenUrl;
  reopener: Reopener;
  sleep: SleepGuard;
  settings: SettingsStore;
  copy: (kind: CopyKind) => Promise<void>;
  jumpToStart: () => Promise<void>;
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

function entries(p: Playback, jump: Prompt, customRate: Prompt, f: Features): Entry[] {
  const seek = (id: string, title: string, order: number, delta: number, bindings: string[], repeat = false): Entry => ({
    id,
    path: ["播放", "跳转", title],
    order,
    bindings,
    repeat,
    action: () => p.seekBy(delta),
    state: () => ready(p.canStep()),
  });
  const rate = (id: string, title: string, order: number, value: number, bindings?: string[]): Entry => ({
    id,
    path: ["播放", "速度", title],
    order,
    radio: true,
    bindings,
    action: () => p.setRate(value),
    state: () => ready(p.can("rate"), p.state.rate === value, p.rateBusy),
  });
  const rateStep = (id: string, title: string, order: number, delta: number, binding: string): Entry => ({
    id,
    path: ["播放", "速度", title],
    order,
    bindings: [binding],
    action: () => p.stepRate(delta),
    state: () => ready(p.can("rate"), null, p.rateBusy),
  });
  const volume = (id: string, title: string, order: number, delta: number, bindings: string[]): Entry => ({
    id,
    path: ["声音", title],
    order,
    bindings,
    repeat: true,
    action: () => p.volumeBy(delta),
    state: () => ready(true, null, p.outputBusy),
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
      id: "close-source",
      path: ["关闭"],
      order: 120,
      group: "open",
      bindings: ["F4"],
      action: () => p.closeSource(),
      state: () => ready(p.can("close")),
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
    {
      id: "toggle-pause",
      path: ["播放", "播放/暂停"],
      order: 201,
      bindings: ["Space"],
      action: () => p.togglePause(),
      state: () => ready(p.canToggle(), p.state.phase === "ready" ? p.state.paused : null, p.pauseBusy),
    },
    {
      id: "previous",
      path: ["播放", "上一个"],
      order: 202,
      bindings: ["PageUp", "Mouse:X1"],
      action: () => p.step("previous"),
      state: () => ready(p.state.can.previous, null, false, "没有上一个"),
    },
    {
      id: "next",
      path: ["播放", "下一个"],
      order: 203,
      bindings: ["PageDown", "Mouse:X2"],
      action: () => p.step("next"),
      state: () => ready(p.state.can.next, null, false, "没有下一个"),
    },
    seek("backward-5", "后退5秒", 210, -5, ["ArrowLeft"], true),
    seek("forward-5", "前进5秒", 211, 5, ["ArrowRight"], true),
    seek("backward-30", "后退30秒", 212, -30, ["Ctrl+ArrowLeft"]),
    seek("forward-30", "前进30秒", 213, 30, ["Ctrl+ArrowRight"]),
    seek("backward-60", "后退60秒", 214, -60, ["Shift+ArrowLeft"]),
    seek("forward-60", "前进60秒", 215, 60, ["Shift+ArrowRight"]),
    seek("backward-300", "后退300秒", 216, -300, ["Ctrl+Alt+ArrowLeft"]),
    seek("forward-300", "前进300秒", 217, 300, ["Ctrl+Alt+ArrowRight"]),
    {
      id: "jump-start",
      path: ["播放", "跳转", "跳到开头"],
      order: 219,
      group: "points",
      bindings: ["Backspace"],
      action: () => f.jumpToStart(),
      state: () => ready(p.canRestart(), null, p.pauseBusy),
    },
    {
      id: "jump",
      path: ["播放", "跳转到…"],
      order: 218,
      bindings: ["G"],
      action: () => jump.show(),
      state: () => ready(p.canSeek()),
    },
    rate("rate-10", "正常", 220, 1, ["Z"]),
    rateStep("rate-down", "减速 0.1", 221, -0.1, "X"),
    rateStep("rate-up", "加速 0.1", 222, 0.1, "C"),
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
      f.sleep.sync();
      p.notify(`播放时禁止屏保: ${f.settings.value.preventSleep ? "开" : "关"}`);
    }),
    option("hover-time", "进度条悬停显示时间", 242, () => f.settings.value.hoverTime, () => {
      f.settings.update((s) => {
        s.hoverTime = !s.hoverTime;
      });
      p.notify(`进度条悬停显示时间: ${f.settings.value.hoverTime ? "开" : "关"}`);
    }),
    volume("volume-up", "音量 +", 501, 5, ["ArrowUp", "Mouse:WheelUp"]),
    volume("volume-down", "音量 −", 502, -5, ["ArrowDown", "Mouse:WheelDown"]),
    {
      id: "mute",
      path: ["声音", "静音"],
      order: 503,
      bindings: ["M"],
      action: () => p.toggleMute(),
      state: () => ready(true, p.state.muted, p.outputBusy || p.volumeDraft !== null),
    },
  ];
}

/**
 * Register every menu command and return `run(id)`, which the bar's buttons
 * use too. The bar bypasses the host's enabled check, so `run` repeats it.
 */
export function addCommands(scope: Scope, playback: Playback, jump: Prompt, customRate: Prompt, features: Features) {
  const table = entries(playback, jump, customRate, features);
  const run = async (id: string) => {
    const command = table.find((each) => each.id === id);
    if (!command?.state().enabled) return;
    await command.action();
  };
  for (const { id, path, order, group, radio, bindings, repeat, state } of table)
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
      run: () => run(id),
    });
  return run;
}
