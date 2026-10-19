import type { Host, Json } from "../../sdk/index.ts";
import type {
  SourceState,
  SourceIntent,
  SourceOperation,
} from "../../sdk/source.ts";
import type { CommandState } from "../../sdk/commands.ts";
import { LatestValue, SeekFence, parseTime, time } from "./interaction.ts";

type Command = {
  id: string;
  title: string;
  path: string[];
  shortcut?: string;
  action: () => Promise<void>;
  state: () => CommandState;
  revision: number;
  last: string;
};
export async function activate(host: Host) {
  let active = true,
    state = await host.source.state(),
    operationId = 0;
  let selecting = false,
    pauseBusy = false,
    rateBusy = false,
    outputBusy = false;
  let preferred = true,
    error = "",
    editor = "",
    draft: number | null = null,
    dragging = false;
  let seekFence: SeekFence | null = null,
    seekTarget: number | null = null;
  let blockedGesture = false,
    editorStamp = "",
    volumeDraft: number | null = null,
    volumeRevision = 0;
  let mounted = false,
    projecting = false,
    dirty = false;
  const commands: Command[] = [];
  const cache = new Map<string, Json>();
  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const report = (e: unknown) => {
    error = message(e);
    schedule();
  };
  const can = (kind: "pause" | "seek" | "rate" | "close") =>
    state.capabilities[kind] &&
    (kind === "close" ? state.phase !== "empty" : state.phase === "ready");
  const canSeek = () =>
    can("seek") && state.duration !== null && state.duration > 0;
  const ready = (
    enabled = true,
    checked: boolean | null = null,
    busy = false,
    reason = "当前来源不支持此操作",
  ): CommandState => ({
    enabled: enabled && !busy,
    checked,
    busy,
    reason: enabled ? null : reason,
  });
  async function refresh() {
    if (active) accept(await host.source.state());
  }
  function clearSeek() {
    seekFence?.cancel();
    seekFence = null;
    seeks.clear();
    draft = null;
    seekTarget = null;
  }
  function accept(next: SourceState) {
    if (next.eventSequence < state.eventSequence) return;
    const changedIntent =
      next.stamp.generation !== state.stamp.generation ||
      next.stamp.intentVersion !== state.stamp.intentVersion;
    const invalidGesture = seekFence !== null && !seekFence.observe(next.stamp);
    if (next.stamp.generation !== state.stamp.generation || invalidGesture) {
      clearSeek();
      blockedGesture = true;
      dragging = false;
      editor = "";
      error = "";
      void host.ui
        .interaction({ focused: false, dragging: false, popupOpen: false })
        .catch(report);
    }
    if (changedIntent && editorStamp !== JSON.stringify(next.stamp))
      editor = "";
    state = next;
    schedule();
  }
  async function operation(
    intent: SourceIntent,
    current: () => boolean = () => active,
  ) {
    if (!current()) throw new Error("操作已过期");
    const id = ++operationId;
    let result: SourceOperation;
    try {
      result = await host.source.submit(intent, id);
    } catch (e) {
      if (current()) {
        error = `结果未知，正在核对：${message(e)}`;
        schedule();
      }
      result = await host.source.operation(id);
    }
    const deadline = Date.now() + 12000;
    while (active && result.status === "pending") {
      if (Date.now() > deadline) {
        await refresh();
        throw new Error("结果未知；已读取当前来源状态，请核对后操作");
      }
      await new Promise((resolve) => setTimeout(resolve, 60));
      result = await host.source.operation(id);
    }
    await refresh();
    if (result.status === "failed")
      throw new Error(`来源操作失败：${JSON.stringify(result.error)}`);
    if (!active || result.status !== "completed")
      throw new Error("操作已结束或插件已停用");
    return result.ticket;
  }
  const seeks = new LatestValue<{ seconds: number; fence: SeekFence }>(
    async ({ seconds, fence }) => {
      if (!active || !fence.valid || seekFence !== fence) return;
      const expected = fence.begin();
      if (!expected) return;
      const ticket = await operation(
        { kind: "control", control: { kind: "seek", seconds }, expected },
        () => active && seekFence === fence && fence.valid,
      );
      if (seekFence !== fence || !fence.complete(ticket.stamp)) return;
      if (!dragging && seekTarget === seconds) {
        seekTarget = null;
        draft = null;
        schedule();
      }
    },
    (e) => {
      seekFence?.cancel();
      seekFence = null;
      draft = null;
      seekTarget = null;
      report(e);
    },
  );
  const volumes = new LatestValue<{ value: number; revision: number }>(
    async ({ value, revision }) => {
      // Set the requested lower/higher target before unmuting; never restore an old loud value.
      await host.playback.set("volume", value);
      if (!active) return;
      await host.playback.set("muted", false);
      await refresh();
      if (volumeRevision === revision) {
        volumeDraft = null;
        schedule();
      }
    },
    (e) => {
      volumeDraft = null;
      report(e);
    },
  );
  function setVolume(value: number) {
    if (outputBusy || !Number.isFinite(value)) return;
    volumeDraft = Math.max(0, Math.min(200, value));
    volumes.push({ value: volumeDraft, revision: ++volumeRevision });
    schedule();
  }
  function seek(seconds: number) {
    if (!can("seek")) return;
    seconds = Math.max(0, Math.min(state.duration ?? Infinity, seconds));
    draft = seconds;
    seekTarget = seconds;
    error = "";
    if (!seekFence?.valid) seekFence = new SeekFence(state.stamp);
    seeks.push({ seconds, fence: seekFence });
    schedule();
  }
  async function open() {
    if (selecting) return;
    selecting = true;
    error = "";
    schedule();
    try {
      const result = await host.dialogs.open({
        kind: "openFile",
        title:
          state.phase === "empty" ? "打开媒体" : "打开媒体（将替换当前来源）",
      });
      if (active && !result.cancelled && result.paths[0]) {
        clearSeek();
        await operation({
          kind: "open",
          source: result.paths[0],
          startPaused: false,
        });
      }
    } finally {
      selecting = false;
      schedule();
    }
  }
  async function pause() {
    if (
      pauseBusy ||
      !(can("pause") || (state.phase === "ended" && state.capabilities.restart))
    )
      return;
    clearSeek();
    pauseBusy = true;
    error = "";
    schedule();
    try {
      await operation(
        state.phase === "ended"
          ? { kind: "restart", expected: state.stamp }
          : {
              kind: "control",
              control: { kind: "pause", paused: !state.paused },
              expected: state.stamp,
            },
      );
    } finally {
      pauseBusy = false;
      schedule();
    }
  }
  async function rate(value: number) {
    if (!can("rate") || rateBusy) return;
    if (!Number.isFinite(value) || value < 0.25 || value > 4)
      throw new Error("速度范围为0.25–4倍");
    clearSeek();
    rateBusy = true;
    schedule();
    try {
      await operation({
        kind: "control",
        control: { kind: "rate", rate: value },
        expected: state.stamp,
      });
    } finally {
      rateBusy = false;
      schedule();
    }
  }
  async function mute() {
    if (outputBusy || volumeDraft !== null) return;
    outputBusy = true;
    schedule();
    try {
      await host.playback.set("muted", !state.output.muted);
      await refresh();
    } finally {
      outputBusy = false;
      schedule();
    }
  }
  async function edit(kind: string) {
    editor = kind;
    editorStamp = JSON.stringify(state.stamp);
    error = "";
    await host.ui.setMount(
      "editor-value",
      kind === "jump"
        ? time(state.position)
        : kind === "rate"
          ? String(state.rate)
          : String(state.output.volume),
    );
    await host.ui.interaction({
      focused: true,
      dragging: false,
      popupOpen: true,
    });
    schedule();
  }
  function add(
    id: string,
    title: string,
    path: string[],
    action: () => Promise<void>,
    derive: () => CommandState,
    shortcut?: string,
  ) {
    commands.push({
      id,
      title,
      path,
      action,
      state: derive,
      shortcut,
      revision: 1,
      last: "",
    });
  }
  add(
    "open",
    "打开媒体…",
    ["文件", "打开媒体…"],
    open,
    () => ready(!selecting, null, selecting),
  );
  add(
    "close-source",
    "关闭当前来源",
    ["文件", "关闭当前来源"],
    async () => {
      clearSeek();
      await operation({ kind: "close", expected: state.stamp });
    },
    () => ready(can("close")),
  );
  add(
    "toggle-pause",
    "播放/暂停",
    ["播放", "播放/暂停"],
    pause,
    () =>
      ready(
        can("pause") || (state.phase === "ended" && state.capabilities.restart),
        state.phase === "ready" ? state.paused : null,
        pauseBusy,
      ),
    "Space",
  );
  add(
    "jump",
    "跳转到…",
    ["播放", "跳转到…"],
    () => edit("jump"),
    () => ready(canSeek()),
  );
  for (const [id, title, delta, shortcut] of [
    ["backward-5", "后退5秒", -5, "ArrowLeft"],
    ["forward-5", "前进5秒", 5, "ArrowRight"],
  ] as const)
    add(
      id,
      title,
      ["播放", title],
      async () => {
        seek((seekTarget ?? state.position ?? 0) + delta);
      },
      () => ready(can("seek") && state.position !== null),
      shortcut,
    );
  for (const [id, title, value] of [
    ["rate-05", "0.5倍", 0.5],
    ["rate-10", "正常速度", 1],
    ["rate-15", "1.5倍", 1.5],
    ["rate-20", "2倍", 2],
  ] as const)
    add(
      id,
      title,
      ["播放", "速度", title],
      () => rate(value),
      () => ready(can("rate"), state.rate === value, rateBusy),
    );
  add(
    "rate-custom",
    "自定义…",
    ["播放", "速度", "自定义…"],
    () => edit("rate"),
    () => ready(can("rate"), null, rateBusy),
  );
  add("mute", "静音", ["音频", "静音"], mute, () =>
    ready(true, state.output.muted, outputBusy || volumeDraft !== null),
  );
  add(
    "volume",
    "音量…",
    ["音频", "音量…"],
    () => edit("volume"),
    () => ready(true, null, outputBusy),
  );
  add(
    "bar-visible",
    "显示播放栏",
    ["视图", "显示播放栏"],
    async () => {
      const mount = await host.ui.mountState();
      await host.ui.showMount(!(mount?.preferred ?? preferred));
      preferred = (await host.ui.mountState())?.preferred ?? preferred;
      schedule();
    },
    () => ready(true, preferred),
  );
  async function run(id: string) {
    const command = commands.find((c) => c.id === id);
    if (!active || !command?.state().enabled) return;
    error = "";
    try {
      await command.action();
    } catch (e) {
      report(e);
    }
  }
  function schedule() {
    dirty = true;
    if (mounted && !projecting && active)
      void project().catch((e) => {
        error = message(e);
        console.error("播放栏投影失败", e);
      });
  }
  async function project() {
    projecting = true;
    try {
      while (dirty && active) {
        dirty = false;
        const position = draft ?? state.position;
        const status =
          error ||
          (state.phase === "loading"
            ? "正在打开"
            : state.phase === "error"
              ? `打开失败：${JSON.stringify(state.terminalReason)}`
              : state.phase === "ended"
                ? "已结束"
                : seekTarget !== null
                  ? "跳转中"
                  : state.phase === "empty"
                    ? "未打开媒体"
                    : state.paused
                      ? "已暂停"
                      : "播放中");
        const props: Record<string, Json> = {
          "source-label": state.source ?? "未打开媒体",
          status: status,
          "time-label": `${time(position)} / ${time(state.duration)}`,
          progress:
            state.duration && position !== null ? position / state.duration : 0,
          "can-seek": canSeek(),
          "can-pause": commands.find((c) => c.id === "toggle-pause")!.state()
            .enabled,
          "pause-label": pauseBusy
            ? "等待…"
            : state.phase === "ended"
              ? "重播"
              : state.paused
                ? "播放"
                : "暂停",
          "can-open": !selecting,
          muted: state.output.muted,
          volume: volumeDraft ?? state.output.volume,
          "volume-label": `${Math.round(volumeDraft ?? state.output.volume)}%`,
          "rate-label": `${state.rate}倍`,
          "can-rate": can("rate") && !rateBusy,
          "can-mute": !outputBusy && volumeDraft === null,
          "can-volume": !outputBusy,
          editor: editor,
          "can-retry": state.phase === "error" && state.source !== null,
        };
        for (const [key, value] of Object.entries(props))
          if (cache.get(key) !== value) {
            await host.ui.setMount(key, value);
            cache.set(key, value);
          }
        for (const c of commands) {
          const value = c.state(),
            serialized = JSON.stringify(value);
          if (c.last !== serialized) {
            await host.commands.update(c.id, ++c.revision, value);
            c.last = serialized;
          }
        }
      }
    } finally {
      projecting = false;
    }
  }
  const offCommand = host.commands.onExecute((e) => {
    void run(e.id);
  });
  const offUi = host.ui.onCallback((e) => {
    if (e.view !== "$mount" || !active) return;
    const value = e.args[0];
    if (e.callback === "action" && typeof value === "string") void run(value);
    if ((e.callback === "volume-changed" || e.callback === "volume-released") && typeof value === "number") setVolume(value);
    if (e.callback === "seek-cancelled") {
      clearSeek();
      blockedGesture = true;
      dragging = false;
      void host.ui.interaction({ focused: false, dragging: false, popupOpen: editor !== "" }).catch(report);
      void refresh().catch(report);
      schedule();
    }
    if (
      e.callback === "seek-changed" &&
      typeof value === "number" &&
      canSeek() &&
      !blockedGesture
    )
      seek(value * state.duration!);
    if (e.callback === "dragging") {
      dragging = value === true;
      if (dragging) {
        blockedGesture = false;
        clearSeek();
        seekFence = new SeekFence(state.stamp);
      }
      void host.ui
        .interaction({ focused: false, dragging, popupOpen: editor !== "" })
        .catch(report);
      if (!dragging) schedule();
    }
    if (e.callback === "interaction")
      void host.ui
        .interaction({
          focused: value === true,
          dragging,
          popupOpen: editor !== "",
        })
        .catch(report);
    if (e.callback === "cancel-editor") {
      editor = "";
      void host.ui
        .interaction({ focused: false, dragging, popupOpen: false })
        .catch(report);
      schedule();
    }
    if (e.callback === "confirm-editor" && typeof value === "string")
      void (async () => {
        if (editorStamp !== JSON.stringify(state.stamp))
          throw new Error("来源已变化，请重新打开编辑器");
        if (editor === "jump") {
          const seconds = parseTime(value);
          if (seconds === null || !canSeek() || seconds > state.duration!)
            throw new Error("请输入来源时长范围内的HH:MM:SS.mmm");
          seek(seconds);
        }
        if (editor === "rate") await rate(Number(value));
        if (editor === "volume") {
          const amount = Number(value);
          if (!value.trim() || !Number.isFinite(amount) || amount < 0 || amount > 200)
            throw new Error("音量范围为0–200%");
          setVolume(amount);
        }
        editor = "";
        await host.ui.interaction({
          focused: false,
          dragging,
          popupOpen: false,
        });
        schedule();
      })().catch(report);
    if (e.callback === "retry" && state.phase === "error" && state.source)
      void operation({
        kind: "open",
        source: state.source,
        startPaused: false,
      }).catch(report);
  });
  for (const c of commands) {
    const value = c.state();
    await host.commands.register(
      c.id,
      c.revision,
      { title: c.title, path: c.path, shortcut: c.shortcut ?? null },
      value,
    );
    c.last = JSON.stringify(value);
  }
  await host.ui.mount({
    slot: "bottom",
    file: "ui/bar.slint",
    component: "PlaybackBar",
    global: "PlaybackBridge",
    properties: {},
  });
  mounted = true;
  preferred = (await host.ui.mountState())?.preferred ?? true;
  const offState = await host.events.subscribe("source.state", () => {
    void refresh().catch(report);
  });
  const offWindow = await host.events.subscribe("window.state", () => {
    void host.ui
      .mountState()
      .then((mount) => {
        if (mount) {
          preferred = mount.preferred;
          schedule();
        }
      })
      .catch(report);
  });
  await refresh();
  schedule();
  return () => {
    active = false;
    clearSeek();
    volumes.clear();
    offUi();
    offCommand();
    void offState().catch(() => {});
    void offWindow().catch(() => {});
  };
}
