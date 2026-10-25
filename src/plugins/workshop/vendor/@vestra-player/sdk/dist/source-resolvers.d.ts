import type { Transport } from "./wire.ts";
/** One stream of a {@link ResolvedMedia}. */
export type ResolvedResource = {
    /** The playable stream URL. */
    url: string;
    /** HTTP headers for requests to `url` (e.g. a referer or cookie). */
    headers?: Record<string, string>;
};
/**
 * What a resolver returns ({@link WebResolverOptions.resolve}): the playable
 * stream(s) behind a page URL. Both streams must use identical `headers`;
 * different maps fail explicitly (nothing is dropped silently).
 *
 * @example
 * ```ts
 * const media: ResolvedMedia = {
 *   kind: "media",
 *   originalUrl: url,
 *   title: "Talk",
 *   primary: { url: videoUrl, headers: { Referer: url } },
 *   audio: { url: audioUrl, headers: { Referer: url } },
 * };
 * ```
 */
export type ResolvedMedia = {
    /** Always `"media"`. */
    kind: "media";
    /** The page URL that was resolved. */
    originalUrl: string;
    /** A title for the player to show. */
    title?: string;
    /** The video (or combined) stream. */
    primary: ResolvedResource;
    /** A separate audio stream, when audio and video are split. */
    audio?: ResolvedResource;
};
/**
 * A resolver declaration on the wire: a web resolver (`schemes` + `hosts`)
 * or, from player 0.4.1, a file resolver (`extensions`), never both.
 */
export type SourceResolverDeclaration = {
    id: string;
    /** Finite initial contract: http and/or https, exact normalized host names. */
    schemes: ("http" | "https")[];
    hosts: string[];
} | {
    id: string;
    /** Local files by extension: 1–16, lower-case letters or digits, no dot. */
    extensions: string[];
};
/**
 * A file resolver's answer as the handler gives it: the producer it prepared
 * (sent to the host), `accepted` once the host selected it (with the stamp of
 * the source it became) and `abandoned` when it never will be: the request
 * was cancelled or answered too late. Exactly one of the two is called.
 */
export type ResolvedProducer = {
    kind: "stream";
    producerId: number;
    accepted(reply: {
        stamp: {
            generation: number;
            intentVersion: number;
        };
    }): Promise<void>;
    abandoned(): Promise<void>;
};
export declare function sourceResolverClient(transport: Transport): {
    /** Requires source.resolve. One registered host/scheme pair, and one file
     * extension, has one owner (another plugin: `conflict`, reason
     * `resolver-taken`). Resolution shares source.submit arbitration and the
     * original operation ID. This backend supports paired streams with
     * identical header maps; different maps fail explicitly. No header is
     * silently discarded or made global. A failure is sent as its message. */
    register(declaration: SourceResolverDeclaration, handler: (request: {
        url: string;
        signal: AbortSignal;
    }) => Promise<ResolvedMedia | ResolvedProducer>): Promise<{
        dispose(): Promise<void>;
    }>;
};
