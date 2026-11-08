/**
 * The single-flight write queue (L1, refactor plan §2.2): a view's
 * property writes, a command's state, an OSD panel. It knows nothing of
 * update passes (it lived in `runtime.ts`, beside the pass scheduler).
 */
/** One value in flight, only the newest waiting. Each push learns its fate. */
export declare class Latest<T> {
    #private;
    constructor(send: (value: T) => Promise<void>);
    push(value: T): Promise<boolean>;
    stop(): void;
}
