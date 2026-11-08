/**
 * The view binder (L2, the framework; doc/specs/sdk-refactor.md §2.2 in the
 * player repository): `View.bind` functions, run in the update passes with
 * read tracking (`Runtime.part`), their values handed to the view's writer.
 */
import type { ScopeImpl } from "../scope.ts";
import type { PropValue, Props } from "../types.ts";
import type { ViewWriter } from "./writer.ts";
/** One view's bindings, evaluated into its {@link ViewWriter}. */
export declare class ViewBinder {
    #private;
    constructor(scope: ScopeImpl, writer: ViewWriter, closed: () => boolean);
    bind(props: () => Props): import("../types.ts").Disposable;
    /**
     * Current desired properties (bound values win over earlier `set`). A
     * precise pass (`Runtime.part`) skips a bind function that read none of
     * the changed state: its last values stay.
     */
    snapshot(): Record<string, PropValue>;
    /** One update pass: the bindings' values, then whatever changed is sent. */
    evaluate(): void;
}
