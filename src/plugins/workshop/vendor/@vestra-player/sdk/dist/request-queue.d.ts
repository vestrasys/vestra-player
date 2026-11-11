import type { Json, Transport } from "./wire.ts";
/**
 * How much of the plugin's traffic the SDK keeps in flight, documented in
 * the limits reference. Exported from `./runtime` only, for the player's
 * agreement test: a plugin never meets these figures as an error, only as
 * a delay.
 *
 * @internal
 */
export declare const REQUEST_LIMITS: Readonly<{
    /**
     * Requests one plugin has sent the player and not yet had answered; the
     * next waits in the plugin's queue until an answer comes, under the
     * player's backstop of 320 lines (256).
     */
    readonly inFlight: 256;
    /**
     * UTF-8 bytes of those requests' lines together; the next waits unless
     * nothing is in flight. One `MESSAGE_LIMIT` line more stays under the
     * player's backstop of 4 MiB, for the replies that never wait (3 MiB).
     */
    readonly inFlightBytes: number;
}>;
/**
 * Requests that answer the player's own calls (a resolution, a producer's
 * control): sent at once, never behind the queue. The player may hold one
 * of the plugin's requests until such an answer comes (an open waits for
 * the plugin's resolver), so waiting behind it could never end.
 * `palette.lead.done` is not one: the player waits for nothing on it, and
 * unqueued it could pass a waiting `commands.run` of the same ticket. They count while in flight; their number is the player's, so
 * the backstop's margin over `inFlight` covers them.
 */
export declare const UNQUEUED_METHODS: ReadonlySet<string>;
/** A request's options on the plugin's side; nothing of them is sent. */
export interface RequestOptions {
    /**
     * Aborting it while the request still waits takes the request out of the
     * queue, unsent, and rejects it with `aborted` (`AbortError`). Once sent,
     * it runs to its answer, as before.
     */
    signal?: AbortSignal;
}
/** The transport behind a per-plugin limit of requests in flight. */
export declare class RequestQueue {
    #private;
    /** The most in flight at once (the SDK's own tests). */
    peak: number;
    constructor(transport: Transport, limits?: {
        inFlight: number;
        inFlightBytes: number;
    });
    /** Requests sent and not yet answered, and their bytes. */
    get inFlight(): {
        requests: number;
        bytes: number;
    };
    /** Requests waiting to be sent. */
    get waiting(): number;
    request<T>(method: string, params?: Json, options?: RequestOptions): Promise<T>;
    /**
     * The plugin stops: every waiting request rejects with `error` (the one
     * the runner gives the requests in flight), and later ones go to the
     * transport as they come, which refuses them while stopping.
     */
    close(error?: Error): void;
}
