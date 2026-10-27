import type { Server } from "node:net";
import { HostError } from "../wire.ts";
import type { PackageManifest, PackageSource } from "./types.ts";
import type { Json, LayoutSlot, LocalServices, PlaybackStats, Transport, WireSystemInfo } from "../wire.ts";
import { BUILTIN_COMMANDS, BUILTIN_SUBMENUS, COMMAND_LIMITS, MENU_IDS } from "../commands.ts";
import type { BuiltinCommand, BuiltinSubmenu } from "../commands.ts";
import { CAPABILITY_INFO, MESSAGE_LIMIT } from "../capabilities.ts";
import type { CapabilityInfo } from "../capabilities.ts";
import { OSD_LIMITS } from "./osd.ts";
import { SECRET_LIMITS, SESSION_STORAGE_LIMITS } from "./services.ts";
import { BINDING_INPUTS } from "./bindings.ts";
import { THEME_PALETTES } from "../palettes.ts";
import { IMAGE_LIMITS, UI_VALUE_LIMITS } from "./values.ts";
import type { CommandDeclaration } from "../commands.ts";
import type { SourceState } from "../source.ts";
import type { SubtitleState } from "../subtitles.ts";
import type { RenderSettings, ViewSettings } from "./types.ts";
import type { MediaJob } from "../media.ts";
import type { LanguagePreference, Locale, Plugin, TemplateArgs, TextValue, ThemeId } from "./types.ts";
import { LOCALES, LIST_SEPARATORS, TEXT_LIMITS } from "../i18n.ts";
/** Throw one of these from `host.handle(...)` to simulate a definite refusal. */
export { HostError };
/** The player's command caps, which the test host enforces as well. */
export { COMMAND_LIMITS, MENU_IDS };
/** The submenus of the player's built-in plugins, by key path. */
export { BUILTIN_COMMANDS, BUILTIN_SUBMENUS };
export type { BuiltinCommand, BuiltinSubmenu };
/** The manifest capabilities: what each opens and how the store shows it. */
export { CAPABILITY_INFO };
export type { CapabilityInfo };
/** The longest message a plugin may send the player. */
export { MESSAGE_LIMIT };
/** The player's limits on the on-screen display. */
export { OSD_LIMITS };
/** The player's limits on stored secrets. */
export { SECRET_LIMITS };
/** The player's limits on `plugin.storage.session`. */
export { SESSION_STORAGE_LIMITS };
/** The names bindings are made of. */
export { BINDING_INPUTS };
/** The colours of the player's palettes, by `Theme` token. */
export { THEME_PALETTES };
/** The player's property value limits, which the test host enforces as well. */
export { IMAGE_LIMITS, UI_VALUE_LIMITS };
/**
 * The player's limits on one text value ({@link Text}, {@link TextList},
 * with everything nested in it), which the test host enforces as well:
 * nesting depth 8, 1024 texts, lists and arguments in all, 16 arguments per
 * text, 256 items per list, 4096 bytes per source text or string argument.
 *
 * @since 1.5.11
 */
export { TEXT_LIMITS };
/**
 * The separator of each {@link ListStyle} per interface language, as the
 * player joins a {@link TextList}.
 *
 * @since 1.5.11
 */
export { LIST_SEPARATORS };
/**
 * The player's interface languages, English (the source) first, as the
 * player's own table: `["en", "zh-Hans", "ja", "ko", "de", "fr", "es"]`.
 *
 * @example
 * ```ts
 * import { LOCALES } from "@vestra-player/sdk/testing";
 *
 * for (const locale of LOCALES.slice(1)) {
 *   const { host, stop } = await startPlugin(activate, { directory, locale });
 *   assert.notEqual(host.command("open").display.title, "Open…");
 *   await stop();
 * }
 * ```
 */
export { LOCALES };
type Handler = (params: Record<string, Json>) => unknown;
type Call = {
    method: string;
    params: Record<string, Json>;
};
/**
 * A tool window as the test host keeps it: the record of a
 * `plugin.windows.create(...)` window after its first `show()`. Get one with
 * {@link TestHost.window}.
 *
 * @example
 * ```ts
 * await host.command("open-picture").run(); // the plugin calls window.show()
 * const picture = host.window("picture");
 * assert.equal(picture.visible, true);
 * picture.emit("adjust", "brightness", 40); // fire a Slint callback
 * await host.settle();
 * assert.equal(picture.props["brightness"], 40);
 * ```
 */
export interface FakeWindow {
    /** The `.slint` file the window was created from (package-relative). */
    file: string;
    /** Whether the window is shown (`show()`) or hidden (`hide()`). */
    visible: boolean;
    /**
     * The properties the plugin wrote, by their Slint (kebab-case) names:
     * `canMark` is stored as `can-mark`. Runtime images appear in their wire
     * form; read the path with {@link TestHost.imagePath}.
     */
    props: Record<string, Json>;
    /** The Slint callback names (kebab-case) the plugin bound with `on()`. */
    callbacks: string[];
    /**
     * Fire a Slint callback as if the user triggered it in the window. The
     * name may be kebab-case or camelCase; `args` are the callback arguments.
     * Follow it with `await host.settle()` before asserting.
     */
    emit(callback: string, ...args: Json[]): void;
    /**
     * The user closes the window with its title-bar button: the window closes
     * and the plugin's `window.onClose` listeners run (unlike the plugin's own
     * `close()`).
     */
    userClose(): void;
}
/**
 * A file dialog as the player shows it ({@link TestHost.fileDialogs}).
 *
 * @since 1.5.11
 */
export interface FileDialogText {
    /** Which dialog. */
    kind: "openFile" | "openFiles" | "openDirectory" | "saveFile";
    /** Its title as shown, if the plugin gave one. */
    title?: string;
    /** The proposed file name as shown, if the plugin gave one. */
    fileName?: string;
}
/**
 * A command the plugin registered with `plugin.commands.add`, as the test
 * host keeps it. Get one with {@link TestHost.command} (the plugin's own) or
 * {@link TestHost.coreAction} (the player's core actions).
 *
 * @example
 * ```ts
 * const toggle = host.command("toggle-loop");
 * assert.deepEqual(toggle.declaration.path, ["Playback", "Loop"]);
 * assert.equal(toggle.state.checked, false);
 * await toggle.run();
 * assert.equal(host.command("toggle-loop").state.checked, true);
 * ```
 */
export interface FakeCommand {
    /**
     * What the plugin declared: id, menu path, title, order, group, default
     * bindings, focus and repeat. Wire-level shape; it may gain fields in minor
     * versions.
     */
    declaration: CommandDeclaration;
    /**
     * What the menu shows, as the player shows it in the test host's
     * {@link TestHost.locale}: the title and each path segment translated with
     * the plugin's catalog (the player's own menus in the player's words), the
     * keys normalized (`@video` is "Video").
     *
     * @since 1.5.9
     */
    readonly display: {
        /** The menu text of the item. */
        title: string;
        /** Each segment of the path as shown; empty without a path. */
        path: string[];
        /**
         * The disabled or busy reason as shown, `null` without one (a plugin
         * command's; not enumerable, so comparing `display` with
         * `{ title, path }` still works).
         *
         * @since 1.5.11
         */
        reason?: string | null;
    };
    /** The command state the player last received (after the update pass). */
    state: {
        /** `false` when the plugin disabled the command. */
        enabled: boolean;
        /** The check mark: `true`, `false`, or `null` for none. */
        checked: boolean | null;
        /** `true` while the plugin reports the command as busy. */
        busy: boolean;
        /** The reason the plugin sent (source text or a text value; see `display.reason`). */
        reason: TextValue | null;
    };
    /**
     * The state revision the player holds; increases with every state update
     * the SDK sends.
     */
    revision: number;
    /**
     * Execute the command the way the menu would, then wait until the host is
     * idle. Rejects (with a {@link HostError}) when the command is disabled or
     * busy, like the player, which does not call `run` then.
     */
    run(): Promise<void>;
}
/**
 * A generated stream (`plugin.player.openStream`) as the test host sees it:
 * the fake player end of the frame connection. Assertion surface; see
 * {@link TestHost.producers}.
 */
