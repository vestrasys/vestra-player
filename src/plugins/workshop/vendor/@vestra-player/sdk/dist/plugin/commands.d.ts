import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Commands, Palette } from "./types.ts";
/**
 * One per plugin: the SDK's copy of the key labels of the plugin's own
 * commands. Started by the first `keys` read or a `player.on("commands.keys")`: it
 * subscribes to `commands.keys` (a mirror, unless a listener or a binding holds it), then
 * asks once; the player pushes every later change. Answers and events carry the
 * binding table's revision, and an older one than what the copy has is dropped.
 */
declare class KeysService {
    #private;
    constructor(runtime: Runtime);
    start(): void;
    get(id: string): readonly string[];
}
export declare const keysOf: (runtime: Runtime) => KeysService;
export declare const commands: (scope: ScopeImpl) => Commands;
export declare const palette: (scope: ScopeImpl) => Palette;
export {};
