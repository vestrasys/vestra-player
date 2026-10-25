// Runs one plugin tool call of the agent (worker/plugin-tools.ts) in the
// plugin process: the toolchain's tools, and install when a player is at
// hand (the eval runner has none).
import type { SessionPaths, ToolResult, Toolchain } from "./toolchain.ts";

export interface Delivery {
  install(session: SessionPaths, id: string, signal: AbortSignal): Promise<ToolResult>;
}

export const BUILD_TOOLS = ["scaffold_plugin", "typecheck", "test", "check", "pack"];
export const DELIVERY_TOOLS = ["install_plugin"];

export async function runTool(
  toolchain: Toolchain,
  session: SessionPaths,
  owner: string,
  name: string,
  args: Record<string, unknown>,
  signal: AbortSignal,
  delivery?: Delivery,
): Promise<ToolResult> {
  const id = String(args?.id ?? "");
  try {
    switch (name) {
      case "scaffold_plugin":
        return await toolchain.scaffold(session, { name: String(args?.name ?? ""), title: args?.title === undefined ? undefined : String(args.title) }, owner);
      case "typecheck":
        return await toolchain.typecheck(session, id, signal);
      case "test":
        return await toolchain.test(session, id, signal);
      case "check":
        return await toolchain.check(session, id, owner, signal);
      case "pack": {
        const { ok, text } = await toolchain.pack(session, id, owner, signal);
        return { ok, text };
      }
      case "install_plugin":
        if (delivery) return await delivery.install(session, id, signal);
        break;
    }
    return { ok: false, text: `No tool named ${name}` };
  } catch (error) {
    return { ok: false, text: (error as Error).message };
  }
}
