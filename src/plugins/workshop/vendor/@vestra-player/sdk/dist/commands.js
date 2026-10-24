/**
 * The player's command registry caps, the same values as the player's own
 * table (the player's tests check that they agree). They are runaway guards,
 * not product limits: a command added past them fails with code `limit`
 * (its `ready` rejects; during activation, activation fails), commands
 * already registered are unaffected, and disposing a command frees its slot
 * at once. {@link TestHost} enforces the same caps.
 *
 * For menus built from media content (tracks, chapters, presets), set your
 * own sensible display cap well below `perPlugin`.
 *
 * @example
 * ```ts
 * import { COMMAND_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const shown = chapters.slice(0, Math.min(50, COMMAND_LIMITS.perPlugin - 20));
 * ```
 */
export const COMMAND_LIMITS = {
    /**
     * Commands one plugin instance may have at the same time (160).
     */
    perPlugin: 160,
    /** Commands of every plugin plus the player's own, together (1024). */
    total: 1024,
};
/**
 * The player's built-in top-level menus, by id. Write `@<id>` as the first
 * segment of a command path (`["@video", "Blur"]`, see `CommandOptions.path`) to put a
 * command into that menu whatever language the player's menus are in. The
 * player stores the menu's title (视频 today); later segments are titles.
 * Needs player 0.3.0 (`"minPlayerVersion": "0.3.0"`); older players show
 * `@video` as a menu of its own. The player's tests check that this list
 * agrees with the player's.
 *
 * @example
 * ```ts
 * import { MENU_IDS } from "@vestra-player/sdk/testing";
 *
 * assert.ok(MENU_IDS.includes("video"));
 * ```
 */
export const MENU_IDS = [
    "open",
    "playback",
    "subtitles",
    "video",
    "audio",
    "screen",
    "settings",
];
export function commandClient(transport) {
    return {
        register: (id, revision, declaration, state) => transport.request("commands.register", {
            id,
            revision,
            declaration,
            state,
        }),
        /** Remove the command from the menu and the binding table; the id is free again. */
        unregister: (id) => transport.request("commands.unregister", { id }),
        update: (id, revision, state) => transport.request("commands.update", {
            id,
            revision,
            state,
        }),
    };
}
