/** Finite candidate command registry. Host authenticates owner; no plugin supplies an owner id. */
import type { Json, Transport } from "./wire.ts";
import type { ListJson, TextJson } from "./i18n.ts";
import type { ConditionJson } from "./plugin/when.ts";
import type { CommandList, CommandRunResult } from "./plugin/types.ts";
/**
 * The player's command registry caps, the same values as the player's own
 * table (the player's tests check that they agree). They are runaway guards,
 * not product limits: a command added past them is refused by the player
 * with code `limit` (its `ready` rejects; during activation, activation
 * fails; later the failure goes to `Scope.onError`), commands
 * already registered are unaffected, and disposing a command frees its slot
 * at once. {@link TestHost} enforces the same caps.
 *
 * The other fields are the player's rules for one declaration (since
 * 1.5.48): a command past them is refused with `invalid`, the same way.
 *
 * For menus built from media content (tracks, chapters, presets), set your
 * own sensible display cap well below `perPlugin`.
 *
 * @example Cap a chapter menu well below the command limit
 * The Chapters submenu lists at most 50 chapters, however many the file
 * has. Every menu item is a command, and the player counts them per plugin.
 * `add` itself returns at once and never throws `limit`: past `perPlugin`
 * the player refuses the registration, which fails the activation or, later,
 * reaches `Scope.onError`. A try/catch around `add` cannot catch that, so a
 * menu built from media content keeps its own cap and leaves room for the
 * plugin's fixed commands.
 * ```ts
 * declare const chapters: { title: string; time: number }[];
 * // ---cut---
 * import { COMMAND_LIMITS } from "@vestra-player/sdk/testing";
 *
 * // 20 slots stay free for the plugin's own commands.
 * const shown = chapters.slice(0, Math.min(50, COMMAND_LIMITS.perPlugin - 20));
 * ```
 *
 * @since 1.0.0
 * @category Testing
 */
export declare const COMMAND_LIMITS: {
    /**
     * Commands one plugin instance may have at the same time (160).
     */
    readonly perPlugin: 160;
    /** Commands of every plugin plus the player's own, together (1024). */
    readonly total: 1024;
    /**
     * Segments of a menu path, the leaf included.
     *
     * @since 1.5.48
     */
    readonly depth: 8;
    /**
     * UTF-8 bytes of a command's local id: ASCII letters, digits, `.`,
     * `_` and `-`.
     *
     * @since 1.5.48
     */
    readonly idBytes: 128;
    /**
     * UTF-8 bytes of a title, a text value counted by its English
     * rendering. Like every command text: not empty, no control characters,
     * no whitespace at either end.
     *
     * @since 1.5.48
     */
    readonly titleBytes: 128;
    /**
     * UTF-8 bytes of a menu path segment, a text value counted by its
     * English rendering.
     *
     * @since 1.5.48
     */
    readonly segmentBytes: 80;
    /**
     * UTF-8 bytes of a group.
     *
     * @since 1.5.48
     */
    readonly groupBytes: 80;
    /**
     * UTF-8 bytes of a category (`CommandOptions.category`), a text value
     * counted by its English rendering.
     *
     * @since 1.6.4
     */
    readonly categoryBytes: 80;
    /**
     * UTF-8 bytes of a state's reason, as shown in the player's
     * language.
     *
     * @since 1.5.48
     */
    readonly reasonBytes: 512;
    /**
     * The largest `order` either way: an order is within ±1,000,000.
     *
     * @since 1.5.48
     */
    readonly order: 1000000;
};
/**
 * The player's built-in top-level menus, by id. Write `@<id>` as the first
 * segment of a command path (`["@video", "Blur"]`, see `CommandOptions.path`)
 * to put a command into that menu whatever language the player's menus are in.
 * Later segments are titles.
 * `reset`: the Reset menu, directly below Settings, gathers every command that
 * puts something back to its default. `tools` (SDK 1.7.12): the Tools menu,
 * after the domain menus and before Options…, the home of the player's tools
 * (tracks, snapshot, Vestra Player AI, the store, the command palette) and of
 * any plugin's own. The player's tests check that this list agrees with the
 * player's.
 *
 * @example Put a command into the built-in Video menu
 * "Blur" appears in the player's Video menu in every interface language.
 * The player replaces a leading `@video` with that menu's key ("Video")
 * before it merges paths, so the item joins the built-in menu; an id that
 * is not in `MENU_IDS` makes `add` fail with `invalid`, which a test can
 * rule out ahead of time.
 * ```ts
 * import type { Plugin } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * // ---cut---
 * import assert from "node:assert/strict";
 * import { MENU_IDS } from "@vestra-player/sdk/testing";
 *
 * plugin.commands.add({ id: "blur", path: ["@video", "Blur"], run() {} });
 * assert.ok(MENU_IDS.includes("video")); // a menu id, not a title
 * ```
 *
 * @since 1.3.0
 * @category Testing
 */
