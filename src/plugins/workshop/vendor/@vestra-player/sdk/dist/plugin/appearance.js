/** The player's palette (`plugin.appearance`; `ui.theme` to change it). */
import { failure } from "./runtime.js";
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
    #started = false;
    constructor(runtime) {
        this.#runtime = runtime;
        this.theme =
            windowOf(runtime).appearance ?? "vestra";
        this.language = preference(windowOf(runtime).language) ?? "system";
    }
    start() {
        if (this.#started)
            return;
        this.#started = true;
        this.#runtime.on("appearance.changed", (data) => {
            const event = data;
            const language = preference(event?.language);
            if (language && language !== this.language) {
                this.language = language;
                this.#runtime.invalidate();
            }
            const theme = event?.theme;
            if (theme && THEMES.includes(theme))
                this.accept(theme);
        });
    }
    accept(theme) {
        if (theme === this.theme)
            return;
        this.theme = theme;
        for (const listener of [...this.#listeners])
            listener(theme);
        this.#runtime.invalidate();
    }
    watch(listener) {
        this.#listeners.add(listener);
        return { dispose: async () => void this.#listeners.delete(listener) };
    }
}
const appearanceOf = (runtime) => runtime.service("appearance", () => new AppearanceService(runtime));
export function appearance(scope) {
    const service = () => appearanceOf(scope.runtime);
    return {
        get theme() {
            service().start();
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
