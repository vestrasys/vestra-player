import type { ScopeImpl } from "./scope.ts";
import type { MediaCapture } from "./types.ts";
export declare function mediaCapture(scope: ScopeImpl): MediaCapture;
/**
 * The player's bounds of remembered files and exports of their parts,
 * documented in the limits reference. Exported from `./runtime` only, for
 * the player's agreement test; TestHost and the SDK's own checks read
 * these figures. The bound on references is a resource limit; tokens are
 * an API convenience, not a security boundary.
 *
 * @internal
 */
export declare const MEDIA_SOURCE_LIMITS: Readonly<{
    /** References one plugin may hold; one more fails with `limit` (500). */
    readonly references: 500;
    /** Parts of one export or check, at least 1; more fail with `invalid` (100). */
    readonly parts: 100;
    /** Seconds of a part's end in its file (24 hours). */
    readonly partEnd: 86400;
    /** Seconds of all parts of one export together, as one clip (24 hours). */
    readonly totalSeconds: 86400;
    /**
     * `remember`, `get`, `list` and checks one plugin may have the
     * player working on at the same time; one more fails with `busy` (4).
     */
    readonly requests: 4;
    /** Seconds between two `locate` dialogs of one plugin; sooner fails with `busy` (2). */
    readonly locateInterval: 2;
}>;
