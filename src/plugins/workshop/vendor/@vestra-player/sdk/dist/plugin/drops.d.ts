import type { Connection } from "./connection.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Disposable, DropModifiers, Drops } from "./types.ts";
type PanelDrop = {
    paths: string[];
    at: {
        x: number;
        y: number;
    };
    modifiers: DropModifiers;
};
/**
 * One per plugin: provider handlers by id (the player names the provider
 * in `drops.drop`) and the panel's `onDrop` listeners (`drops.panel`).
 */
declare class DropService {
    #private;
    readonly handlers: Map<string, (paths: string[], modifiers: DropModifiers) => void>;
    constructor(runtime: Connection);
    id(): string;
    /** The first listener asks the player for panel drops; the last stops them. */
    listen(scope: ScopeImpl, listener: (drop: PanelDrop) => void): Disposable;
}
export declare const dropsOf: (runtime: Connection) => DropService;
export declare const drops: (scope: ScopeImpl) => Drops;
export {};