export declare const MENU_IDS: readonly ["open", "playback", "subtitles", "video", "audio", "screen", "settings", "reset", "tools"];
/**
 * The icons a bottom-bar button may name (`CommandOptions.button.icon`): every
 * icon of the player's V-Line set, `@vestra/icons.slint` (draw one in your own
 * Slint with `Icon { name: "store"; }`). The player's tests check that this
 * list is its icon set. `"sliders"` (three sliders, the player's Adjustments
 * and Effects button); `"search-ai"` (a magnifier with the AI icon's
 * sparkle, the palette's button while a plugin holds its lead row).
 *
 * @example Check in a test that the button's icon exists
 * The test fails before release if the plugin's bar button names an icon
 * the player does not have. `commands.add` would refuse an unknown name
 * with `invalid`; the player's tests keep `BAR_BUTTON_ICONS` equal to its
 * icon set, so checking against it is the same check without a player.
 * ```ts
 * import assert from "node:assert/strict";
 * import { BAR_BUTTON_ICONS } from "@vestra-player/sdk/testing";
 *
 * assert.ok(BAR_BUTTON_ICONS.includes("store")); // the icon main.ts uses
 * ```
 *
 * @since 1.5.29
 *
 * @category Testing
 */
export declare const BAR_BUTTON_ICONS: readonly ["play", "pause", "stop", "back", "forward", "open", "volume", "mute", "playlist", "detach", "attach", "close", "failed", "move-top", "move-up", "move-down", "move-bottom", "search", "chapter", "bookmark", "check-task", "check-box", "menu-check", "submenu", "scroll-up", "scroll-down", "caret-down", "sort-asc", "sort-desc", "tag-remove", "preview-play", "add", "remove", "sort", "fullscreen", "exit-fullscreen", "store", "ai", "sliders", "search-ai"];
/**
 * The player's limits on bottom-bar buttons (`CommandOptions.button`), the same
 * values as the player's (its tests check that they agree). A button past
 * `perPlugin` has none (its command still registers and its menu entry stays).
 * `total` is what the strip holds: the `total` lowest by order, plugin id and
 * declaration order; the rest are in the strip's » menu, in the same order.
 *
 * @since 1.5.29
 *
 * @category Testing
 */
export declare const BAR_BUTTON_LIMITS: {
    /** Buttons one plugin shows (1). */
    readonly perPlugin: 1;
    /** Buttons in the strip, every plugin together, besides the player's own (8); the rest are in its » menu. */
    readonly total: 8;
    /** The highest `order` (999; the lowest is 0). */
    readonly maxOrder: 999;
};
/**
 * The player's limits on `commands.run` (built-in plugins only), the same
 * values as the player's (its agreement test checks that they agree).
 * Internal: exported from `@vestra-player/sdk/runtime` only.
 *
 * @internal
 */
export declare const COMMAND_RUN_LIMITS: {
    /** Commands of one plan: the run and the ids of its `then` (8). */
    readonly plan: 8;
    /** Runs of one ticket (30). */
    readonly perTicket: 30;
    /** Runs a second, per ticket (4). */
    readonly perSecond: 4;
    /** Runs a minute, per plugin (120). */
    readonly perMinute: 120;
    /** A ticket ends this long after it was given or last ran a command (ms). */
    readonly ticketIdleMs: 120000;
};
/**
 * The player's limits on the texts of `palette.lead.set`, in characters as
 * shown (translated, a text value with its arguments), each on one line;
 * the same values as the player's (its agreement test checks that they
 * agree). The player checks them; `TestHost` does the same.
 * Internal: exported from `@vestra-player/sdk/runtime` only.
 *
 * @internal
 */
