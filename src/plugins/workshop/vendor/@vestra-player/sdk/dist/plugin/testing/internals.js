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
    next = { snapshot: 0, job: 0, producer: 0, source: 0, resolution: 0 };
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
     * The resources this plugin loaded into the current source (resource id →
     * bytes), removed ones included: like the player, a removed track frees
     * its place among the live tracks but not among the loads.
     */
    subtitleResources = { generation: -1, bytes: new Map() };
}
