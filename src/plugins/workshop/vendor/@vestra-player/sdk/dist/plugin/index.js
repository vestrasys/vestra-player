import { failure, Runtime } from "./runtime.js";
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
    // Failures the plugin did not ask about: a property the player could not
    // show when it rebuilt a panel or the Options window (an image removed
    // since), a callback whose arguments it could not convert, a detached
    // sidebar that could not be prepared. Older players send no code.
    root.add(runtime.raw("ui.error", (data) => {
        const { code, message } = (data ?? {});
        root.report(failure(code ?? "ui", message ?? "user interface error"));
    }));
    return {
        plugin: root,
        activated: () => runtime.activated(),
        dispose: async () => {
            await root.dispose();
            runtime.stop();
        },
    };
}