export declare const PALETTE_LEAD_LIMITS: {
    /** `title` (40). */
    readonly title: 40;
    /** `placeholder` (60). */
    readonly placeholder: 60;
    /** `empty` (80). */
    readonly empty: 80;
    /** `note` (40). */
    readonly note: 40;
    /** `status.text` of a working status: one line of the row and the strip (80). */
    readonly statusText: 80;
    /** `status.text` of an end state: what the strip's two lines hold (160). */
    readonly statusEndText: 160;
    /** `status.note` (40). */
    readonly statusNote: 40;
    /** `status.strip`: one line of the strip (80). */
    readonly statusStrip: 80;
};
/**
 * `palette.lead.set`'s `status.state` values, the first the default
 * (absent or `null`); the others are end states. The same list as the
 * player's. Internal: exported from `@vestra-player/sdk/runtime` only.
 *
 * @internal
 */
export declare const PALETTE_LEAD_STATES: readonly ["working", "done", "failed", "note", "stopped"];
/**
 * What is wrong with `commands.run`'s `then` (given, not `null`), as
 * the player words it; `null` when nothing is. The SDK and `TestHost`
 * check it the same way.
 *
 * @internal
 */
export declare function thenProblem(then: unknown): string | null;
/**
 * A submenu that one of the player's built-in plugins declares: an entry of
 * {@link BUILTIN_SUBMENUS}.
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export interface BuiltinSubmenu {
    /**
     * The submenu's key path: the English texts paths merge by, the
     * built-in menu first by its key (`"Playback"` for `@playback`). Write
     * these segments, followed by your command's title, to put a command into
     * this submenu.
     */
    readonly path: readonly string[];
    /** The built-in plugins that declare it (`vestra.history`). */
    readonly plugins: readonly string[];
}
/**
 * The submenus the player's built-in plugins declare, by key path, in menu
 * order (built-in menus as in {@link MENU_IDS}, then top-level submenus).
 * Menu paths merge by their English key, so a command whose path starts
 * with the same keys appears in the same submenu, whatever the interface
 * language; the submenu's title is the declaring plugin's translation. The
 * player's own submenus (`Playback ▸ Speed`, `Playback ▸ Seek`) are not
 * listed.
 *
 * A snapshot of the player's built-in submenus at this SDK release; the
 * player may differ between SDK releases. The player's tests check that
 * this list agrees with what its built-in plugins declare when the SDK is
 * released.
 *
 * @example Add an item to the built-in Snapshot submenu
 * "Copy Snapshot" appears in Tools ▸ Snapshot ▸ next to the built-in
 * capture plugin's items. Paths merge by their English keys, so a command
 * whose path starts with the submenu's keys (`["Tools", "Snapshot"]`) lands
 * in the same submenu whatever the interface language.
 * ```ts
 * import type { Plugin } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare function copySnapshot(): Promise<void>;
 * // ---cut---
 * import { BUILTIN_SUBMENUS } from "@vestra-player/sdk/testing";
 *
 * const snapshot = BUILTIN_SUBMENUS.find((menu) => menu.path.at(-1) === "Snapshot")!;
 * plugin.commands.add({
 *   id: "snapshot-to-clipboard",
 *   // ["Tools", "Snapshot", "Copy Snapshot"]
 *   path: [...snapshot.path, "Copy Snapshot"],
 *   run: copySnapshot,
 * });
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export declare const BUILTIN_SUBMENUS: readonly BuiltinSubmenu[];
/**
 * A command the player or one of its built-in plugins registers: an entry
 * of {@link BUILTIN_COMMANDS}.
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export interface BuiltinCommand {
    /** `"host"` for the player's own commands, else the built-in plugin's id. */
    readonly owner: string;
    /** The command's id within its owner (`"play-pause"`, `"capture.save"`). */
    readonly id: string;
    /**
     * The menu path by key (the English texts paths merge by), the leaf
     * included; a built-in menu by its key (`"Video"`, not `"@video"`).
     * Empty for a command that is not in the main menu (a panel shortcut).
     */
    readonly path: readonly string[];
    /** The English title (the last segment of `path` when there is one). */
    readonly title: string;
    /** The default bindings, canonical, in the order the command declares them. */
    readonly bindings: readonly string[];
    /** `"panel"`: the bindings apply only while the owner's panel has focus. */
    readonly focus: "panel" | null;
}
/**
 * The commands the player and its built-in plugins register when the
 * player starts with nothing open, in menu order (commands outside the main
 * menu last): their key paths, default bindings and owners. Use it to pick
 * a menu place and default keys that do not collide with the player's (the
 * user's own changes come on top; see {@link @vestra-player/sdk!Bindings | Bindings}). Menu items built
 * from content (tracks, recent files, output devices) are not listed; their
 * placeholders (`No Audio Tracks`) are. Items that appear only in a
 * passing state are not listed either: the video plugin's Restart
 * (`video.gpu-rendering.restart`), in the Picture panel's Rendering section
 * beside GPU Rendering (not in a menu), shown while a rendering choice waits
 * for the next start.
 *
 * A snapshot of the player's built-in commands at this SDK release; the
 * player may differ between SDK releases. The player's tests check that
 * this list agrees with what the player and its built-in plugins register
 * when the SDK is released, not with the player a user runs later.
 *
 * @example Check in a test that a default key is still free
 * The test fails if the player or a built-in plugin already binds Ctrl+K
 * globally, before users ever see a conflict. It collects the default
 * bindings of every built-in command that is not limited to its panel's
 * focus (`focus: null`) and looks for the key the plugin wants; bindings
 * are canonical, so compare them as the player writes them.
 * ```ts
 * import assert from "node:assert/strict";
 * import { BUILTIN_COMMANDS } from "@vestra-player/sdk/testing";
 *
 * const taken = BUILTIN_COMMANDS.filter((command) => command.focus === null)
 *   .flatMap((command) => command.bindings);
 * assert.ok(!taken.includes("Ctrl+K"));
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export declare const BUILTIN_COMMANDS: readonly BuiltinCommand[];
export type CommandState = {
    enabled: boolean;
    checked: boolean | null;
    busy: boolean;
    /** Source text, or a text value. */
    reason: string | TextJson | ListJson | null;
};
/**
 * A command's conditions: `enabled` and `checked` evaluated by the player
 * (`plugin.when`).
 */
