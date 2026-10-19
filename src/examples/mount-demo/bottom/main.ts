/** Independent shared-source specimen; this is not the product playback plugin. */
import type { Host } from "../../../sdk/index.ts";

export async function activate(host: Host) {
  await host.ui.mount({
    slot: "bottom",
    file: "bar.slint",
    component: "SourceBar",
    global: "Bridge",
    properties: {},
  });

  let preferred = true,
    preferenceRevision = 1;
  const commandState = {
    enabled: true,
    checked: true,
    busy: false,
    reason: null,
  };
  await host.commands.register(
    "toggle-bar",
    1,
    {
      title: "Toggle bottom preference",
      path: ["视图", "Toggle bottom preference"],
      shortcut: null,
    },
    commandState,
  );
  const offCommand = host.commands.onExecute((event) => {
    if (event.id !== "toggle-bar") return;
    void (async () => {
      const current = await host.ui.mountState();
      preferred = !(current?.preferred ?? preferred);
      await host.ui.showMount(preferred);
      await host.commands.update("toggle-bar", ++preferenceRevision, {
        ...commandState,
        checked: preferred,
      });
    })().catch(console.error);
  });
  let pending = false;
  async function refresh() {
    const state = await host.source.state();
    await host.ui.setMount("source-label", state.source ?? "No source");
    await host.ui.setMount("paused", state.paused);
    await host.ui.setMount("can-pause", state.capabilities.pause);
  }

  const offUi = host.ui.onCallback((event) => {
    if (event.view === "$mount" && event.callback === "interaction") {
      void host.ui
        .interaction({
          focused: event.args[0] === true,
          dragging: false,
          popupOpen: false,
        })
        .catch(console.error);
      return;
    }
    if (
      event.view !== "$mount" ||
      event.callback !== "set-paused" ||
      typeof event.args[0] !== "boolean" ||
      pending
    )
      return;
    const paused = event.args[0];
    pending = true;
    void (async () => {
      await host.ui.setMount("busy", true);
      await host.ui.setMount("status", "Applying…");
      try {
        const state = await host.source.state();
        const operation = await host.source.submit({
          kind: "control",
          control: { kind: "pause", paused },
          expected: state.stamp,
        });
        await host.ui.setMount(
          "status",
          operation.status === "failed" ? JSON.stringify(operation.error) : "",
        );
        await refresh();
      } catch (error) {
        await host.ui.setMount("status", String(error));
      } finally {
        pending = false;
        await host.ui.setMount("busy", false);
      }
    })().catch(console.error);
  });
  const offState = await host.events.subscribe("source.state", () => {
    void refresh().catch(console.error);
  });
  await refresh();
  return () => {
    offUi();
    offCommand();
    void offState();
    // Host owner cleanup removes the mount even when dispose never runs.
  };
}