export interface FakeProducer {
    /** The producer id the host assigned (`1`, `2`, … per host). */
    id: number;
    /** Audio sample frames received so far (one stereo f32 pair = 1 frame). */
    receivedFrames: number;
    /** Video frames received so far. */
    videoFrames: number;
    /** `true` once the plugin released the producer (stream stopped or disposed). */
    released: boolean;
    /**
     * `true` once the player retired the producer, as the real player does at
     * once: another source replaced it (`open`, another stream, a resolved
     * file), the source was closed, or {@link TestHost.failStream} failed it.
     * A retired producer's `producer.state` answers `unavailable`.
     */
    retired: boolean;
    /**
     * `true` if the frame connection closed while this stream was still the
     * player's source: the player reports that as a playback failure
     * ("producer transport closed"). `stop()` closes the source first.
     */
    droppedWhileSelected: boolean;
}
/** A Markdown document as {@link TestHost.markdown} reads it back. */
export interface MarkdownWire {
    /** The Markdown text. */
    text: string;
    /** The base given to `ui.markdown`, or `null`. */
    base: string | null;
    /** Remote image address → the file the SDK downloaded for it. */
    images: Record<string, string>;
}
/**
 * An in-memory fake Vestra Player for plugin tests.
 *
 * The plugin under test talks to it over the real wire protocol (the host is
 * the plugin's transport), so everything the SDK does runs unchanged. The
 * host answers each request with simplified player behaviour, records what
 * the plugin did, and lets the test act as the user or another plugin.
 *
 * Members fall into a few groups:
 *
 * - **Drive the plugin**: {@link TestHost.command}`(id).run()`,
 *   {@link TestHost.press}, {@link TestHost.window}`(id).emit(...)`,
 *   {@link TestHost.mounted}`().emit(...)`, {@link TestHost.drop},
 *   {@link TestHost.step}, {@link TestHost.dialogAnswers}.
 * - **Simulate the player and other plugins**: {@link TestHost.setSource},
 *   {@link TestHost.openElsewhere}, {@link TestHost.setMainWindow},
 *   {@link TestHost.setTimelineLayer}, {@link TestHost.setAppearance},
 *   {@link TestHost.setPackages}, {@link TestHost.emit}.
 * - **Preset data** the plugin reads: {@link TestHost.stats},
 *   {@link TestHost.app}, {@link TestHost.systemInfo},
 *   {@link TestHost.chapters}, {@link TestHost.audioDevices},
 *   {@link TestHost.secrets}, {@link TestHost.clipboard},
 *   {@link TestHost.tools}.
 * - **Assert** on records: {@link TestHost.osd}, {@link TestHost.calls},
 *   {@link TestHost.sent}, {@link TestHost.conflicts},
 *   {@link TestHost.layout}, {@link TestHost.menus}, and many more.
 * - **Inject failures**: {@link TestHost.handle} / {@link TestHost.reset}.
 * - **Synchronise**: {@link TestHost.settle}.
 *
 * Usually created by {@link startPlugin}. Create one yourself only to preset
 * data that the plugin reads during activation (`app`, `appearance`,
 * `secrets`, ...), then pass it as `startPlugin(activate, { host })`.
 *
 * Fields that mirror player state at the wire level (`source`, `subtitles`,
 * `processing`, `snapshots`, `jobs`, `producers`, `uploads`, `resolvers`,
 * `calls`) use the protocol's shapes. They are fine for assertions but are
 * not part of the plugin API: their shape may change in minor versions.
 *
 * @example
 * ```ts
 * import assert from "node:assert/strict";
 * import test from "node:test";
 * import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
 * import activate from "../main.ts";
 *
 * test("shows the playing file in the panel", async () => {
 *   const host = new TestHost();
 *   host.appearance = "vestra"; // read during activation
 *   const { stop } = await startPlugin(activate, {
 *     directory: new URL("..", import.meta.url),
 *     host,
 *   });
 *   host.openElsewhere("/media/clip.mp4"); // another plugin opened a file
 *   await host.settle();
 *   assert.equal(host.mounted().props["title"], "clip.mp4");
 *   await stop();
 * });
 * ```
 */
