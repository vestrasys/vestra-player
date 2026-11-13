import type TS from "typescript";
/** @internal The scan's categories, in the order results list them. */
export declare const SCAN_CATEGORIES: readonly ["player", "plugins", "clipboard", "credentials", "camera", "microphone", "screen", "notifications"];
/** @internal One of {@link SCAN_CATEGORIES}. */
export type ScanCategory = (typeof SCAN_CATEGORIES)[number];
/** @internal The version of the result's format (`scan`). */
export declare const SCAN_FORMAT = 1;
/** @internal One list of `capability-scan.json`. */
export interface ScanList {
    readonly about: string;
    /** API members, `Interface.member` (`Clipboard.readText`, `Scope.video.filter`). */
    readonly members: readonly string[];
    /** Methods of the player's protocol, as its capability table names them (`packages.*`). */
    readonly requests: readonly string[];
}
/** @internal `capability-scan.json`. */
export interface ScanTable {
    readonly about: string;
    readonly version: number;
    readonly categories: Readonly<Record<ScanCategory, ScanList>>;
    readonly none: ScanList;
}
/**
 * `capability-scan.json`, read once from the package's root (one level
 * above this module, in `src/` and in `dist/` alike).
 *
 * @internal
 */
export declare function scanTable(): ScanTable;
/** @internal Where the scan found a use, for people who check it. */
export interface ScanEvidence {
    /** The API member used (`Clipboard.readText`); absent for a request by name. */
    readonly member?: string;
    /** The protocol method requested by name (`clipboard.readText`); absent for a member. */
    readonly request?: string;
    /** The file, relative to the plugin's directory, with `/`. */
    readonly file: string;
    /** 1-based line. */
    readonly line: number;
    /** `"types"`: found by type; `"names"`: found by name (best effort). */
    readonly by: "types" | "names";
}
/** @internal A request by name the scan could not put in a category. */
export interface ScanUnchecked {
    readonly file: string;
    readonly line: number;
    /** One line for people. */
    readonly message: string;
}
/** @internal What a scan found, before `vestra-plugin scan` adds the plugin's identity. */
export interface ScanFindings {
    /** The categories used, in {@link SCAN_CATEGORIES}' order. */
    readonly uses: ScanCategory[];
    /** Per used category, where: by file, line, then name. */
    readonly evidence: Partial<Record<ScanCategory, ScanEvidence[]>>;
    readonly unchecked: ScanUnchecked[];
}
/**
 * The category of a request by name, as the player routes a method: the
 * exact name first, then the first `prefix.*` entry it starts with.
 * `null`: in `none`; `undefined`: the table does not know the method.
 *
 * @internal
 */
export declare function requestCategory(method: string, scan?: ScanTable): ScanCategory | null | undefined;
/** @internal What {@link scan} reads. */
export interface ScanOptions {
    /** The plugin's TypeScript files, in `program`: scanned by type. */
    readonly typed: readonly string[];
    /** The plugin's JavaScript files, with their text: scanned by name. */
    readonly named: readonly {
        readonly file: string;
        readonly text: string;
    }[];
    /** How the result names a file (relative to the plugin, with `/`). */
    readonly relative?: (file: string) => string;
    /** Default: `capability-scan.json`. */
    readonly table?: ScanTable;
}
/**
 * The categories a plugin's files use.
 *
 * `resolved` is `false` when there are TypeScript files and the program
 * resolves no import of `@vestra-player/sdk` in them (the SDK is not
 * installed where the plugin looks, or nothing imports it): the caller
 * then scans those files by name too, or reports it.
 *
 * @param program - A program that has the `typed` files; `undefined` with
 *   none.
 *
 * @internal
 */
export declare function scan(ts: typeof TS, program: TS.Program | undefined, options: ScanOptions): ScanFindings & {
    resolved: boolean;
};
