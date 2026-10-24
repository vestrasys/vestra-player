import { Runtime } from "./runtime.js";
import { ScopeImpl } from "./scope.js";
import { playerOf } from "./player.js";
import { windowOf } from "./window.js";
import { timelineOf } from "./timeline.js";
export async function createPlugin(transport, manifest, directory, dataDirectory, 
/** `plugin.app` (the runner passes the player's `VESTRA_APP_INFO`). */
app) {
    const runtime = new Runtime(transport, manifest, directory, dataDirectory, app);
    // Player, window and timeline state are synchronous for plugins: read them first.
    await Promise.all([
        playerOf(runtime).init(),
        windowOf(runtime).init(),
        timelineOf(runtime).init(),
    ]);
    const root = new ScopeImpl(runtime);
    return {
        plugin: root,
        activated: () => runtime.activated(),
        dispose: () => root.dispose(),
    };
}
