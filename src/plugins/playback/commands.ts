import type { CommandState, PanelSlot, Scope } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import type { Editor } from "./editor.ts";

type Entry = {
  id: string;
  path: string[];
  /** Menu position; reproduces the order of the old fixed-domain menu. */
  order: number;
  bindings?: string[];
  state: () => CommandState;
  action: () => unknown;
};

/** Busy implies disabled; the reason is shown only when the source cannot do it. */
const ready = (enabled = true, checked: boolean | null = null, busy = false): CommandState => ({
  enabled: enabled && !busy,
  checked,
  busy,
  reason: enabled ? null : "当前来源不支持此操作",
});

function entries(playback: Playback, editor: Editor, bottom: PanelSlot): Entry[] {
  const p = playback;
  const entry = (
    id: string,
    path: string[],
    order: number,
    action: () => unknown,
    state: () => CommandState,
    bindings?: string[],
  ): Entry => ({ id, path, order, action, state, bindings });
  return [
    entry("open", ["文件", "打开媒体…"], 202, () => p.open(), () => ready(!p.selecting, null, p.selecting)),
    entry("close-source", ["文件", "关闭当前来源"], 201, () => p.closeSource(), () => ready(p.can("close"))),
    entry(
      "toggle-pause",
      ["播放", "播放/暂停"],
      263,
      () => p.togglePause(),
      () => ready(p.canToggle(), p.state.phase === "ready" ? p.state.paused : null, p.pauseBusy),
      ["Space"],
    ),
    entry("jump", ["播放", "跳转到…"], 264, () => editor.open("jump"), () => ready(p.canSeek())),
    ...(
      [
        ["backward-5", "后退5秒", 262, -5, "ArrowLeft"],
        ["forward-5", "前进5秒", 261, 5, "ArrowRight"],
      ] as const
    ).map(([id, title, order, delta, key]) =>
      entry(id, ["播放", title], order, () => p.seekBy(delta), () => ready(p.can("seek") && p.state.position !== null), [key]),
    ),
    ...(
      [
        ["rate-05", "0.5倍", 265, 0.5],
        ["rate-10", "正常速度", 268, 1],
        ["rate-15", "1.5倍", 266, 1.5],
        ["rate-20", "2倍", 267, 2],
      ] as const
    ).map(([id, title, order, rate]) =>
      entry(id, ["播放", "速度", title], order, () => p.setRate(rate), () => ready(p.can("rate"), p.state.rate === rate, p.rateBusy)),
    ),
    entry("rate-custom", ["播放", "速度", "自定义…"], 269, () => editor.open("rate"), () => ready(p.can("rate"), null, p.rateBusy)),
    entry("mute", ["音频", "静音"], 386, () => p.toggleMute(), () =>
      ready(true, p.state.muted, p.outputBusy || p.volumeDraft !== null),
    ),
    entry("volume", ["音频", "音量…"], 387, () => editor.open("volume"), () => ready(true, null, p.outputBusy)),
    entry("bar-visible", ["视图", "显示播放栏"], 502, () => bottom.toggle(), () => ready(true, bottom.state?.preferred ?? true)),
  ];
}

/**
 * Register every menu command and return `run(id)`, which the bar's buttons
 * use too. The bar bypasses the host's enabled check, so `run` repeats it.
 */
export function addCommands(scope: Scope, playback: Playback, editor: Editor) {
  const table = entries(playback, editor, scope.panels.bottom);
  const run = async (id: string) => {
    const command = table.find((each) => each.id === id);
    if (!command?.state().enabled) return;
    playback.error = "";
    await command.action();
  };
  for (const { id, path, order, bindings, state } of table)
    scope.commands.add({
      id,
      path,
      order,
      bindings,
      enabled: () => state().enabled,
      checked: () => state().checked,
      busy: () => state().busy,
      reason: () => state().reason,
      run: () => run(id),
    });
  return run;
}
