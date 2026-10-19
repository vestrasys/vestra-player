import type { Host, PlaybackState } from "../../sdk/index.ts";
/** API exercise only. Product playback controls are designed in the following phase. */
function report(error: unknown) {
  if (
    error &&
    typeof error === "object" &&
    "code" in error &&
    error.code === "disabled"
  )
    return;
  console.error(error);
}
export async function activate(host: Host) {
  const capabilities = await host.capabilities();
  if (process.env.VESTRA_DEMO_MEDIA)
    await host.playback.open(process.env.VESTRA_DEMO_MEDIA);
  if (capabilities.ui) {
    await host.ui.open(
      "sample",
      "sample.slint",
      { status: "Waiting for backend state" },
      ["toggle", "adjust"],
    );
  }
  const callbacks = host.ui.onCallback((event) => {
    const action = async () => {
      if (event.callback === "toggle") {
        const state = await host.playback.state();
        await host.playback.set("pause", !state.paused);
      }
      if (event.callback === "adjust")
        await host.playback.set("brightness", Number(event.args[0]));
    };
    void action().catch(report);
  });
  await host.events.subscribe("playback.state", (data) => {
    const state = data as PlaybackState;
    if (capabilities.ui)
      void host.ui
        .set(
          "sample",
          "status",
          `Session ${state.session} · ${(state.position ?? 0).toFixed(2)}s · frame ${state.videoFrames}`,
        )
        .catch(report);
  });
  return async () => {
    callbacks();
    // The host clears subscriptions on disable; stop handlers do not issue RPC.
  };
}
