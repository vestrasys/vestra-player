import type { Json } from "../../wire.ts";
import type { SourceStamp } from "../../source.ts";
import type { Locale } from "../types.ts";
import { Catalog } from "../../i18n.ts";
import type { TestHost } from "./index.ts";
/** A wire method's default or overriding handler (as `TestHost.handle` takes it). */
export type Handler = Parameters<TestHost["handle"]>[1];
/** The private state of one `TestHost`. */
export declare class Internals {
    locale: Locale | undefined;
    /** The plugin's translations for `locale` (see `loadCatalog`). */
    catalog: Catalog;
    /**
     * Row patches (player 0.6.0): the `seq` of each property's last write, by
     * view and property (none: 0, the initial properties).
     */
    readonly writes: Map<string, number>;
    /** The parts of a whole value held until its last part, by view and property. */
    readonly parts: Map<string, {
        seq: number;
        rows: Json[];
    }>;
    nextOwner: number;
    /** Owners of other plugins' panels, by plugin id ({@link TestHost.mountOther}). */
    readonly others: Map<string, {
        slot: string;
        owner: number;
    }>;
    /** What each job was asked to write, for its outcome. */
    readonly exportJobs: Map<number, {
        destination: string;
        clip: {
            start: number;
            end: number;
            container: "mp4" | "mkv" | null;
        } | null;
    }>;
    sentTypes: string;
    readonly overrides: Map<string, (params: Record<string, Json>) => unknown>;
    readonly listeners: Map<string, Set<(data: unknown) => void>>;
    inFlight: number;
    /** `disconnect()` of `startPlugin`: the player is gone. */
    lost: boolean;
    lose: (error: Error) => void;
    /** Rejects when the player is lost: requests in flight fail with it. */
    readonly gone: Promise<never>;
    next: {
        snapshot: number;
        job: number;
        producer: number;
        source: number;
        resolution: number;
    };
    sourceBook: Map<number, Json>;
    /** The pending resolutions' stamps and operations (beside `resolutions`). */
    resolving: Map<number, {
        stamp: SourceStamp;
        operation?: number;
    }>;
    /** The producer the source is (`openProvider`, a resolved stream). */
    selected: number | null;
    readonly scratch: string;
    /** The labels `commands.keys` last answered or pushed, as JSON. */
    keysSent: string;
    /** The binding table revision of `commands.keys` (a counter here). */
    keysRevision: number;
    /**
     * The resources this plugin loaded into the current source (resource id →
     * bytes), removed ones included: like the player, a removed track frees
     * its place among the live tracks but not among the loads.
     */
    subtitleResources: {
        generation: number;
        bytes: Map<string, number>;
    };
}
