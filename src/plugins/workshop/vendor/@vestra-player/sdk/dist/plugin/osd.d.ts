import type { ScopeImpl } from "./scope.ts";
import type { Osd } from "./types.ts";
import type { TemplateArgsJson, TextValueJson } from "../wire.ts";
/**
 * The player's limits on the on-screen display ({@link Osd}), counted on
 * the text as shown: after the player translated it and filled in its
 * arguments. A text over them fails with code `invalid`: `osd.show`
 * rejects, `OsdPanel.set` reports to `Scope.onError`. The player's tests
 * check that these agree with the player.
 *
 * @example
 * ```ts
 * import { OSD_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const lines = rows.slice(0, OSD_LIMITS.panelLines);
 * panel.set(lines.join("\n"), null, { maxColumns: 60 });
 * ```
 *
 * @since 1.5.14
 */
export declare const OSD_LIMITS: Readonly<{
    /**
     * Characters of one `osd.show` message (200; at least 1 that is not
     * white space). Line breaks are shown as spaces.
     */
    readonly messageChars: 200;
    /** Lines of the information block (`OsdPanel.set`) (32). */
    readonly panelLines: 32;
    /** UTF-8 bytes of the information block (4096). */
    readonly panelBytes: 4096;
    /** The smallest `OsdPanelOptions.maxColumns` (8). */
    readonly minColumns: 8;
    /** The largest `OsdPanelOptions.maxColumns` (1000). */
    readonly maxColumns: 1000;
}>;
/**
 * Copy and check template arguments: an array or object of at most 16
 * strings, finite numbers, texts and lists (`TEXT_LIMITS`).
 */
export declare function templateArgs(args: unknown): TemplateArgsJson | undefined;
/**
 * Check a shown text (`osd.show`, dialogs): source text, or a text value
 * that carries its own arguments (then `args` is not given).
 */
export declare function shownText(text: unknown, args: unknown, what?: string): TextValueJson;
export declare const osd: (scope: ScopeImpl) => Osd;