export declare class TestHost implements Transport {
    #private;
    /**
     * Every request the plugin sent, in order: `{ method, params }`.
     * Wire-level; prefer {@link TestHost.sent} for one method.
     */
    readonly calls: Call[];
    /** The host events the plugin subscribed to (needs `events` etc.). */
    readonly subscriptions: Set<string>;
    /**
     * The player's source facts (phase, position, duration, paused, rate,
     * audio tracks, loop, ...). Wire-level shape; change it with
     * {@link TestHost.setSource}, which also broadcasts it.
     */
    source: SourceState;
    /** Tool windows by id (see {@link TestHost.window}). */
    readonly windows: Map<string, FakeWindow>;
    /**
     * The plugin's mounted panel (bottom bar, sidebar or backdrop), or `null`.
     * See {@link TestHost.mounted}.
     */
    panel: (Omit<FakeWindow, "callbacks" | "userClose"> & {
        /** `"bottom"`, `"sidebar"` or `"backdrop"`. */
        slot: string;
        /** The exported component the SDK found in the file. */
        component: string;
        /** The last `panel.setInteraction(...)` locks, `null` before any. */
        locks: Json;
    }) | null;
    /** The plugin's commands by local id (see {@link TestHost.command}). */
    readonly commands: Map<string, FakeCommand>;
    /** The player's 14 core actions (`host/<id>`), kept apart from the plugin's `commands`. */
    readonly coreCommands: Map<string, FakeCommand>;
    /**
     * Both panel positions as the player reports them (`null` while no panel
     * is mounted there). You may change a slot directly and then call
     * `host.emit("window.state", {})` to simulate a change made elsewhere.
     *
     * @example
     * ```ts
     * assert.equal(host.layout.sidebar?.preferred, true);
     * await plugin.panels.sidebar.toggle();
     * assert.equal(host.layout.sidebar?.preferred, false);
     * ```
     */
    readonly layout: {
        /**
         * The bottom bar position: `preferred`, `effective`, `reason`,
         * `detached`, `owner` and `panels` (its one panel).
         */
        bottom: LayoutSlot | null;
        /**
         * The sidebar position: `preferred`, `effective`, `reason`, `detached`;
         * `panels` are its tabs in order (`{ owner, plugin, title }`), `owner`
         * the current tab's (the plugin under test is owner 1).
         */
        sidebar: LayoutSlot | null;
    };
    /**
     * Every `osd.show` text as the player shows it (translated with the
     * plugin's catalog and formatted with its args), newest last. The
     * template and args the plugin sent are in `host.sent("osd.show")`.
     */
    readonly osd: string[];
    /** The text of this plugin's osd panel as shown, `null` when hidden. */
    osdPanel: string | null;
    /**
     * The interface language the test host plays (`plugin.i18n.locale`), `"en"`
     * by default. Set it before {@link startPlugin} (or pass its `locale`
     * option): the plugin's catalog for it is read from the package's
     * `locales/` then, and the menus ({@link FakeCommand.display}), OSD
     * messages, dialog texts, settings page and panel titles show its
     * translations, the way the player shows them.
     *
     * The player's own texts (core actions' messages and menus, its
     * refusals) are English, or the player's Simplified Chinese for
     * `"zh-Hans"`. While `locale` was never set they stay in Chinese, as
     * players up to 0.4.2 showed them, so tests written for SDK 1.5.8 keep
     * passing; set `locale` to test what a user sees now.
     *
     * @example
     * ```ts
     * const { host, stop } = await startPlugin(activate, { directory, locale: "zh-Hans" });
     * assert.deepEqual(host.command("blur").display.path, ["视频", "模糊…"]);
     * await stop();
     * ```
     *
     * @since 1.5.9
     */
    get locale(): Locale;
    set locale(locale: Locale);
    /**
     * Read the plugin's translations for {@link TestHost.locale} from a
     * package directory, as the player would (`locales/<locale>.json` of a
     * built package, else the source `locales/<locale>.po`); `startPlugin`
     * does this. None for English; a missing file means none; a broken `.po`
     * throws.
     *
     * @param directory - The package directory (holding `locales/`).
     * @since 1.5.9
     */
    loadCatalog(directory: string): void;
    /**
     * A text of the plugin as the player shows it: translated with the
     * plugin's catalog ({@link TestHost.loadCatalog}), then formatted with
     * `args` when given.
     *
     * @param text - English source text or template.
     * @param args - Values for its placeholders.
     * @returns The shown text.
     *
     * @example
     * ```ts
     * assert.equal(host.translate("Speed {}×", [2]), "倍速 2×");
     * ```
     *
     * @since 1.5.9
     */
    translate(text: TextValue, args?: TemplateArgs): string;
    /**
     * The saved interface language (`plugin.appearance.language`), changed by
     * `appearance.setLanguage` or {@link TestHost.setLanguage}.
     *
     * @since 1.5.9
     */
    language: LanguagePreference;
    /**
     * Simulate another plugin (or the user) saving the interface language:
     * pushes `appearance.changed`.
     *
     * @param language - `"system"` or a locale.
     * @since 1.5.9
     */
    setLanguage(language: LanguagePreference): void;
    /**
     * Every `dialogs.confirm` as the player shows it: its texts translated
     * (message and detail formatted with `args`), newest last. The options
     * the plugin sent are in {@link TestHost.confirms}.
     *
     * @since 1.5.9
     */
    readonly confirmTexts: {
        /** The question. */
        message: string;
        /** The text under it, if any. */
        detail?: string;
        /** The confirm button. */
        confirm: string;
        /** The cancel button (the player's "Cancel" by default). */
        cancel: string;
        /** The checkbox, if any. */
        checkbox?: string;
    }[];
    /**
     * Every file dialog (`openFile`, `openFiles`, `openDirectory`,
     * `saveFile`) as the player shows it: the title translated, a text file
     * name translated, newest last. The options the plugin sent are in
     * `host.sent("dialogs.open")`.
     *
     * @since 1.5.11
     */
    readonly fileDialogs: FileDialogText[];
    /** The sequence provider's flags, `null` without a provider. */
    sequence: {
        /** The provider's `canNext`, as last evaluated. */
        canNext: boolean;
        /** The provider's `canPrevious`, as last evaluated. */
        canPrevious: boolean;
    } | null;
    /**
     * What `player.stats()` returns. Fields added in later rounds may be left
     * out; the SDK fills them with `null`.
     *
     * @example
     * ```ts
     * host.stats = {
     *   ...host.stats,
     *   video: { codec: "h264", width: 1920, height: 1080, fps: 24 },
     * } as typeof host.stats;
     * ```
     */
    stats: PlaybackStats;
    /** Every `subtitles.load`: its options and bytes, newest last. */
    readonly subtitleLoads: {
        /** The `format` option (`"srt"` by default). */
        format: "srt" | "ass";
        /** The `title` option, `null` when omitted. */
        title: string | null;
        /** The `select` option (`"primary"` by default). */
        select: "primary" | "secondary" | "none";
        /** The uploaded file bytes. */
        bytes: Buffer;
    }[];
    /**
     * The fake system clipboard. Preset `text` to test `readText`;
     * `writeText` replaces it and `writeImage` appends the image path.
     */
    readonly clipboard: {
        /** The clipboard's text, `null` when it holds none. */
        text: string | null;
        /** Paths given to `clipboard.writeImage`, newest last. */
        images: string[];
    };
    /** The effective `power.preventSleep` mode. */
    power: "off" | "playing";
    /**
     * The fake `ffmpeg` / `ffprobe` behind `plugin.tools`. A tool without a
     * handler is not found (`available()` is false, `run()` rejects with
     * `unavailable`).
     *
     * @example
     * ```ts
     * host.tools.handle("ffprobe", (args) => {
     *   assert.ok(args.includes("-show_streams"));
     *   return { stdout: JSON.stringify({ streams: [] }) };
     * });
     * host.tools.handle("ffmpeg", () => {
     *   throw new HostError("failed", "Invalid data found when processing input");
     * });
     * await host.command("probe").run();
     * assert.equal(host.tools.calls[0]?.tool, "ffprobe");
     * ```
     */
    readonly tools: {
        /** Every run: the tool name and its arguments, newest last. */
        calls: {
            /** `"ffmpeg"` or `"ffprobe"`. */
            tool: string;
            /** The arguments, as passed to `tools.run`. */
            args: string[];
        }[];
        /** @internal The installed handlers by tool; use `handle()`. */
        handlers: Map<string, (args: string[]) => {
            stdout?: Uint8Array | string;
            stderr?: string;
        } | Promise<{
            stdout?: Uint8Array | string;
            stderr?: string;
        }>>;
        /**
         * Make `tool` available and answer each run with `handler(args)`. Return
         * `{ stdout?, stderr? }` (a string `stdout` is UTF-8 encoded), or throw a
         * {@link HostError} to fail the run with that code. A later call
         * replaces the handler.
         */
        handle(tool: "ffmpeg" | "ffprobe", handler: (args: string[]) => {
            stdout?: Uint8Array | string;
            stderr?: string;
        } | Promise<{
            stdout?: Uint8Array | string;
            stderr?: string;
        }>): void;
    };
    /**
     * Paths given to `system.reveal` / `system.openPath`, newest last; inject
     * failures with `host.handle("system.openPath", …)`.
     */
    readonly system: {
        /** Paths given to `system.reveal`, newest last. */
        revealed: string[];
        /** Paths given to `system.openPath`, newest last. */
        opened: string[];
        /** `system.openUrl` addresses, newest last. */
        urls: string[];
    };
    /** @internal Local processes, faked: see {@link TestHost.tools}. */
    readonly local: LocalServices;
    /** Every `panel.showMenu`, newest last. */
    readonly menus: {
        /** Command ids, `null` for a separator. */
        items: (string | null)[];
        /** `at.x`, in window coordinates. */
        x: number;
        /** `at.y`, in window coordinates. */
        y: number;
    }[];
    /**
     * Timeline layers by plugin id (this plugin's is `manifestId`) and the
     * merged list `plugin.timeline.marks` reads. A new source clears them.
     */
    readonly timeline: {
        /** Each plugin's layer by plugin id; layers with no marks are absent. */
        layers: Map<string, {
            /** Seconds from the start. */
            time: number;
            /** `"chapter"`, `"bookmark"` or `"marker"`. */
            kind: string;
            /** The mark's title, `null` without one. */
            title: string | null;
        }[]>;
        /** Every layer merged, sorted by time, with the publishing plugin's id. */
        readonly marks: {
            /** The manifest id of the plugin that published the mark. */
            plugin: string;
            /** Seconds from the start. */
            time: number;
            /** `"chapter"`, `"bookmark"` or `"marker"`. */
            kind: string;
            /** The mark's title, `null` without one. */
            title: string | null;
        }[];
    };
    /**
     * Simulate another plugin publishing its timeline layer (or clearing it
     * with `[]`); pushes `timeline.changed` when the plugin subscribed.
     *
     * @param pluginId - The other plugin's manifest id.
     * @param marks - Its whole layer.
     *
     * @example
     * ```ts
     * host.setTimelineLayer("community.chapters", [
     *   { time: 0, kind: "chapter", title: "Intro" },
     *   { time: 95, kind: "chapter", title: "Part 1" },
     * ]);
     * await host.settle();
     * assert.equal(plugin.timeline.marks.length, 2);
     * ```
     */
    setTimelineLayer(pluginId: string, marks: {
        time: number;
        kind: string;
        title?: string | null;
    }[]): void;
    /**
     * Runtime images written to views (`plugin.ui.image`,
     * `plugin.ui.remoteImage`), whole values and (SDK 1.5.19) images nested
     * in rows, newest last. The SDK refuses images of released scopes before
     * anything is sent. The test host does not read the files, so it does not
     * check the player's per-view budget (`IMAGE_LIMITS.viewBytes`).
     */
    readonly images: {
        /** The image file's path, as given to `ui.image`. */
        path: string;
        /** The SDK's reference number for that `ImageRef`. */
        ref: number;
        /** Its `maxSide` option, when given. */
        maxSide?: number;
    }[];
    /**
     * The file an image property shows, or `null` when the value is not a
     * runtime image. View properties hold images in their wire form; this
     * reads the path back.
     *
     * @example
     * ```ts
     * assert.equal(
     *   host.imagePath(host.window("cover").props["cover"]),
     *   join(plugin.dataDirectory, "cover.png"),
     * );
     * ```
     */
    imagePath(value: Json | undefined): string | null;
    /** `drops.provide` registrations of this plugin, in order (`id` is the SDK's). */
    readonly dropProviders: {
        /** The SDK's provider id. */
        id: string;
        /** The extensions it takes (lower case, `"folder"` for directories). */
        extensions: string[];
    }[];
    /** The panel has an `onDrop` listener. */
    panelDrops: boolean;
    /**
     * Drop files like the player: on a panel with an `onDrop` listener the
     * whole list goes there; otherwise (and on the video area) grouped by
     * extension to the first provider (`folder` for existing directories).
     * Files nobody takes are ignored. Resolves once the host is idle again.
     *
     * @param paths - The dropped paths, in the order the file manager gives.
     * @param options - `target`: `"video"` (default) or `"panel"`; `at`: the
     *   drop point in panel coordinates (default `{ x: 0, y: 0 }`);
     *   `modifiers`: keys held (default all `false`).
     *
     * @example
     * ```ts
     * await host.drop(["/media/a.srt", "/media/b.mkv"]);
     * await host.drop(["/media/list.m3u"], {
     *   target: "panel",
     *   at: { x: 10, y: 48 },
     *   modifiers: { alt: true },
     * });
     * ```
     */
    drop(paths: string[], options?: {
        target?: "video" | "panel";
        at?: {
            x: number;
            y: number;
        };
        modifiers?: Partial<{
            alt: boolean;
            shift: boolean;
            cmd: boolean;
        }>;
    }): Promise<void>;
    /** The palette (`plugin.appearance.theme`); `setAppearance` changes it elsewhere. */
    appearance: ThemeId;
    /**
     * Simulate another plugin switching the palette: updates
     * {@link TestHost.appearance} and pushes `appearance.changed`.
     */
    setAppearance(theme: ThemeId): void;
    /**
     * Settings pages by id (`plugin.settings.page`), every `show` (page id or
     * null), and the page the options window shows.
     */
    readonly settings: {
        /** Registered pages by page id. */
        pages: Map<string, {
            /** The page's settings category. */
            category: string;
            /** The page title. */
            title: string;
            /** The page order within its category. */
            order: number;
            /** The `.slint` file (package-relative). */
            file: string;
            /** The exported component the SDK found. */
            component: string;
            /** The exported global (the page's bridge) the SDK found. */
            global: string;
            /** The page's properties by Slint (kebab-case) name. */
            props: Record<string, Json>;
            /** Fire one of the page's callbacks. */
            emit(callback: string, ...args: Json[]): void;
        }>;
        /** Every `settings.show(pageId)`: the page id, `null` when omitted. */
        shows: (string | null)[];
        /** The page the options window shows, `null` before any `show`. */
        current: string | null;
        /** Whether the options window is open. */
        open: boolean;
    };
    /**
     * One settings page by id. Throws when it was never registered.
     *
     * @example
     * ```ts
     * const page = host.settingsPage("general");
     * page.emit("setStep", 10);
     * await host.settle();
     * assert.equal(page.props["step"], 10);
     * ```
     */
    settingsPage(id: string): {
        /** The page's settings category. */
        category: string;
        /** The page title. */
        title: string;
        /** The page order within its category. */
        order: number;
        /** The `.slint` file (package-relative). */
        file: string;
        /** The exported component the SDK found. */
        component: string;
        /** The exported global (the page's bridge) the SDK found. */
        global: string;
        /** The page's properties by Slint (kebab-case) name. */
        props: Record<string, Json>;
        /** Fire one of the page's callbacks. */
        emit(callback: string, ...args: Json[]): void;
    };
    /** The backdrop: who mounted it and whether it shows (`panels.backdrop.state`). */
    backdrop: {
        /** The manifest id of the plugin that mounted it. */
        owner: string;
        /** Whether it shows now (no video frame). `true` when mounted. */
        effective: boolean;
    } | null;
    /**
     * Simulate the backdrop's `effective` changing (`false`: a video frame
     * appeared; `true`: there is none again); pushed to the plugin.
     */
    setBackdropEffective(effective: boolean): void;
    /**
     * Simulate another plugin mounting a panel, before or after the plugin
     * under test, as the player handles it: in the sidebar it is another tab
     * (after the existing ones; the first panel's tab is current); the bottom
     * bar and the backdrop take one panel, so the plugin's own mount there
     * then rejects with `conflict` (not an activation failure, as in the
     * player). Pushed as a `window.state` change.
     *
     * @param slot - The position.
     * @param plugin - The other plugin's manifest id.
     * @param options - `title`: its tab label (default: `plugin`).
     *
     * @example The playlist next to this plugin's sidebar panel
     * ```ts
     * const host = new TestHost();
     * host.mountOther("sidebar", "vestra.playlist", { title: "播放列表" });
     * const { plugin } = await startPlugin(activate, { host, directory });
     * assert.equal(host.layout.sidebar?.panels?.length, 2);
     * ```
     *
     * @since 1.5.2
     */
    mountOther(slot: "bottom" | "sidebar" | "backdrop", plugin: string, options?: {
        title?: string;
    }): void;
    /**
     * Simulate another plugin's panel going away ({@link TestHost.mountOther}):
     * when its tab was current, the next tab (else the previous one) is.
     *
     * @since 1.5.2
     */
    unmountOther(plugin: string): void;
    /**
     * Simulate the user clicking a sidebar tab: `plugin`'s panel (the plugin
     * under test's `manifestId`, or one of {@link TestHost.mountOther})
     * becomes the current tab. Pushed as a `window.state` change.
     *
     * @since 1.5.2
     */
    selectTab(plugin: string): void;
    /**
     * The display layer (`plugin.video.view()`): who owns it (this plugin is
     * `manifestId`) and the values the player holds.
     */
    readonly videoView: {
        /** The owning plugin's manifest id, `null` while nobody owns it. */
        owner: string | null;
        /** The values the player holds. */
        values: ViewSettings;
    };
    /**
     * Simulate another plugin taking the display layer first (the plugin's
     * `video.view()` then fails with `conflict`), or, with `null`, releasing
     * it (values reset to the defaults).
     */
    setVideoViewOwner(pluginId: string | null): void;
    /**
     * The render options (`plugin.video.render()`): who owns them, the values
     * the player holds, and whether it renders on the GPU (`gpu: false` is
     * software rendering, where every `set` fails with `unsupported`).
     */
    readonly videoRender: {
        /** The owning plugin's manifest id, `null` while nobody owns them. */
        owner: string | null;
        /** The values the player holds. */
        values: RenderSettings;
        /** `true`: GPU rendering; `false`: software rendering (`unsupported`). */
        gpu: boolean;
    };
    /**
     * Simulate another plugin owning the render options (the plugin's
     * `video.render()` then fails with `conflict`), or, with `null`, releasing
     * them (values reset to the defaults).
     */
    setVideoRenderOwner(pluginId: string | null): void;
    /**
     * What the player pushes when mpv cannot compile a shader (code `failed`)
     * or switches to software rendering (`unsupported`, which also sets
     * `videoRender.gpu` to false). Pushed only when this plugin is the owner.
     *
     * @example
     * ```ts
     * host.failVideoRender("failed", "shader compile failed: ERROR: 0:39: ...");
     * await host.settle();
     * assert.equal(render.status.state, "failed");
     * ```
     */
    failVideoRender(code: "failed" | "unsupported", message: string): void;
    /**
     * The player's processing (filter) state. Wire-level; may change in minor
     * versions. `resources` holds each `video.filter` / `audio.filter` chain the
     * plugin applied, keyed by the SDK's resource id.
     *
     * @example
     * ```ts
     * const [chain] = [...host.processing.resources.values()];
     * assert.deepEqual(chain?.nodes, [{ kind: "hue", degrees: 30 }]);
     * ```
     */
    readonly processing: {
        /** Increases with every accepted change. */
        configurationRevision: number;
        /** Set a stage here to simulate a blocked backend (`backend-uncertain`). */
        backendUncertain: Partial<Record<"video" | "audio", string>>;
        /** Applied chains by resource id; an empty chain removes its entry, as in the player. */
        resources: Map<string, {
            /** `"video"` or `"audio"`. */
            stage: string;
            /** The chain's revision. */
            revision: number;
            /** The applied nodes. */
            nodes: Json;
            /** The audio delay in milliseconds (audio chains only). */
            offsetMs?: number;
        }>;
        /** Outcomes of processing operations, for the SDK's recovery queries. */
        book: Map<string, {
            /** The operation's outcome. */
            status: "completed" | "failed";
            /** The result of a completed operation. */
            result?: Json;
            /** The refusal of a failed operation. */
            error?: {
                /** The error code. */
                code: string;
                /** The error message. */
                message: string;
            };
        }>;
    };
    /**
     * The player's subtitle state: tracks, selection, style layers, fonts and
     * loaded resources. Wire-level shape (may change in minor versions); add
     * tracks to `subtitles.tracks` (entries may carry `codec`, `text`,
     * `streamIndex`) to simulate a source's subtitles.
     */
    subtitles: SubtitleState;
    /** Subtitle uploads in progress, by upload id. Wire-level. */
    readonly uploads: Map<number, {
        /** The chunks received so far. */
        bytes: Buffer[];
        /** The announced total size in bytes. */
        expected: number;
    }>;
    /** Snapshots taken with `media.capture`, by snapshot id. Wire-level. */
    readonly snapshots: Map<number, Json>;
    /** Every `jobs.start` request (`kind`, `destination`, `quality`, …), newest last. */
    readonly exports: Record<string, Json>[];
    /**
     * Export jobs by job id. Each state query advances a job one step
     * (`queued` → `running` → `completed`; `cancelling` → `cancelled`).
     * Wire-level.
     */
    readonly jobs: Map<number, MediaJob>;
    /**
     * `openStream`'s transport: a local stream (a Unix socket on macOS and
     * Linux, a named pipe on Windows) or, with `"tcp"`, loopback TCP where the
     * plugin must send the token first, as the player on Windows answers.
     * Set it before the plugin calls `openStream`.
     */
    producerTransport: "unix" | "tcp";
    /**
     * Generated streams (`player.openStream`) by producer id: the fake player
     * end of each frame connection. Wire-level beyond the
     * {@link FakeProducer} fields.
     */
    readonly producers: Map<number, FakeProducer & {
        /** The listening server for the frame connection. */
        server: Server;
        /** The Unix socket path (unused with TCP). */
        path: string;
        /** TCP: the connection sent the right token (always true for Unix). */
        admitted: boolean;
        /** The current start/stop control epoch. */
        epoch: number;
        /** A control waiting for the plugin's acknowledgement, or `null`. */
        pending: Json | null;
        /** Whether the last control was acknowledged. */
        acknowledged: boolean;
        /** Whether the stream is producing (selected and started). */
        active: boolean;
    }>;
    /** Registered source resolvers (`resolvers.add`) by id: their declarations. */
    readonly resolvers: Map<string, Json>;
    /**
     * Resolvers other plugins hold, as `resolvers.add` declares them plus the
     * holder's id: a registration of this plugin that claims one of their
     * extensions (or scheme + host pairs) is refused with `conflict`, which
     * does not fail activation, like the player.
     *
     * @example
     * ```ts
     * const host = new TestHost();
     * host.otherResolvers.push({ owner: "example.emulator", extensions: ["nes"] });
     * ```
     */
    readonly otherResolvers: {
        /** The manifest id of the plugin holding them (shown in the refusal). */
        owner: string;
        /** File extensions it resolves (a file resolver), lower case, no dot. */
        extensions?: string[];
        /** URL schemes it resolves (a web resolver), with `hosts`. */
        schemes?: string[];
        /** Exact host names it resolves (a web resolver), with `schemes`. */
        hosts?: string[];
    }[];
    /**
     * What `plugin.resolvers.fileTypes` reports: the extensions of this
     * plugin's file resolvers and of {@link TestHost.otherResolvers}, sorted,
     * each once.
     */
    fileTypes(): string[];
    /**
     * Push `resolvers.fileTypes` when {@link TestHost.fileTypes} changed:
     * called after this plugin's resolvers come and go; call it yourself
     * after changing {@link TestHost.otherResolvers} (another plugin started
     * or stopped).
     *
     * @example
     * ```ts
     * host.otherResolvers.push({ owner: "example.emulator", extensions: ["nes"] });
     * host.updateFileTypes();
     * await host.settle();
     * assert.deepEqual(plugin.resolvers.fileTypes, ["nes"]);
     * ```
     */
    updateFileTypes(): void;
    /**
     * File opens waiting for this plugin's file resolver (`extensions`), by
     * request id: the path and how it was opened. Opening a matching path
     * (`player.open`, {@link TestHost.openElsewhere}) sends
     * `source.resolveRequest`; the answer selects the stream, fails the open
     * or, once the source changed, is `stale`. Any later source change
     * cancels it (`source.resolveCancel`). Web resolvers are not simulated:
     * emit `source.resolveRequest` yourself.
     */
    readonly resolutions: Map<number, {
        /** The id of this plugin's resolver asked (`resolvers.add` `id`). */
        resolverId: string;
        /** The path being opened, as sent to `resolve`. */
        source: string;
        /** The open asked for a paused start (`player.open(path, { paused })`). */
        paused: boolean;
    }>;
    /**
     * Answers for `dialogs.open` (paths, `null` = cancelled) and
     * `dialogs.confirm` (`{ confirmed, checked }`), consumed in order. A
     * dialog with no queued answer, or with the wrong kind, fails the request
     * (as a lost reply).
     *
     * For `openFile` / `openDirectory` / `saveFile` queue a one-path array;
     * for `openFiles` any number of paths.
     *
     * @example
     * ```ts
     * host.dialogAnswers.push(["/media/movie.mkv"]); // openFile picks it
     * host.dialogAnswers.push(null); // the next file dialog is cancelled
     * host.dialogAnswers.push({ confirmed: true, checked: false }); // confirm
     * await host.command("open").run();
     * ```
     */
    readonly dialogAnswers: (string[] | null | {
        /** The user clicked the confirm button. */
        confirmed: boolean;
        /** The checkbox state. */
        checked: boolean;
    })[];
    /** Every `dialogs.confirm` request (its options), newest last. */
    readonly confirms: Record<string, Json>[];
    /**
     * The main window as the player would show it. Change it with
     * `setMainWindow` (the user resized it, entered fullscreen, closed the
     * picture-in-picture window); `onTop`, `cursorAutoHide` and
     * `pictureInPicture.active` record the plugin's `window.set…` calls.
     * `pictureInPicture.supported: false` stands for Windows and Linux.
     */
    readonly mainWindow: {
        /** Whether the main window is fullscreen. */
        fullscreen: boolean;
        /** The video area in logical points (default 780×609). */
        videoArea: {
            /** Width in logical points. */
            width: number;
            /** Height in logical points. */
            height: number;
        };
        /** The effective `window.setAlwaysOnTop` value. */
        onTop: boolean;
        /** The effective `window.setCursorAutoHide` mode. */
        cursorAutoHide: "fullscreen" | "playing";
        /** Picture-in-picture: `supported` (preset) and `active`. */
        pictureInPicture: {
            /** Set to `false` to simulate Windows or Linux. */
            supported: boolean;
            /** Whether the picture-in-picture window shows the video. */
            active: boolean;
        };
    };
    /**
     * The player's secret store for this plugin: name → value. Preset entries
     * to simulate stored secrets; inject failures with
     * `host.handle("secrets.get", ...)`. Like the player, `secrets.set` of a
     * new name fails `limit` once 256 names are stored.
     */
    readonly secrets: Map<string, string>;
    /**
     * `plugin.storage.session`: key → value, as the player keeps them. Preset
     * entries to simulate values an earlier run of the plugin stored in the
     * same player session, or pass `startPlugin(..., { sessionStorage })`.
     * Like the player, a key is 1–1024 bytes, a `set` that would make the
     * entries larger than 1 MiB (keys, values as compact JSON and
     * `SESSION_STORAGE_LIMITS.entryBytes` per key) fails `limit`, and an
     * object's properties are stored sorted, as the player hands them back.
     *
     * @since 1.5.25
     */
    readonly sessionStorage: Map<string, Json>;
    /** What `plugin.app.system()` reports. */
    systemInfo: WireSystemInfo;
    /**
     * What `plugin.app` reports. Set it before {@link startPlugin}: the plugin
     * receives it once at start-up.
     */
    app: {
        /** The player version. */
        version: string;
        /** The short git hash of the build, `null` when unknown. */
        build: string | null;
        /** Component versions. */
        components: {
            /** libmpv's `mpv-version`. */
            mpv: string | null;
            /** `{ ffmpeg: <version> }`. */
            libav: Record<string, string>;
            /** The Slint version. */
            slint: string;
        };
    };
    /** What `player.chapters()` lists while media is ready. */
    chapters: {
        /** The chapter title, `null` without one. */
        title: string | null;
        /** Start time in seconds. */
        start: number;
    }[];
    /**
     * `plugin.dataDirectory`: a fresh empty temporary directory, removed with
     * the host on `stop()`. `plugin.storage.json` reads and writes real files
     * here. Pass `startPlugin(..., { dataDirectory })` to use another one.
     */
    readonly dataDirectory: string;
    /**
     * The manifest id of the plugin under test (set by {@link startPlugin};
     * `"test.plugin"` by default).
     */
    manifestId: string;
    constructor();
    /** The `player.setStepSeek` value in effect (default 5). */
    stepSeek: number;
    /** The `player.setStepSeek` `keyframes` in effect (default `false`, SDK 1.5.19). */
    stepSeekKeyframes: boolean;
    /**
     * Test data: the keyframe times (seconds, ascending) of the current
     * source, for keyframe step seeks. `null` (the default): none modelled,
     * and step seeks are exact even with `stepSeekKeyframes`.
     *
     * @example
     * ```ts
     * host.setSource({ phase: "ready", duration: 45, position: 12 });
     * host.keyframes = [0, 10, 20, 30, 40];
     * await plugin.player.setStepSeek({ seconds: 5, keyframes: true });
     * await plugin.host.run("seek-forward");
     * assert.equal(host.source.position, 20);
     * assert.equal(host.osd.at(-1), "Forward 5 s: 00:20 / 00:45");
     * ```
     */
    keyframes: number[] | null;
    /**
     * Every step seek the core ran (`seek-backward` / `seek-forward`, from a
     * key, the menu or `host.run`), newest last: the step, whether it went to
     * a keyframe, and where it landed.
     */
    readonly coreSeeks: {
        /** `seek-backward` or `seek-forward`. */
        action: string;
        /** The step: `host.run`'s `seconds`, else `setStepSeek`'s. */
        seconds: number;
        /** It landed on a keyframe (`stepSeekKeyframes` and keyframes modelled). */
        keyframes: boolean;
        /** The new position. */
        landed: number;
    }[];
    /**
     * Every seek a plugin submitted (`player.seek`, `Scrub.seek`), newest
     * last, with the mode it asked for. The test host lands on the exact
     * target either way (the player lands a `keyframes` seek on the last
     * keyframe at or before it); the core's step seeks are in `coreSeeks`.
     */
    readonly seeks: {
        /** The target. */
        seconds: number;
        /** `Scrub.seek(seconds, { keyframes: true })` (SDK 1.5.22). */
        keyframes: boolean;
    }[];
    /** Every `player.step`, newest last. */
    readonly steps: {
        /** `"frame"` or `"keyframe"`. */
        unit: string;
        /** `"backward"` or `"forward"`. */
        direction: string;
    }[];
    /** Every core action run (`host.run`, a core command, a pressed core binding), newest last. */
    readonly coreRuns: string[];
    /**
     * One of the 14 core actions (`play-pause`, `stop`, `seek-forward`, ...)
     * that the test host registers like the player, as `host/<id>`. Throws for
     * an unknown id. `run()` executes it with the player's checks and writes
     * its on-screen text to {@link TestHost.osd}.
     *
     * @example
     * ```ts
     * host.setSource({ phase: "ready", duration: 100, position: 10 });
     * await host.coreAction("play-pause").run();
     * assert.equal(host.source.paused, true);
     * assert.deepEqual(host.coreRuns, ["play-pause"]);
     * ```
     */
    coreAction(id: string): FakeCommand;
    /**
     * @internal The transport side the SDK calls: answer one request with the
     * override from {@link TestHost.handle} or the default behaviour.
     */
    request<T>(method: string, params?: Json): Promise<T>;
    /** @internal The transport side the SDK calls: listen to a host event. */
    on(event: string, listener: (data: unknown) => void): () => void;
    /**
     * Replace the host's behaviour for one wire method until
     * {@link TestHost.reset}. The handler receives the request parameters and
     * returns the reply (or a promise of it).
     *
     * Throw a {@link HostError} to simulate a definite refusal (the plugin's
     * call rejects with that `code`); throw a plain `Error` to simulate a lost
     * reply (the SDK then runs its recovery path). Method names are protocol
     * names, e.g. `"clipboard.writeText"`, `"secrets.get"`,
     * `"system.openPath"`, `"audio.addTrack"`; see {@link TestHost.sent} to
     * find the one a call uses.
     *
     * @param method - The wire method name.
     * @param handler - Produces the reply from the request parameters.
     *
     * @example
     * ```ts
     * host.handle("system.openPath", () => {
     *   throw new HostError("missing", "no such file");
     * });
     * await host.command("open-folder").run();
     * assert.deepEqual(host.osd, ["Folder not found"]);
     * host.reset("system.openPath");
     * ```
     */
    handle(method: string, handler: Handler): void;
    /** Restore the default behaviour of a method replaced with {@link TestHost.handle}. */
    reset(method: string): void;
    /**
     * Deliver a host event to the plugin. Like the player, subscription-based
     * events (`source.state`, `playback.backend`, `window.state`,
     * `timeline.changed`, `appearance.changed`) are delivered only when the
     * plugin subscribed (it declared `events` and listens). Most tests use the
     * higher-level helpers (`setSource`, `setMainWindow`, ...) instead.
     *
     * @param event - The wire event name.
     * @param data - The event payload (wire-level shape).
     */
    emit(event: string, data: unknown): void;
    /**
     * Change the source facts and broadcast them, as if the player's state
     * changed (the user paused, playback reached the end, a file finished
     * loading). Any phase other than `empty` also turns on the source
     * capabilities (pause, seek, rate, ...) unless the patch sets them.
     * `phase: "loading"` makes a mounted backdrop effective, like the player.
     *
     * @param patch - Wire-level source fields, e.g. `phase`, `position`,
     *   `duration`, `paused`, `audioTracks`.
     *
     * @example
     * ```ts
     * host.setSource({ phase: "ready", duration: 60, position: 0 });
     * await host.settle();
     * assert.equal(plugin.player.state.phase, "ready");
     * host.setSource({ phase: "ended", terminalReason: "natural" });
     * await host.settle();
     * ```
     */
    setSource(patch: Partial<SourceState>): void;
    /**
     * Simulate the user changing the main window (resizing it, entering or
     * leaving fullscreen, closing the picture-in-picture window) and broadcast
     * it. Like the player, entering fullscreen collapses an attached, visible
     * sidebar (`effective: false`, `reason: "fullscreen"`) and leaving
     * fullscreen docks it again.
     *
     * @example
     * ```ts
     * host.setMainWindow({ fullscreen: true, videoArea: { width: 1920, height: 1080 } });
     * await host.settle();
     * assert.equal(plugin.window.state.fullscreen, true);
     * host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
     * ```
     */
    setMainWindow(patch: Partial<{
        fullscreen: boolean;
        videoArea: {
            width: number;
            height: number;
        };
        pictureInPicture: {
            supported: boolean;
            active: boolean;
        };
    }>): void;
    /**
     * The pointer reached the right edge in fullscreen (`true`) or left the
     * popped-out sidebar (`false`): the attached sidebar pops out or collapses
     * (reference §6.3). No effect outside fullscreen, when hidden or floating.
     */
    popSidebar(popped: boolean): void;
    /**
     * Simulate another plugin (or the user) opening a source: a new
     * `sourceId`, then the given phase. Calls the plugin made against the old
     * state now fail with `stale`.
     *
     * @param source - The path or URL.
     * @param phase - The phase to end in (default `"ready"`).
     *
     * @example
     * ```ts
     * const before = plugin.player.state;
     * host.openElsewhere("/media/other.mp4");
     * await host.settle();
     * assert.equal(plugin.player.isCurrent(before), false);
     * ```
     */
    openElsewhere(source: string, phase?: SourceState["phase"]): void;
    /**
     * Simulate a generated stream failing in the player (its audio device
     * lost, its frame connection broken), like the player: the source fails
     * with `message` (`phase: "error"`) and the producer is retired at once.
     * The stream's `onStop` listeners get a `stream` error with `message`.
     *
     * @param producerId - The producer, from {@link TestHost.producers}.
     * @param message - The player's failure text.
     *
     * @example
     * ```ts
     * const [producer] = host.producers.values();
     * host.failStream(producer.id, "producer transport closed");
     * ```
     */
    failStream(producerId: number, message: string): void;
    /**
     * Press a binding like the host's table would: panel-focus bindings first
     * when `panel` is true, then global ones; within a scope the earliest
     * registration first (the player's own commands before the plugin's), as
     * the conflict rule orders them. A disabled command lets the key pass to
     * the next command bound to it (SDK 1.5.12, as player 0.4.3): the one it
     * shadowed, then the global scope. The first enabled one runs; `press`
     * resolves after it ran (`"ran"`), or rejects when it is busy. When only
     * disabled commands hold the key, the first one with a `reason` shows the
     * reason on the OSD and eats the key (`"reason"`, see `host.osd`);
     * without a reason the key goes on to the focused control (`"passed"`,
     * nothing runs). Rejects when nothing is bound. Bindings use the
     * reference syntax (`"Space"`, `"Ctrl+O"`, `"Mouse:WheelUp"`), and user
     * overrides in `bindings.overlay` apply.
     *
     * @returns What the key did: `"ran"`, `"reason"` or `"passed"`.
     *
     * @example
     * ```ts
     * await host.press("Ctrl+B"); // a global binding
     * await host.press("Delete", { panel: true }); // while the panel has focus
     * ```
     */
    press(binding: string, options?: {
        panel?: boolean;
    }): Promise<"ran" | "reason" | "passed">;
    /**
     * Bindings declared by more than one command, each as
     * `[binding, winner, ...ignored]` (command ids; core actions as
     * `host/<id>`). Empty when there are none.
     *
     * @example
     * ```ts
     * assert.deepEqual(host.conflicts, []);
     * ```
     */
    get conflicts(): string[][];
    /**
     * The binding overlay `plugin.bindings.set` writes (`"<plugin>/<command>"`
     * to inputs; preset it to simulate saved overrides). `press` honours it.
     */
    readonly bindings: {
        /** `"<plugin>/<command>"` → the user's inputs for it. */
        overlay: Map<string, string[]>;
    };
    /**
     * What `packages.list` reports; `setEnabled`, `uninstall` and `install`
     * update it. `builtin`, `source` and `dataDirectory` are optional here
     * (default `false`, `null`, `null`).
     */
    packages: {
        /** The plugin's manifest id. */
        id: string;
        /** Its version, `null` when unknown. */
        version: string | null;
        /** Where it was loaded from. */
        origin: "installed" | "development";
        /** Its run state. */
        state: "starting" | "running" | "disabled" | "failed" | "stopped";
        /** Why it failed (`state: "failed"`). */
        error: string | null;
        /** Its declared capabilities. */
        capabilities: string[];
        /** The package directory. */
        directory: string;
        /** Shipped with the player: `uninstall` and `install` are `unsupported`. */
        builtin?: boolean;
        /** Where it was installed from. */
        source?: PackageSource | null;
        /** Its data directory. */
        dataDirectory?: string | null;
    }[];
    /** Every `packages.setEnabled` / `uninstall` / `install`, newest last. */
    readonly packageCalls: {
        /** Which call. */
        method: "setEnabled" | "reload" | "uninstall" | "install";
        /** The target plugin id. */
        id: string;
        /** `setEnabled`'s value. */
        enabled?: boolean;
        /** `uninstall`'s `removeData` option. */
        removeData?: boolean;
        /** `install`'s source (`{ kind: "file" }` when not given). */
        source?: PackageSource;
        /** `install`'s `replace` option. */
        replace?: boolean;
    }[];
    /**
     * Packages `validate` and `validateArchive` accept: path (directory or
     * `.vxp` file) → its manifest. Other paths fail with `missing`. `install`
     * adds or upgrades the entry in {@link TestHost.packages} following the
     * player's source rules: the same source upgrades, another source needs
     * `replace` (`conflict` otherwise), a built-in id is `unsupported`.
     *
     * @example
     * ```ts
     * host.candidates.set("/downloads/someone.demo-1.0.0.vxp", {
     *   id: "someone.demo", version: "1.0.0", apiVersion: "vestra/1",
     *   entry: "main.js", capabilities: [],
     * });
     * ```
     */
    readonly candidates: Map<string, PackageManifest>;
    /**
     * Remote images (`plugin.ui.markdown`, `plugin.ui.remoteImage`): address
     * → a local PNG or JPEG file served for it, or `null` for a failing
     * download. Nothing is fetched from the network in tests; unknown
     * addresses fail.
     */
    readonly remoteImages: Map<string, string | null>;
    /**
     * The Markdown document a value holds (its wire form read back), or
     * `null` when the value is not one. `images` maps each remote address to
     * the downloaded file. Pass a whole property value or a field of one: a
     * document nested in rows is read the same way.
     *
     * @example
     * ```ts
     * const rows = host.window("chat").props["messages"] as { body: Json }[];
     * assert.equal(host.markdown(rows[0]!.body)?.text, "**Hello**");
     * ```
     */
    markdown(value: Json | undefined): MarkdownWire | null;
    /**
     * What `player.audioDevices()` lists. `setAudioDevice` updates
     * `host.source.audioOutput`.
     */
    audioDevices: {
        /** The device id, e.g. `coreaudio/<UID>`. */
        id: string;
        /** The system's name for it. */
        name: string;
    }[];
    /**
     * Replace the plugin list and push `packages.changed` (another plugin
     * started, stopped, was installed or removed).
     */
    setPackages(list: TestHost["packages"]): void;
    /**
     * Simulate another plugin calling `player.next()` / `previous()`: the host
     * asks this plugin's sequence provider. Rejects with `unavailable` when
     * there is no provider or its flag is false.
     */
    step(direction: "next" | "previous"): Promise<unknown>;
    /**
     * The plugin's command with this local id. Throws when it is not
     * registered (never added, or disposed).
     */
    command(id: string): FakeCommand;
    /**
     * The plugin's tool window with this id (by default the file name without
     * extension: `ui/picture.slint` → `picture`). Throws when it was never
     * opened with `show()`.
     *
     * @example
     * ```ts
     * host.window("picture").emit("reset");
     * await host.settle();
     * assert.equal(host.window("picture").props["brightness"], 0);
     * ```
     */
    window(id: string): FakeWindow;
    /**
     * The plugin's mounted panel (bottom bar, sidebar or backdrop). Throws when
     * none is mounted. Its `props` use Slint (kebab-case) names; `emit` fires a
     * callback on the panel's global.
     *
     * @example
     * ```ts
     * const panel = host.mounted();
     * assert.equal(panel.slot, "sidebar");
     * panel.emit("select", 2);
     * await host.settle();
     * ```
     */
    mounted(): Omit<FakeWindow, "callbacks" | "userClose"> & {
        /** `"bottom"`, `"sidebar"` or `"backdrop"`. */
        slot: string;
        /** The exported component the SDK found in the file. */
        component: string;
        /** The last `panel.setInteraction(...)` locks, `null` before any. */
        locks: Json;
    };
    /**
     * The parameters of every request of one wire method, oldest first.
     *
     * @example
     * ```ts
     * assert.deepEqual(host.sent("osd.show").at(-1), { text: "Saved" });
     * ```
     */
    sent(method: string): Record<string, Json>[];
    /**
     * Wait until no request is in flight (and the SDK's follow-up passes ran),
     * and at least `ms` milliseconds have passed so that timers due by then
     * fired. Call it after every action that makes the plugin talk to the host,
     * before asserting.
     *
     * It only waits for host calls. Work the plugin does on its own (reading
     * or parsing a file, a child process, a network request) is not waited
     * for: on a loaded machine it can finish after `settle()` returns. Poll
     * the result you assert on instead (second example).
     *
     * @param ms - Extra time to wait, for debounces and timers (default 0).
     *
     * @example
     * ```ts
     * store.save({ step: 10 });
     * await host.settle(400); // past storage.json's 300 ms debounce
     * ```
     *
     * @example
     * ```ts
     * window.emit("load-file"); // the plugin reads and parses the file itself
     * for (let i = 0; i < 100 && window.props["status-text"] !== expected; i++)
     *   await host.settle(20);
     * assert.equal(window.props["status-text"], expected);
     * ```
     */
    settle(ms?: number): Promise<void>;
    /**
     * Close the fake stream servers and delete the host's temporary
     * directories. `stop()` from {@link startPlugin} calls it.
     */
    close(): void;
}
/**
 * Activate a plugin against a test host, the way the player's runner does:
 * build the `plugin` object, call `activate(plugin)`, wait for every
 * registration started during activation (commands, panels, resolvers, ...),
 * then wait until the host is idle.
 *
 * The manifest id and capabilities come from `directory/manifest.json`
 * unless `capabilities` is given; then `id` (default `"test.plugin"`) and
 * `capabilities` are used and no manifest is read. Calls that need an
 * undeclared capability fail with `capability`, as in the player.
 *
 * Rejects when `manifest.json` cannot be read or parsed, when `activate`
 * throws, or when a registration made during activation fails (for example
 * `limit`, `conflict`, `invalid`), which is when the player would fail the
 * activation.
 *
 * @param activate - The plugin's default-exported `activate` function.
 * @param options - Where the plugin lives and how to start it.
 * @returns `host`: the {@link TestHost}; `plugin`: the object passed to
 *   `activate` (call the plugin API on it directly); `stop()`: dispose the
 *   plugin like the player stopping it (scopes released, stores flushed),
 *   wait until the host is idle and remove the host's temporary files.
 *
 * @example
 * ```ts
 * test("remembers the step across restarts", async () => {
 *   const directory = new URL("..", import.meta.url);
 *   const first = await startPlugin(activate, { directory });
 *   await first.host.command("step-10").run();
 *   const data = mkdtempSync(join(tmpdir(), "data-"));
 *   cpSync(first.host.dataDirectory, data, { recursive: true });
 *   await first.stop();
 *
 *   const second = await startPlugin(activate, { directory, dataDirectory: data });
 *   assert.equal(second.host.command("step-10").state.checked, true);
 *   await second.stop();
 * });
 * ```
 */
