/**
 * What the player undoes by itself when a plugin's run stops (SDK 1.11.0):
 * the plugin's own cleanup requests are not sent after `runtime.stop`
 * (`RequestQueue.close`), so the test host revokes what the run held, as
 * the player does (its stop revokes the run's windows, panels, commands,
 * settings pages, resolvers, sequence provider, streams and filter
 * chains). By host.
 */
export const revokers = new WeakMap();
/** Revoke what the plugin's run held (`startPlugin`'s `stop`). */
export function revokeRun(host) {
    revokers.get(host)?.();
}
