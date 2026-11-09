import type { Json, LayoutPanel } from "../../wire.ts";
import type { FakePanel, TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
import type { Refusal, ViewTypes } from "./slint-types.ts";
export declare const kebab: (name: string) => string;
export declare const SLOT_LABELS: Record<string, string>;
/** The view a panel's writes are kept under (the player's, per slot). */
export declare const mountView: (slot: string) => string;
/** The positions that hold tabs: any number of plugins, one tab each. */
export type TabSlot = "bottom" | "sidebar" | "adjust";
export type WireRowOp = {
    op: "set";
    at: number;
    row: Json;
} | {
    op: "insert";
    at: number;
    rows: Json[];
} | {
    op: "remove";
    at: number;
    count: number;
} | {
    op: "move";
    from: number;
    count: number;
    to: number;
};
/**
 * A patch's ops checked against an array of `length` rows, as the player
 * checks them (`ui_patch.rs`): in order, each on the result of the ones
 * before; at most {@link PATCH_OPS}; the result at most
 * `UI_VALUE_LIMITS.arrayRows` rows.
 */
/** The player takes no Markdown documents in parts of a value (`invalid`). */
export declare function noMarkdownParts(rows: Json[]): void;
export declare function patchOps(ops: unknown, length: number): WireRowOp[];
export declare function timelineChanged(host: TestHost, _inner: Internals): void;
export declare function resetWrites(_host: TestHost, inner: Internals, view: string): void;
/**
 * A property write as the player takes it, returning the value the
 * property shows after it: a whole `value`, or a `patch` of the array
 * `current` applying on the write numbered `base` (code `stale`
 * otherwise). The result is checked against the value limits whole: the
 * player counts the rows a patch keeps too.
 */
export declare function writeProperty(host: TestHost, inner: Internals, view: string, property: string, p: Record<string, Json>, current: Json | undefined, typed?: TypedView | null): Json | undefined;
/**
 * Refuse property values over the player's limits ({@link UI_VALUE_LIMITS})
 * with `limit`, naming the property, as the player does.
 */
export declare function checkValues(_host: TestHost, _inner: Internals, values: Record<string, Json>): void;
/**
 * A view's declared property types, with how the player words a refusal
 * there: the error `code`, and `bare` when it reads the property before it
 * writes it (`ui.set`, `ui.setMount`, `settings.set`, a panel's or settings
 * page's initial values), so that an unknown one is refused as "no such
 * property" without its name. Those writes compare first
 * (`ui_values::reconcile_in`): a value equal to what the property holds
 * does nothing, on an `out` property too. `fresh`: the property shows its
 * default although a value is stored (a panel's initial value it could
 * not take).
 */
export interface TypedView {
    types: ViewTypes;
    code: string;
    bare: boolean;
    fresh?: boolean;
}
/**
 * The declared types of the view in `file` of the plugin's package: its
 * exported component, or `global` (a panel's or settings page's). `null`
 * (nothing checked) when the host runs no package or the file cannot be
 * read.
 */
export declare function typedView(host: TestHost, file: string, global: string | undefined, code: string, bare: boolean): TypedView | null;
/** A refusal in the player's words, for a write that reads the property first (`bare`) or not. */
export declare const refusalText: (found: Refusal, bare: boolean) => string;
/**
 * Refuse a value the player's Slint refuses for `property` (a value of
 * another type than declared, an undeclared or `out` property), with the
 * player's code and words. `current`: what the property shows now (none:
 * its default), which the words describe the declared type by.
 */
export declare function checkType(typed: TypedView, property: string, value: Json, current: Json | undefined): void;
/**
 * Refuse a struct field name the player's conversion refuses (not in
 * Slint's canonical spelling), whatever the view declares: code `host`,
 * the property named.
 */
export declare function checkSpelling(property: string, value: Json): void;
/** Property names in the order the player sets them (a `BTreeMap`). */
export declare const inSetOrder: (values: Record<string, Json>) => string[];
/**
 * A panel's or settings page's initial properties, as the player keeps
 * them: one over the limits (`limit`) or with a struct field name not in
 * Slint's canonical spelling (`host`) is left out (the property keeps its
 * default) and reported with `ui.error`, the request itself succeeds.
 */
export declare function initialValues(host: TestHost, _inner: Internals, values: Record<string, Json>): Record<string, Json>;
export declare function seeImages(host: TestHost, _inner: Internals, values: Record<string, Json> | Json[]): void;
export declare function addPanel(host: TestHost, _inner: Internals, slot: TabSlot, panel: LayoutPanel): void;
export declare function removePanel(host: TestHost, _inner: Internals, slot: TabSlot, owner: number): void;
/** The panel a request is about: its `slot`, else the only one. */
export declare function panelFor(host: TestHost, _inner: Internals, p: Record<string, unknown>): FakePanel;
/** Implements `TestHost.mountOther`. */
export declare function mountOther(host: TestHost, inner: Internals, slot: "bottom" | "sidebar" | "adjust" | "backdrop", plugin: string, options?: {
    title?: string;
}): void;
/** Implements `TestHost.unmountOther`. */
export declare function unmountOther(host: TestHost, inner: Internals, plugin: string): void;
/** Implements `TestHost.mounted`. */
export declare function mounted(host: TestHost, _inner: Internals, slot?: "bottom" | "sidebar" | "adjust" | "backdrop"): FakePanel;
/** Implements `TestHost.setTimelineRanges`. */
export declare function setTimelineRanges(host: TestHost, inner: Internals, pluginId: string, ranges: {
    start: number;
    end: number;
    style: string;
    title?: string | null;
}[]): void;
/** Implements `TestHost.setTimelineLayer`. */
export declare function setTimelineLayer(host: TestHost, inner: Internals, pluginId: string, marks: {
    time: number;
    kind: string;
    title?: string | null;
}[]): void;
/** The default handlers of this area's wire methods, by method name. */
export declare function layoutHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
