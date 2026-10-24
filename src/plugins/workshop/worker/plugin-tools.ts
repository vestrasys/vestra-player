// The plugin tools: fixed-parameter calls the plugin process runs for the
// agent (scaffold, type check, tests, checks, packing, install). The
// worker only forwards them over IPC; an abort cancels the call there.
import { Type, type ToolDefinition } from "../vendor/pi/pi.mjs";
import type { ToolReply } from "./protocol.ts";

const id = Type.String({ description: "Plugin id, the folder name under the work folder, e.g. octocat.subtitle-delay" });

/** Name → description and parameters. The plugin decides which are offered (StartMessage.pluginTools). */
export const PLUGIN_TOOLS: Record<string, { description: string; parameters: ReturnType<typeof Type.Object> }> = {
  scaffold_plugin: {
    description:
      "Create a new plugin project <owner>.<name>/ in the work folder from the official template (manifest.json, main.ts, ui/main.slint, tests/main.test.ts, README.md, CHANGELOG.md, package.json, tsconfig.json). The owner prefix comes from the user's settings. Returns the id and the files.",
    parameters: Type.Object({
      name: Type.String({ description: "Short name: lower-case letters, digits and -, e.g. subtitle-delay" }),
      title: Type.Optional(Type.String({ description: "Display name for the menu and manifest, e.g. Subtitle Delay" })),
    }),
  },
  typecheck: {
    description: "Strict TypeScript check of the plugin (main.ts, other .ts, tests/*.ts) and its manifest, with the player's SDK types. Returns errors as file, line, column, code, message.",
    parameters: Type.Object({ id }),
  },
  test: {
    description: "Run the plugin's tests (tests/*.test.ts, node:test with @vestra-player/sdk/testing) in a fresh restricted Node process: it may read only the plugin folder and write only os.tmpdir(), no network. Returns passed/failed tests with their assertion messages and, when a test failed, what the tests printed (console.log / console.error).",
    parameters: Type.Object({ id }),
  },
  check: {
    description: "Release check: the workshop rules (id, no npm dependencies), the manifest rules, strict types and a build (no runtime import of the SDK). Slint is compiled by the player when installing (install_plugin).",
    parameters: Type.Object({ id }),
  },
  pack: {
    description: "Build and pack the plugin as dist/<id>-<version>.vxp in the session folder.",
    parameters: Type.Object({ id }),
  },
  install_plugin: {
    description: "Pack the plugin, let the player validate it (including Slint) and install it. The user confirms in a dialog first; Roll Back in the window undoes the install (the previous version, or uninstall after a first install). Only after the user asked for it.",
    parameters: Type.Object({ id }),
  },
};

export type Ask = (name: string, args: Record<string, unknown>, signal: AbortSignal | undefined) => Promise<ToolReply>;

export function pluginTools(names: string[], ask: Ask): ToolDefinition[] {
  return names
    .filter((name) => PLUGIN_TOOLS[name])
    .map(
      (name) =>
        ({
          name,
          label: name,
          description: PLUGIN_TOOLS[name].description,
          parameters: PLUGIN_TOOLS[name].parameters,
          async execute(_id: string, params: Record<string, unknown>, signal?: AbortSignal) {
            const reply = await ask(name, params ?? {}, signal);
            // A failed check is an error result: the model reads why and fixes it.
            if (!reply.ok) throw new Error(reply.text);
            return { content: [{ type: "text", text: reply.text }], details: undefined };
          },
        }) as unknown as ToolDefinition,
    );
}
