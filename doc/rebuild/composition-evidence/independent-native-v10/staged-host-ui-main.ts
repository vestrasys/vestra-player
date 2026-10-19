/** Shared-infrastructure specimen; no product behavior is privileged by the host. */
import type { Host } from "../../sdk/index.ts";
export async function activate(host: Host) {
  const offErrors = await host.events.subscribe("host.error", (data) =>
    console.error("observed host.error", data),
  );
  const offWindow = await host.events.subscribe("window.state", (data) =>
    console.info("window.state", JSON.stringify(data)),
  );
  const ready = { enabled: true, checked: null, busy: false, reason: null };
  let revision = 1;
  const off = host.commands.onExecute((event) => {
    void run(event.id).catch(console.error);
  });
  await host.commands.register(
    "pause",
    1,
    { title: "Pause / play", path: ["播放", "Pause / play"], shortcut: null },
    ready,
  );
  await host.commands.register(
    "open",
    1,
    {
      title: "Open native file dialog",
      path: ["文件", "Open native file dialog"],
      shortcut: null,
    },
    ready,
  );
  await host.commands.register(
    "disabled",
    1,
    {
      title: "Disabled example",
      path: ["文件", "Disabled example"],
      shortcut: null,
    },
    { ...ready, enabled: false, reason: "Example reason" },
  );
  await host.commands.register(
    "workbench",
    1,
    {
      title: "Processing specimen",
      path: ["视频", "Processing specimen"],
      shortcut: null,
    },
    ready,
  );
  for (const [id, title] of [
    ["disable-sidebar", "Disable sidebar specimen"],
    ["reload-sidebar", "Reload sidebar specimen"],
    ["bad-mount", "Load syntax-error sidebar specimen"],
  ]) {
    await host.commands.register(
      id,
      1,
      { title, path: ["插件", title], shortcut: null },
      ready,
    );
  }
  await host.commands.register(
    "fullscreen",
    1,
    {
      title: "Toggle fullscreen",
      path: ["视图", "Toggle fullscreen"],
      shortcut: "F11",
    },
    ready,
  );
  let workbench = false;
  async function run(id: string) {
    if (id === "fullscreen") {
      const state = await host.window.state();
      if (state) await host.window.setFullscreen(!state.fullscreen);
    }
    if (id === "disable-sidebar")
      await host.plugins.disable("example.mount-list");
    if (id === "reload-sidebar" && process.env.VESTRA_SIDEBAR_PACKAGE)
      await host.plugins.load(process.env.VESTRA_SIDEBAR_PACKAGE);
    if (id === "bad-mount" && process.env.VESTRA_BAD_MOUNT_PLUGIN)
      await host.plugins.load(process.env.VESTRA_BAD_MOUNT_PLUGIN);
    if (id === "open")
      console.info(
        "native dialog",
        await host.dialogs.open({
          kind: "openFile",
          title: "Shared native dialog specimen",
        }),
      );
    if (id === "pause") {
      const s = await host.source.state();
      await host.source.submit({
        kind: "control",
        control: { kind: "pause", paused: !s.paused },
      });
      await host.commands.update("pause", ++revision, {
        ...ready,
        checked: !s.paused,
      });
    }
    if (id === "workbench") {
      if (!workbench) {
        await host.ui.open("processing", "processing.slint", {}, [
          "brightness",
          "audio-gain",
        ]);
        workbench = true;
      } else await host.ui.show("processing", true);
    }
  }
  let videoRevision = 0,
    audioRevision = 0;
  const offUi = host.ui.onCallback((event) => {
    const value = event.args[0];
    if (typeof value !== "number") return;
    void (async () => {
      const s = await host.source.state();
      if (event.callback === "brightness")
        await host.processing.replace({
          stage: "video",
          resourceId: "tone",
          revision: ++videoRevision,
          expectedGeneration: s.stamp.generation,
          nodes: [
            {
              kind: "eq",
              brightness: value,
              contrast: 1,
              saturation: 1,
              gamma: 1,
            },
            { kind: "gblur", sigma: 0.5, steps: 3 },
          ],
        });
      if (event.callback === "audio-gain")
        await host.processing.replace({
          stage: "audio",
          resourceId: "tone",
          revision: ++audioRevision,
          expectedGeneration: s.stamp.generation,
          nodes: [
            { kind: "volume", gainDb: -12 },
            { kind: "equalizer", frequency: 1000, q: 1, gainDb: value },
            { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 0 },
            { kind: "dynaudnorm", enabled: true, maxGain: 2, peak: 0.9 },
          ],
        });
    })().catch(console.error);
  });
  if (process.env.VESTRA_DEMO_MEDIA)
    await host.source.submit({
      kind: "open",
      source: process.env.VESTRA_DEMO_MEDIA,
      startPaused: false,
    });
  return () => {
    off();
    offUi();
    void offErrors();
    void offWindow();
  };
}
