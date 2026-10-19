/** Finite candidate command registry. Host authenticates owner; no plugin supplies an owner id. */
import type { Transport, Json } from "./index.ts";
export type CommandState = {
  enabled: boolean;
  checked: boolean | null;
  busy: boolean;
  reason: string | null;
};
export type CommandDeclaration = {
  title: string;
  path: string[];
  shortcut: string | null;
};
export type CommandExecution = { id: string; revision: number };
export function commandClient(transport: Transport) {
  return {
    register: (
      id: string,
      revision: number,
      declaration: CommandDeclaration,
      state: CommandState,
    ) =>
      transport.request<{ revision: number }>("commands.register", {
        id,
        revision,
        declaration,
        state,
      }),
    update: (id: string, revision: number, state: CommandState) =>
      transport.request<{ revision: number }>("commands.update", {
        id,
        revision,
        state,
      }),
    snapshot: () => transport.request<Json>("commands.snapshot"),
    execute: (id: string) =>
      transport.request<void>("commands.execute", { id }),
    onExecute: (listener: (event: CommandExecution) => void) =>
      transport.on("commands.execute", (data) =>
        listener(data as CommandExecution),
      ),
  };
}
