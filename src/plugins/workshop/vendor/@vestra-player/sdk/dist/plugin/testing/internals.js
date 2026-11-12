/**
 * The private state of a `TestHost` (`./index.ts`): what the class keeps
 * out of its public members. The host holds one in its private `#inner`
 * field and passes it to the area modules' functions, so it is reachable
 * only from inside the SDK.
 */
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Catalog } from "../../i18n.js";
/** The private state of one `TestHost`. */
export class Internals {
    locale;
    /** The plugin's translations for `locale` (see `loadCatalog`). */
    catalog = new Catalog();
    /**
     * Row patches: the `seq` of each property's last write, by view and property
     * (none: 0, the initial properties).
     */
    writes = new Map();
    /** The parts of a whole value held until its last part, by view and property. */
    parts = new Map();
    /** The exported global each mounted panel's properties belong to, by slot. */
    globals = new Map();
    /**
     * A panel's properties (view and name) whose stored initial value the
     * panel could not take: it shows the property's default.
     */
    unshown = new Set();
    nextOwner = 2;
    /** Owners of other plugins' panels, by plugin id ({@link TestHost.mountOther}). */
    others = new Map();
    /** What each job was asked to write, for its outcome. */
    exportJobs = new Map();
    sentTypes = "[]";
    overrides = new Map();
    listeners = new Map();
    inFlight = 0;
    /** `disconnect()` of `startPlugin`: the player is gone. */
    lost = false;
    lose = () => { };
    /** Rejects when the player is lost: requests in flight fail with it. */
    gone = (() => {
        const gone = new Promise((_, reject) => {
            this.lose = reject;
        });
        gone.catch(() => { });
        return gone;
    })();
    next = {
        snapshot: 0,
        job: 0,
        producer: 0,
        source: 0,
        resolution: 0,
        ticket: 0,
    };
    /**
     * `commands.run` tickets: runs, run times in the last second, last use
     * (ms), and the rest of an approved plan (`then`), `null` without one.
     */
    tickets = new Map();
    /** The last `commands.list` table (JSON) and its revision. */
    listed = { text: "", revision: 0 };
    /** The times of the plugin's `commands.run` calls in the last minute. */
    runs = [];
    sourceBook = new Map();
    /** The pending resolutions' stamps and operations (beside `resolutions`). */
    resolving = new Map();
    /** The producer the source is (`openProvider`, a resolved stream). */
    selected = null;
    scratch = mkdtempSync(join(tmpdir(), "vestra-test-host-"));
    /** The labels `commands.keys` last answered or pushed, as JSON. */
    keysSent = "";
    /** The binding table revision of `commands.keys` (a counter here). */
    keysRevision = 0;
    /**
     * Like the player (2026-11-09): a seek from the end plays on unless the
     * end was reached while paused or by a frame step (set when the phase
     * becomes `ended`).
     */
    endKeepsPause = false;
    /** A frame step since the last source intent (the player's `stepped`). */
    stepped = false;
    /** The next source intent is a frame step back from the end: it stays paused. */
    holdPause = false;
    /**
     * The resources this plugin loaded into the current source (resource id →
     * bytes), removed ones included: like the player, a removed track frees
     * its place among the live tracks but not among the loads.
     */
    subtitleResources = { generation: -1, bytes: new Map() };
}
