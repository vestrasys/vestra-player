// Hardware test plugin for the video primitives (doc/architecture/README.md
// §4.5, cases H1–H9): drives a `convolution` /
// `lut3d` filter chain, the shader channel and a processed snapshot from the
// Video menu, and shows and logs every status. Two copies with different ids
// (`a/` and `b/`, the same main.ts and assets: keep them identical) give two
// plugins for the shader-order and compile-error cases; only copy A has
// keys. Not a product plugin. Load with
// `--plugin src/tests/hardware/video-primitives-probe/a` (and `/b`).
import type {
  FilterStatus,
  Plugin,
  PluginError,
  VideoNode,
} from "@vestra-player/sdk";
import { join } from "node:path";

const SHARPEN: VideoNode = {
  kind: "convolution",
  planes: [
    {
      mode: "square",
      matrix: [0, -1, 0, -1, 5, -1, 0, -1, 0],
      rdiv: 1,
      bias: 0,
    },
  ],
};
// A 5×5 box blur on luma; rdiv 0 divides by the sum (25).
const BLUR: VideoNode = {
  kind: "convolution",
  planes: [{ mode: "square", matrix: Array(25).fill(1), rdiv: 0, bias: 0 }],
};

export default async function activate(plugin: Plugin) {
  await plugin.player.read(); // was "playback.read"
  const name = plugin.id.endsWith("-b") ? "B" : "A";
  const menu = `Primitives Probe ${name}`;
  const here = import.meta.dirname;
  const say = async (text: string) => {
    // One line in the player's log (`plugin … stdout`) per event.
    console.log(`video-primitives-probe ${name}: ${text}`);
    await plugin.osd.show("Probe {}: {}", [name, text]);
  };
  const shown = (what: string) => (status: FilterStatus) =>
    void say(
      status.state === "failed"
        ? `${what} failed: ${status.error.code}: ${status.error.message}`
        : `${what} ${status.state}`,
    );
  const filter = plugin.video.filter("probe");
  filter.onStatus(shown("filter"));
  const shaders = plugin.video.shaders("probe");
  shaders.onStatus(shown("shaders"));

  const items: [string, string, string | null, () => unknown][] = [
    ["sharpen", "Convolution Sharpen", "1", () => filter.set([SHARPEN])],
    ["blur", "Convolution Blur 5x5", "2", () => filter.set([BLUR])],
    [
      "lut-warm",
      "LUT Warm",
      "3",
      () =>
        filter.set([
          {
            kind: "lut3d",
            file: join(here, "looks", "warm.cube"),
            interpolation: "tetrahedral",
          },
        ]),
    ],
    [
      "lut-invert",
      "LUT Invert",
      "4",
      () =>
        filter.set([
          {
            kind: "lut3d",
            file: join(here, "looks", "invert.cube"),
            interpolation: "trilinear",
          },
        ]),
    ],
    ["filter-clear", "Clear Filter", "5", () => filter.set([])],
    [
      "shader-tint",
      "Shader Tint (Red)",
      "6",
      () => shaders.set([join(here, "shaders", "tint.glsl")]),
    ],
    [
      "shader-invert",
      "Shader Invert",
      "7",
      () => shaders.set([join(here, "shaders", "invert.glsl")]),
    ],
    [
      "shader-broken",
      "Shader Broken",
      "8",
      () => shaders.set([join(here, "shaders", "broken.glsl")]),
    ],
    ["shader-clear", "Clear Shaders", "9", () => shaders.clear()],
    ["snapshot", "Processed Snapshot", "0", () => snapshot()],
  ];
  for (const [id, title, key, run] of items)
    plugin.commands.add({
      id,
      path: ["@video", menu, title],
      ...(name === "A" && key ? { bindings: [`Ctrl+Alt+${key}`] } : {}),
      run,
    });

  // H2: a processed snapshot (this plugin's chain applied) as a PNG in the
  // data directory; the path goes to the OSD and the log.
  async function snapshot() {
    try {
      const shot = await plugin.media.capture({ processed: true });
      const file = join(plugin.dataDirectory, `snapshot-${Date.now()}.png`);
      const job = await shot.export({ format: "png", destination: file });
      const state = await job.done;
      await shot.dispose();
      await say(`snapshot ${state.status}: ${file}`);
    } catch (error) {
      const { code, message } = error as PluginError;
      await say(`snapshot: ${code}: ${message}`);
    }
  }
}
