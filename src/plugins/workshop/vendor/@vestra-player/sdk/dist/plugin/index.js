import { Runtime } from "./runtime.js";
import { stopReason } from "./connection.js";
import { failure } from "./errors.js";
import { ScopeImpl } from "./scope.js";
import { windowOf } from "./window.js";
import { timelineOf } from "./timeline.js";
import { triggersOf } from "./triggers.js";
export async function createPlugin(transport, manifest, directory, dataDirectory, 
/** `plugin.app` (the runner passes the player's `VESTRA_APP_INFO`). */
app, 
/** `TestHost` sets `strictDeclarations`; the player's runner does not. */
options) {
    const runtime = new Runtime(transport, manifest, directory, dataDirectory, app, options);
    // The window state is synchronous for plugins: read it first. The
    // player's state and the timeline are the plugin's explicit calls, first
    // in `activate`: `plugin.player.subscribe()` (event delivery, the copy
    // of the player's state follows from then on) and
    // `await plugin.player.read()` (state and timeline read once). Window and
    // timeline subscribe on their first use. The timeline's copy exists from
    // the start: it tells the connection how `read()` reads the lists.
    timelineOf(runtime);
    await windowOf(runtime).init();
    const root = new ScopeImpl(runtime);
    // A runaway of the update passes goes to the plugin's error handlers.
    runtime.passes.report = (error) => root.report(error);
    // A state copy that throws on an event, as a listener's error would be.
    runtime.reportMirror = (error) => root.report(error);
    // Failures the plugin did not ask about: a property the player could not
    // show when it rebuilt a panel or the Options window (an image removed
    // since), a callback whose arguments it could not convert, a detached
    // sidebar that could not be prepared. A warning may come without a code.
    root.add(runtime.raw("ui.error", (data) => {
        const { code, message } = (data ?? {});
        root.report(failure(code ?? "ui", message ?? "user interface error"));
    }));
    const triggers = triggersOf(runtime);
    return {
        plugin: root,
        activate(entry) {
            triggers.begin();
            let result;
            try {
                result = entry(root);
            }
            finally {
                triggers.end();
            }
            return Promise.resolve(result);
        },
        activated: () => runtime.activated(),
        dispose: async (info) => {
            runtime.stopping ??= { reason: stopReason(info?.reason) };
            // Before the cleanup runs: what still waits to be sent rejects as the
            // requests in flight do when the runner stops (`disabled`); the
            // cleanup's own requests go to the transport as before.
            runtime.requests.close();
            await root.dispose();
            runtime.stop();
        },
    };
}
