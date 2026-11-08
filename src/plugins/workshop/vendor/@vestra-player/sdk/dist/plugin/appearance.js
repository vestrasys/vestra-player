/** The player's palette (`plugin.appearance`; `ui.theme` to change it). */
import { failure } from "./errors.js";
import { windowOf } from "./window.js";
import { LOCALES } from "../i18n.js";
const preference = (value) => value === "system" || LOCALES.includes(value)
    ? value
    : undefined;
/** The palettes this SDK knows, the default first. */
export const THEMES = ["vestra", "vestra-light"];
/**
 * One per plugin: the value comes with the `window.state` read before
 * activation; with `events`, `appearance.changed` keeps it current once
 * the plugin uses it.
 */
class AppearanceService {
    #runtime;
    theme;
    /** The saved interface language (the next start's). */
    language;
    #listeners = new Set();
    /**
     * The theme listeners last heard (or the last event's), not the one read
     * at start: a plugin woken from standby reads the new theme from
     * `window.state`, then gets the `appearance.changed` the player held for
     * it, and its listeners still hear that change. Left unset until the
     * first event on purpose: set from the start-up read, it would hide that
     * held change again.
     */
    #notified;
    /**
     * The language as the start-up read and later events carried it (not
     * `language`, which the plugin's own `setLanguage` changes before its
     * echo arrives). The player sends `appearance.changed` once per change
     * of the theme or of the language, each with both values: an event whose
     * language differs from this one is a language change, and its theme is
     * compared with `theme` as before. Only a newest event that repeats the
     * start-up language is taken for a theme change; a plugin woken for a
     * language change gets that too, so its listeners may hear the unchanged
     * theme once (the event does not say what changed).
     */
    #eventLanguage;
    #started = false;
    constructor(runtime) {
        this.#runtime = runtime;
        this.theme =
            windowOf(runtime).appearance ?? "vestra";
        this.language = preference(windowOf(runtime).language) ?? "system";
        this.#eventLanguage = this.language;
    }
    start() {
        if (this.#started)
            return;
        this.#started = true;
        // A mirror: a plugin listens through `watch` and bindings.
        this.#runtime.on("appearance.changed", (data) => {
            const event = data;
            const language = preference(event?.language);
            // Older players send no language: their events are theme changes.
            const languageChanged = language !== undefined && language !== this.#eventLanguage;
            if (language)
                this.#eventLanguage = language;
            if (language && language !== this.language) {
                this.language = language;
                this.#runtime.invalidate();
            }
            const theme = event?.theme;
            if (!theme || !THEMES.includes(theme))
                return;
            if (languageChanged) {
                // The theme came along unchanged (or this plugin set it already).
                this.accept(theme);
                this.#notified = theme;
            }
            else
                this.changed(theme);
        }, "events", true);
    }
    /** A theme change event: listeners hear it unless they already did. */
    changed(theme) {
        const changed = theme !== this.theme;
        this.theme = theme;
        if (theme !== this.#notified)
            this.#notify(theme);
        if (changed)
            this.#runtime.invalidate();
    }
    /** After this plugin's own `set`: only a change from the current theme. */
    accept(theme) {
        if (theme === this.theme)
            return;
        this.theme = theme;
        this.#notify(theme);
        this.#runtime.invalidate();
    }
    #notify(theme) {
        this.#notified = theme;
        for (const listener of [...this.#listeners])
            listener(theme);
    }
    watch(listener) {
        this.#listeners.add(listener);
        const held = this.#runtime.hold("appearance.changed");
        return {
            dispose: async () => {
                this.#listeners.delete(listener);
                await held.dispose();
            },
        };
    }
}
const appearanceOf = (runtime) => runtime.service("appearance", () => new AppearanceService(runtime));
export function appearance(scope) {
    const service = () => appearanceOf(scope.runtime);
    return {
        get theme() {
            service().start();
            scope.runtime.reading("appearance.changed");
            return service().theme;
        },
        onChange(listener) {
            service().start();
            const watch = service().watch((theme) => scope.run(listener, theme));
            return scope.add({
                dispose: async () => {
                    scope.forget(watch);
                    await watch.dispose();
                },
            });
        },
        async set(theme) {
            if (!THEMES.includes(theme))
                throw failure("invalid", `theme is ${THEMES.map((t) => `"${t}"`).join(" or ")}`);
            await scope.runtime.host.appearance.set(theme);
            service().accept(theme);
        },
        get language() {
            service().start();
            scope.runtime.reading("appearance.changed");
            return service().language;
        },
        async setLanguage(language) {
            if (!preference(language))
                throw failure("invalid", `language is "system" or one of ${LOCALES.map((l) => `"${l}"`).join(", ")}`);
            await scope.runtime.host.appearance.setLanguage(language);
            if (service().language !== language) {
                service().language = language;
                scope.runtime.invalidate();
            }
        },
    };
}
