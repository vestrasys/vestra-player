import type { Json, PropertyWrite } from "../../wire.ts";
import type { ScopeImpl } from "../scope.ts";
import type { Disposable, PropValue, Props, View } from "../types.ts";
/** Property and callback plumbing shared by windows, panels and settings pages. */
export declare abstract class ViewBase implements View {
    #private;
    protected readonly scope: ScopeImpl;
    protected readonly handlers: Map<string, (...args: Json[]) => unknown>;
    protected closed: boolean;
    constructor(scope: ScopeImpl);
    protected abstract live(): boolean;
    protected abstract write(property: string, write: PropertyWrite): Promise<void>;
    /** {@link ViewWriter.wired}: initial properties in wire form. */
    protected wired(values: Record<string, PropValue>, refuse?: boolean): Record<string, Json>;
    on<A extends Json[]>(callback: string, handler: (...args: A) => unknown): Disposable;
    set(props: Props): void;
    bind(props: () => Props): Disposable;
    /** {@link ViewBinder.snapshot}: current desired properties. */
    protected snapshot(): Record<string, PropValue>;
    protected evaluate(): void;
    /** The host already shows these values (e.g. sent as initial properties). */
    protected shown(values: Record<string, Json>): void;
    /** {@link ViewWriter.flush}: send what changed. */
    protected flush(): void;
    protected dispatch(callback: string, args: Json[]): void;
    protected release(): Promise<void>;
}
/** Find `export component X` / `export global Y` so most mounts need no names. */
export declare function exported(directory: string, file: string, kind: string): string;
