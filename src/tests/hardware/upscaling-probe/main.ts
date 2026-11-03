// Hardware test plugin for AI upscaling (doc/architecture/README.md §4.4,
// cases U1–U6): drives plugin.video.upscaling
// from the Video menu or keys, and shows and logs every answer. Loaded with
// `--plugin src/tests/hardware/upscaling-probe`; not a product plugin (the
// real one is step V4's).
import type { Plugin, PluginError, UpscalingState } from "@vestra-player/sdk";

export default function activate(plugin: Plugin) {
  const report = async (step: string, state: UpscalingState) => {
    // One line in the player's log (`plugin … stdout`) per answer.
    console.log(`upscaling-probe ${step}: ${JSON.stringify(state)}`);
    await plugin.osd.show(
      "{}: available {} ({}), enabled {}, requested {}, factor {}",
      [
        step,
        String(state.available),
        state.reason?.code ?? "-",
        String(state.enabled),
        String(state.requested),
        String(state.factor ?? "-"),
      ],
    );
  };
  const refused = async (step: string, error: unknown) => {
    const { code, message } = error as PluginError;
    console.log(`upscaling-probe ${step}: ${code}: ${message}`);
    await plugin.osd.show("{}: {}: {}", [step, code, message]);
  };
  const run =
    (step: string, call: () => Promise<UpscalingState>) => async () => {
      try {
        await report(step, await call());
      } catch (error) {
        await refused(step, error);
      }
    };
  plugin.commands.add({
    id: "state",
    path: ["@video", "Upscaling Probe", "State"],
    bindings: ["Ctrl+Alt+U"],
    run: run("state", () => plugin.video.upscaling.state()),
  });
  plugin.commands.add({
    id: "on",
    path: ["@video", "Upscaling Probe", "On"],
    bindings: ["Ctrl+Alt+I"],
    run: run("on", () => plugin.video.upscaling.set(true)),
  });
  plugin.commands.add({
    id: "off",
    path: ["@video", "Upscaling Probe", "Off"],
    bindings: ["Ctrl+Alt+O"],
    run: run("off", () => plugin.video.upscaling.set(false)),
  });
}
