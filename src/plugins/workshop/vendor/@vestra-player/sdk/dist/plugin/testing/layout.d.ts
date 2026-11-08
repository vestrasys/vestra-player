import type { Json, LayoutPanel } from "../../wire.ts";
import type { FakePanel, TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
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
export declare function writeProperty(host: TestHost, inner: Internals, view: string, property: string, p: Record<string, Json>, current: Json | undefined): Json | undefined;
/**
 * Refuse property values over the player's limits ({@link UI_VALUE_LIMITS})
 * with `limit`, naming the property, as the player does.
 */
export declare function checkValues(_host: TestHost, _inner: Internals, values: Record<string, Json>): void;
/**
 * A panel's or settings page's initial properties, as the player keeps
 * them: one over the limits is left out (the property keeps its default)
 * and reported with `ui.error`, the request itself succeeds.
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
