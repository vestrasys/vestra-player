import type { Json } from "../../wire.ts";
import type { CommandWhen } from "../../commands.ts";
import type { FakeCommand, FakeBarButton, TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** A command state's reason, refused as the player refuses it. */
export declare function checkReason(host: TestHost, inner: Internals, state: Json | undefined): string | null;
export declare function bindingClaims(host: TestHost, inner: Internals): Map<string, string[]>;
/** `host/<id>` for core commands, `<manifest id>/<id>` for the plugin's. */
export declare function bindingKey(host: TestHost, _inner: Internals, id: string): string;
/**
 * `commands.keys` as the player answers it: each of the plugin's own commands,
 * and the player's core commands as `host/<id>`, with the labels of the keys it
 * holds now (the overlay, else its defaults; not a key an earlier command
 * holds), as menus write them on `systemInfo.os`.
 */
export declare function commandKeys(host: TestHost, inner: Internals): {
    keys: Record<string, string[]>;
    revision: number;
};
/** Push `commands.keys` when the labels changed and the plugin listens. */
export declare function keysChanged(host: TestHost, inner: Internals): void;
export declare function splitCommandKey(_host: TestHost, _inner: Internals, key: string): {
    plugin: string;
    command: string;
};
/**
 * The capabilities of the plugin under test's manifest, set by
 * `startPlugin`; a host without an entry (requests sent by hand) lists
 * every command in `bindings.list`.
 */
export declare const pluginCapabilities: WeakMap<TestHost, readonly string[]>;
/**
 * `bindings.list`, as the player's (binding_manager.rs `list`, player
 * 0.8.0): a `focus: "panel"` command of a plugin whose manifest declares no
 * `ui.mount` can never fire from a key, so it is left out; it stays declared
 * (its stored binding is no orphan, `bindings.set` accepts it).
 */
export declare function bindingTable(host: TestHost, inner: Internals): {
    commands: {
        title: string;
        path: string[] | null;
        key: string[] | null;
        category: string | null;
        scope: "panel" | "global";
        defaults: string[];
        current: string[];
        overridden: boolean;
        shadowed: {
            input: string;
            by: {
                plugin: string;
                command: string;
            };
        }[];
        labels: string[];
        plugin: string;
        command: string;
    }[];
    orphans: {
        ref: {
            plugin: string;
            command: string;
        };
        inputs: string[];
    }[];
};
/** A command's state as the player shows it: its conditions evaluated. */
export declare function commandShown(host: TestHost, _inner: Internals, sent: FakeCommand["state"], when: CommandWhen | undefined): FakeCommand["state"];
/** Implements `TestHost.press`. */
export declare function press(host: TestHost, inner: Internals, binding: string, options?: {
    panel?: boolean;
}): Promise<"ran" | "reason" | "passed">;
/** Implements `TestHost.barButtons`. */
export declare function barButtons(host: TestHost, _inner: Internals): readonly FakeBarButton[];
/** The default handlers of this area's wire methods, by method name. */
export declare function commandsHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
/** The default handlers of `commands.run` and `palette.lead`. */
export declare function builtinHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
/** Implements `TestHost.runPaletteLead`. */
export declare function runPaletteLead(host: TestHost, inner: Internals, query: string, options: {
    keep?: boolean;
    status?: boolean;
}): Promise<string>;
