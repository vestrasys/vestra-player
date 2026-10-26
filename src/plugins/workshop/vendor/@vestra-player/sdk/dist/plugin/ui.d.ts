import type { Json } from "../wire.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Dialogs, Disposable, ImageOptions, ImageRef, Panels, PropValue, Props, Ui, View, Windows } from "./types.ts";
/** Slint names are kebab-case; plugin code may use camelCase. */
export declare const kebab: (name: string) => string;
/**
 * `plugin.ui.image`. The wire form (`{ $vestraImage: path, ref }`) is an
 * internal detail; `ref` makes every call a new value, so a file rewritten
 * under the same path is sent (and decoded) again. The player checks the
 * path on every use, so a hand-made wire value gains nothing.
 */
export declare class ImageHandle implements ImageRef {
    #private;
    readonly path: string;
    readonly __vestraImage: true;
    constructor(scope: ScopeImpl, path: string, maxSide?: number);
    /** Released with its scope: writing it is `invalid`. */
    get released(): boolean;
    toJSON(): Json;
}
/** `ImageOptions.maxSide`, checked: an integer 1–4096 or absent. */
export declare function imageSide(options: ImageOptions | undefined): number | undefined;
export declare const ui: (scope: ScopeImpl) => Ui;
/** Property and callback plumbing shared by windows, panels and settings pages. */
export declare abstract class ViewBase implements View {
    #private;
    protected readonly scope: ScopeImpl;
    protected readonly handlers: Map<string, (...args: Json[]) => unknown>;
    protected closed: boolean;
    constructor(scope: ScopeImpl);
    protected abstract live(): boolean;
    protected abstract write(property: string, value: Json): Promise<void>;
    /**
     * Initial properties in wire form. A released image is left out (the
     * property keeps its default) and reported, like a failed write.
     */
    protected wired(values: Record<string, PropValue>): Record<string, Json>;
    on<A extends Json[]>(callback: string, handler: (...args: A) => unknown): Disposable;
    set(props: Props): void;
    bind(props: () => Props): Disposable;
    /** Current desired properties (bound values win over earlier `set`). */
    protected snapshot(): Record<string, PropValue>;
    protected evaluate(): void;
    /** The host already shows these values (e.g. sent as initial properties). */
    protected shown(values: Record<string, Json>): void;
    /**
     * Send what changed. A bound value is sent when it differs from the last one
     * sent; a `set()` value is always sent once. Nothing is resent merely
     * because a callback arrived, so text the user typed into an in-out
     * property survives other callbacks (call `set()` to pull a control back).
     * A failed bound value is not repeated until it changes; an explicit `set()`
     * is, after a back-off (0.25 s doubling to 30 s). Only the first failure of
     * a failing streak is reported, so an error handler that writes a status to
     * a broken view cannot spin.
     */
    protected flush(): void;
    protected dispatch(callback: string, args: Json[]): void;
    protected release(): Promise<void>;
}
/** Find `export component X` / `export global Y` so most mounts need no names. */
export declare function exported(directory: string, file: string, kind: string): string;
export declare const windows: (scope: ScopeImpl) => Windows;
export declare const panels: (scope: ScopeImpl) => Panels;
export declare const dialogs: (scope: ScopeImpl) => Dialogs;
