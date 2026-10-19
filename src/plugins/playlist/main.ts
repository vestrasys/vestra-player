import { realpath } from "node:fs/promises";
import type { Host, Json, MountVisibility } from "../../sdk/index.ts";
import type {
  SourceOperation,
  SourceState,
  SourceTicket,
} from "../../sdk/source.ts";
import { Queue, sameStamp } from "./queue.ts";
import { LatestProjection } from "./projection.ts";

const titles = {
  add: "添加文件…",
  link: "添加链接…",
  play: "播放选中项",
  previous: "上一项",
  next: "下一项",
  remove: "移除选中项",
  "move-up": "上移",
  "move-down": "下移",
  clear: "清空列表",
  visibility: "显示播放列表",
} as const;
type Action = keyof typeof titles;
type Pending = {
  id: number;
  rowId: string;
  epoch: number;
  ticket?: SourceTicket;
  unknown: boolean;
};
const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export async function activate(host: Host) {
  const queue = new Queue();
  let active = true,
    adding = false,
    linkOpen = false,
    operationId = 0;
  let pending: Pending | null = null;
  let querying = false;
  let latest: SourceState | null = null;
  let failedGeneration: number | null = null;
  let visibility: MountVisibility | null = null;
  const projectedProperties = new Map<string, string>();
  const projectedCommands = new Map<Action, string>();
  const revisions = new Map<Action, number>();
  const off: (() => void | Promise<void>)[] = [];

  const enabled = (action: Action) => {
    const index = queue.rows.findIndex((row) => row.rowId === queue.selected);
    switch (action) {
      case "add":
        return !adding;
      case "play":
        return !!queue.row(queue.selected) && !pending;
      case "previous":
        return !!queue.neighbor(-1) && !pending;
      case "next":
        return !!queue.neighbor(1) && !pending;
      case "remove":
        return index >= 0;
      case "move-up":
        return index > 0;
      case "move-down":
        return index >= 0 && index < queue.rows.length - 1;
      case "clear":
        return queue.rows.length > 0;
      default:
        return true;
    }
  };
  const commandState = (action: Action) => ({
    enabled: enabled(action),
    checked: action === "visibility" ? (visibility?.preferred ?? true) : null,
    busy:
      action === "add"
        ? adding
        : ["play", "next", "previous"].includes(action) && !!pending,
    reason:
      action === "visibility" && visibility?.reason === "space"
        ? "空间不足，放大窗口后可显示"
        : !enabled(action)
          ? pending
            ? "正在打开或结果未知"
            : "当前没有可操作项"
          : null,
  });
  // Coalesce source clock events and only send changed view/command values.
  const projection = new LatestProjection(async () => {
    try {
      if (!active) return;
      const selected = queue.row(queue.selected);
      const values: Record<string, Json> = {
        rows: queue.rows.map((row, index) => ({
          rowId: row.rowId,
          label: `${index + 1}. ${row.title}`,
          status: row.status,
          selected: row.rowId === queue.selected,
          current: row.rowId === queue.binding?.rowId,
        })),
        message: queue.message,
        details: selected
          ? `${selected.source}${selected.error ? `\n${selected.error}` : ""}`
          : "尚未选择项目",
        "can-add": enabled("add"),
        "can-play": enabled("play"),
        "can-previous": enabled("previous"),
        "can-next": enabled("next"),
        "can-remove": enabled("remove"),
        "can-up": enabled("move-up"),
        "can-down": enabled("move-down"),
        "can-clear": enabled("clear"),
      };
      for (const [property, value] of Object.entries(values)) {
        if (!active) return;
        const encoded = JSON.stringify(value);
        if (projectedProperties.get(property) === encoded) continue;
        await host.ui.setMount(property, value);
        projectedProperties.set(property, encoded);
      }
      for (const action of Object.keys(titles) as Action[]) {
        if (!active) return;
        const state = commandState(action);
        const encoded = JSON.stringify(state);
        if (projectedCommands.get(action) === encoded) continue;
        const revision = (revisions.get(action) ?? 1) + 1;
        revisions.set(action, revision);
        await host.commands.update(`playlist.${action}`, revision, state);
        projectedCommands.set(action, encoded);
      }
    } catch (error) {
      if (active) console.error("playlist projection:", errorText(error));
    }
  });
  const render = () => projection.request();

  const scheduleNext = (token: NonNullable<ReturnType<Queue["observe"]>>) => {
    queueMicrotask(() => {
      if (
        !active ||
        pending ||
        queue.epoch !== token.epoch ||
        queue.binding?.rowId !== token.rowId ||
        !latest ||
        latest.phase !== "ended" ||
        latest.terminalReason !== "natural" ||
        latest.cycle !== token.cycle ||
        !sameStamp(latest.stamp, token.stamp)
      )
        return;
      const index = queue.rows.findIndex((row) => row.rowId === token.rowId);
      const next = queue.rows[index + 1];
      if (index < 0) return;
      if (!next) {
        queue.message = "列表已播完";
        void render();
        return;
      }
      // Current order is looked up here; host CAS guards intervening public intents.
      void openRow(next.rowId, token.stamp);
    });
  };
  const observe = (state: SourceState) => {
    if (!active || (latest && state.eventSequence <= latest.eventSequence))
      return;
    latest = state;
    if (
      queue.failed &&
      failedGeneration !== null &&
      state.stamp.generation !== failedGeneration
    ) {
      queue.failed = null;
      failedGeneration = null;
    }
    if (pending?.ticket) {
      if (state.stamp.generation !== pending.ticket.stamp.generation) {
        // A later external source wins, even before this open reply arrives.
        pending = null;
        queue.detach("已切换到列表外来源");
      } else if (state.phase !== "loading") pending = null;
    }
    const next = queue.observe(state);
    if (queue.failed) failedGeneration = state.stamp.generation;
    if (next) scheduleNext(next);
    void render();
  };
  const recover = async (request: Pending, result: SourceOperation) => {
    if (!active || pending !== request) return;
    if (result.status === "failed") {
      pending = null;
      if (request.epoch !== queue.epoch || !queue.row(request.rowId)) return;
      const reason =
        typeof result.error === "object" ? result.error.backend : result.error;
      if (reason === "stale") queue.detach("自动推进已过期，保留较新的来源");
      else {
        queue.fail(request.rowId, `打开失败：${reason}`);
        failedGeneration = latest?.stamp.generation ?? null;
      }
      void render();
      return;
    }
    request.ticket = result.ticket;
    request.unknown = false;
    if (request.epoch !== queue.epoch || !queue.row(request.rowId)) {
      pending = null;
      queue.message = "当前媒体已移出列表";
      void render();
      return;
    }
    // No path equality: only the accepted ticket can establish control.
    queue.binding = {
      rowId: request.rowId,
      ticket: result.ticket,
      endedCycle: null,
    };
    const state = await host.source.state();
    if (!active || pending !== request) return;
    // Replay current snapshot even if an earlier state event preceded the reply.
    if (latest && latest.eventSequence > state.eventSequence) {
      const next = queue.observe(latest);
      if (next) scheduleNext(next);
      if (
        latest.stamp.generation !== result.ticket.stamp.generation ||
        latest.phase !== "loading"
      )
        pending = null;
    } else {
      latest = null;
      observe(state);
    }
    void render();
  };
  const openRow = async (rowId: string, expected?: SourceState["stamp"]) => {
    const row = queue.row(rowId);
    if (!active || pending || !row) return;
    const epoch = queue.begin(rowId);
    const request: Pending = {
      id: ++operationId,
      rowId,
      epoch,
      unknown: false,
    };
    pending = request;
    void render();
    try {
      try {
        // Only local paths participate in filesystem identity. URL identity is
        // the entered URL; public source routing owns provider resolution.
        const key = row.kind === "file" ? await realpath(row.source) : row.key;
        if (
          !active ||
          pending !== request ||
          request.epoch !== queue.epoch ||
          !queue.row(rowId)
        ) {
          if (pending === request) pending = null;
          void render();
          return;
        }
        const duplicate = queue.rows.find(
          (other) =>
            row.kind === "file" && other.rowId !== rowId && other.kind === "file" && other.key === key,
        );
        if (duplicate) {
          pending = null;
          queue.selected = rowId;
          queue.remove();
          queue.selected = duplicate.rowId;
          queue.message = "文件规范路径与已有项重复，已选择已有项";
          void render();
          return;
        }
        row.key = key;
      } catch {
        /* Preserve inaccessible entries; backend returns the real open error. */
      }
      if (
        !active ||
        pending !== request ||
        request.epoch !== queue.epoch ||
        !queue.row(rowId)
      ) {
        if (pending === request) pending = null;
        void render();
        return;
      }
      const result = await host.source.submit(
        {
          kind: "open",
          source: row.source,
          startPaused: false,
          ...(expected ? { expected } : {}),
        },
        request.id,
      );
      await recover(request, result);
    } catch (error) {
      if (!active || pending !== request) return;
      if ((error as { code?: string }).code === "timeout") {
        request.unknown = true;
        queue.message = "打开结果未知，正在按操作编号查询；请勿重复播放";
      } else {
        pending = null;
        if (request.epoch === queue.epoch) queue.fail(rowId, errorText(error));
      }
      void render();
    }
  };

  const action = async (name: Action) => {
    if (!active || !enabled(name)) return;
    try {
      switch (name) {
        case "add": {
          adding = true;
          void render();
          try {
            const result = await host.dialogs.open({
              kind: "openFiles",
              title: "添加到播放列表",
            });
            if (active && !result.cancelled)
              await queue.addFiles(result.paths, () => active);
          } finally {
            adding = false;
          }
          break;
        }
        case "link":
          if (!linkOpen) {
            await host.ui.open("playlist-link", "ui/link.slint", {}, [
              "add-link",
              "hide",
            ]);
            linkOpen = true;
          } else await host.ui.show("playlist-link", true);
          break;
        case "play":
          if (queue.selected) await openRow(queue.selected);
          break;
        case "previous":
        case "next": {
          const row = queue.neighbor(name === "previous" ? -1 : 1);
          if (row) await openRow(row.rowId);
          break;
        }
        case "move-up":
          queue.move(-1);
          break;
        case "move-down":
          queue.move(1);
          break;
        case "remove": {
          const removed = queue.selected;
          queue.remove();
          if (pending?.rowId === removed) {
            queue.epoch++;
            queue.message = "正在打开的媒体已移出列表";
          }
          break;
        }
        case "clear":
          queue.clear();
          break;
        case "visibility":
          visibility = await host.ui.mountState();
          await host.ui.showMount(!(visibility?.preferred ?? true));
          visibility = await host.ui.mountState();
          break;
      }
    } catch (error) {
      if (active) queue.message = errorText(error);
    }
    if (active) void render();
  };

  await host.ui.mount({
    slot: "sidebar",
    file: "ui/sidebar.slint",
    component: "PlaylistSidebar",
    global: "PlaylistBridge",
  });
  visibility = await host.ui.mountState();
  for (const name of Object.keys(titles) as Action[]) {
    revisions.set(name, 1);
    await host.commands.register(
      `playlist.${name}`,
      1,
      {
        title: titles[name],
        path:
          name === "visibility"
            ? ["视图", titles[name]]
            : ["文件", "播放列表", titles[name]],
        shortcut: null,
      },
      commandState(name),
    );
    projectedCommands.set(name, JSON.stringify(commandState(name)));
  }
  off.push(
    host.commands.onExecute((event) => {
      const name = event.id.replace(/^playlist\./, "") as Action;
      if (event.id === `playlist.${name}` && Object.hasOwn(titles, name))
        void action(name);
    }),
  );
  off.push(
    host.ui.onCallback((event) => {
      if (!active) return;
      if (event.view === "$mount") {
        const [arg] = event.args;
        if (
          event.callback === "action" &&
          typeof arg === "string" &&
          Object.hasOwn(titles, arg)
        )
          void action(arg as Action);
        else if (event.callback === "select-row" && typeof arg === "string") {
          queue.select(arg);
          void render();
        } else if (
          event.callback === "play-row" &&
          typeof arg === "string" &&
          queue.row(arg)
        ) {
          queue.select(arg);
          void action("play");
        } else if (
          event.callback === "select-step" &&
          typeof arg === "number"
        ) {
          queue.selectNeighbor(arg < 0 ? -1 : 1);
          void render();
        } else if (event.callback === "hide")
          void host.ui
            .showMount(false)
            .then(async () => {
              visibility = await host.ui.mountState();
              void render();
            })
            .catch(console.error);
        else if (event.callback === "focus-changed" && typeof arg === "boolean")
          void host.ui
            .interaction({ focused: arg, dragging: false, popupOpen: false })
            .catch(console.error);
      } else if (event.view === "playlist-link") {
        if (event.callback === "hide")
          void host.ui.show("playlist-link", false).catch(console.error);
        if (
          event.callback === "add-link" &&
          typeof event.args[0] === "string"
        ) {
          try {
            queue.addUrl(event.args[0]);
            void host.ui
              .set(
                "playlist-link",
                "message",
                "链接已添加，仅选中；显式播放需可用的来源提供者",
              )
              .catch(console.error);
            void host.ui.show("playlist-link", false).catch(console.error);
            void render();
          } catch (error) {
            void host.ui
              .set("playlist-link", "message", errorText(error))
              .catch(console.error);
          }
        }
      }
    }),
  );
  off.push(
    await host.events.subscribe("source.state", (data) =>
      observe(data as SourceState),
    ),
  );
  off.push(
    await host.events.subscribe("window.state", () => {
      void host.ui
        .mountState()
        .then((state) => {
          if (active) {
            visibility = state;
            void render();
          }
        })
        .catch(console.error);
    }),
  );
  observe(await host.source.state());
  const timer = setInterval(() => {
    const request = pending;
    if (!active || !request?.unknown || querying) return;
    querying = true;
    void host.source
      .operation(request.id)
      .then((result) => recover(request, result))
      .catch((error) => {
        if (active && pending === request) {
          request.unknown = true;
          queue.message = `打开结果仍未知：${errorText(error)}`;
          void render();
        }
      })
      .finally(() => { querying = false; });
  }, 1000);
  await render();
  return async () => {
    active = false;
    projection.stop();
    queue.epoch++;
    pending = null;
    clearInterval(timer);
    for (const unsubscribe of off) {
      try {
        await unsubscribe();
      } catch (error) {
        if ((error as { code?: string }).code !== "disabled")
          console.error(errorText(error));
      }
    }
    // The host owns mount/dialog/menu teardown. Current public media continues.
  };
}