export type CommandWhen = {
    enabled?: ConditionJson;
    checked?: ConditionJson;
};
export type CommandDeclaration = {
    /** Source text, or a text value. */
    title: string | TextJson;
    /** Complete primary menu path INCLUDING its leaf; empty = not in the main menu. Segments may be text values. */
    path: (string | TextJson)[];
    /** Sibling order, ascending; a submenu takes its first command's order (a built-in menu keeps a fixed one). */
    order: number;
    /** Separator label: adjacent siblings of different groups get a line. */
    group: string;
    /** Draw `checked` as a radio dot. */
    radio: boolean;
    /** Default input bindings, canonical strings (see sdk/plugin/bindings.ts). */
    bindings: string[];
    /** "panel": bindings apply only while this plugin's panel has focus. */
    focus: "panel" | null;
    /** Fire again on keyboard auto-repeat while held. */
    repeat: boolean;
    /**
     * A bottom-bar button: an icon name, or an image in the `plugin.ui.image`
     * wire form.
     */
    button?: {
        icon: string | Json;
        order: number;
    };
    /**
     * The command's category, source text or a text value.
     */
    category?: string | TextJson;
    /**
     * The local id of the command that opens where this one lives. Sent only when
     * set, like `category`.
     */
    home?: string;
    /** The command cannot be undone; sent only when `true`. */
    destructive?: boolean;
};
/**
 * `commands.keys`: each of the plugin's own commands, by local id, and each of
 * the player's own as `host/<id>`, with the labels of its current keys; the
 * answer to the request and the data of the event of that name.
 *
 * @remarks
 * The player's wire shape (`commands.keys`), part of the protocol, which is not
 * semver: it can gain fields with a player version.
 *
 * @since 1.7.0
 * @category Playback
 */
export type CommandKeys = {
    /** The key labels of each command, by local id (the player's own: `host/<id>`). */
    keys: Record<string, string[]>;
    /** The player's binding table revision: a larger one is newer. */
    revision: number;
};
export declare function commandClient(transport: Transport): {
    register: (id: string, revision: number, declaration: CommandDeclaration, state: CommandState, when?: CommandWhen) => Promise<{
        revision: number;
    }>;
    /** Remove the command from the menu and the binding table; the id is free again. */
    unregister: (id: string) => Promise<void>;
    /**
     * The key labels of the plugin's own commands, as menus show them; later
     * changes come as the `commands.keys` event, in the same shape. `revision`
     * orders answers and events.
     */
    keys: () => Promise<CommandKeys>;
    update: (id: string, revision: number, state: CommandState, when?: CommandWhen) => Promise<{
        revision: number;
    }>;
    /** The palette's table (`commands.run`). */
    list: () => Promise<CommandList>;
    /** Run any listed command for a user action's ticket (`commands.run`). */
    run: (params: {
        id: string;
        ticket: string;
        step?: number;
        of?: number;
        then?: string[];
    }) => Promise<CommandRunResult>;
};