export declare function startPlugin(activate: (plugin: Plugin) => unknown, options?: {
    /**
     * The plugin's package directory, as a path or a `file:` URL (e.g.
     * `new URL("..", import.meta.url)` from `tests/`). Relative `.slint`
     * files resolve against it. Default: the current working directory.
     */
    directory?: string | URL;
    /** The manifest id when `capabilities` is given. Default `"test.plugin"`. */
    id?: string;
    /** Use these capabilities instead of reading `manifest.json`. */
    capabilities?: string[];
    /** A host to use (to preset data before activation). Default: a new one. */
    host?: TestHost;
    /** Use an existing directory as `plugin.dataDirectory` (to test a restart). */
    dataDirectory?: string;
    /**
     * Start with these `plugin.storage.session` values (to test a restart
     * within one player session): pass the previous run's
     * `host.sessionStorage`. They are copied into the new host's
     * {@link TestHost.sessionStorage}.
     *
     * @since 1.5.25
     */
    sessionStorage?: ReadonlyMap<string, Json>;
    /**
     * The interface language to play ({@link TestHost.locale}): the plugin's
     * `locales/<locale>.po` (or `.json`) in `directory` is loaded and its
     * menus, messages and titles show translated. Default: the host's.
     *
     * @since 1.5.9
     */
    locale?: Locale;
}): Promise<{
    /** The test host the plugin talks to. */
    host: TestHost;
    /** The `plugin` object passed to `activate`. */
    plugin: Plugin;
    /** Stop the plugin, wait for the host, and remove temporary files. */
    stop(): Promise<void>;
}>;
