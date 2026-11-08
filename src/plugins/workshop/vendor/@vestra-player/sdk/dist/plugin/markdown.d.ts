import type { Json } from "../wire.ts";
import type { Connection } from "./connection.ts";
import type { ScopeImpl } from "./scope.ts";
import type { MarkdownOptions, MarkdownRef } from "./types.ts";
export declare const MARKDOWN_LIMITS: Readonly<{
    /** Text bytes (UTF-8). */
    textBytes: number;
    /** Bytes of one downloaded image. */
    imageBytes: number;
    /** Download time per image. */
    imageTimeoutMs: 10000;
    /** Redirects followed per image. */
    redirects: 5;
    /** Downloads at once, per plugin. */
    concurrency: 4;
    /** Cache size per plugin. */
    cacheBytes: number;
    /** An address is fetched again after this long. */
    cacheMaxAgeMs: number;
}>;
/** The wire form (`{ $vestraMarkdown: { text, base, images }, ref }`) is internal. */
export declare class MarkdownHandle implements MarkdownRef {
    #private;
    readonly text: string;
    readonly __vestraMarkdown: true;
    constructor(scope: ScopeImpl, text: string, base: string | null, images: Record<string, string>);
    /** Released with its scope: writing it is `invalid`. */
    get released(): boolean;
    toJSON(): Json;
}
/** Sent with every image download (some hosts refuse anonymous clients). */
export declare const USER_AGENT = "VestraPlayer-Markdown/1.1 (+https://github.com/vestrasys/vestra-player-sdk)";
/**
 * `https:` only, redirects followed by hand (≤ 5, all `https:`), size capped
 * while reading; an HTTP error status throws. `fetchImpl` is for tests.
 */
export declare function httpsDownload(url: string, options: {
    maxBytes: number;
    signal: AbortSignal;
}, fetchImpl?: typeof fetch): Promise<Uint8Array>;
/**
 * `ui.remoteImage`: the local copy of an `https:` image in
 * `<dataDirectory>/.vestra-cache/images/` (the Markdown limits: 4 MB, 10 s,
 * 4 at a time per plugin, 32 MB of cache, a day). Rejects `unavailable`.
 */
export declare function remoteImageFile(runtime: Connection, url: string): Promise<string>;
export declare function markdown(scope: ScopeImpl, text: string, options?: MarkdownOptions): Promise<MarkdownRef>;
