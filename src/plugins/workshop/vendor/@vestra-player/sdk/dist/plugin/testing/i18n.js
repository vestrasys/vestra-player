/**
 * Texts as the player shows them: translation with the plugin's catalog, the
 * player's own texts, menu display and the appearance and language settings.
 *
 * Part of `TestHost` (`./index.ts`): functions over the host and its private
 * state (`Internals`, `./internals.ts`) that the class delegates to.
 * Internal to the SDK; not exported from `@vestra-player/sdk/testing`.
 */
import { HostError } from "../../wire.js";
import { THEMES } from "../appearance.js";
import { format as formatTemplate, HOST_GROUPS, HOST_ZH_HANS, LOCALES, isTextValue, menuKeys, renderTemplateArgs, renderText, resolveTexts, textKey, textProblem, } from "../../i18n.js";
/** Texts only the test host uses, with their Chinese of SDK 1.5.8. */
export const TEST_ZH_HANS = {
    "this site": "这个网站",
    "External SRT": "外部 SRT",
    "External ASS": "外部 ASS",
};
/**
 * A text of a request as the player shows it, refused as the player
 * refuses it: a string is a template (formatted with `args`, if given), a
 * text value carries its own arguments.
 */
export function shownText(host, _inner, text, args, what) {
    if (args !== undefined && args !== null) {
        const problem = textProblem(args, "args");
        if (problem)
            throw new HostError("invalid", problem);
    }
    if (typeof text === "string")
        return host.translate(text, (args ?? undefined));
    if (!isTextValue(text))
        throw new HostError("invalid", `${what} requires a string or a text`);
    const problem = textProblem(text);
    if (problem)
        throw new HostError("invalid", `${what}: ${problem}`);
    if (args !== undefined && args !== null)
        throw new HostError("invalid", `${what}: a text value takes no args`);
    return host.translate(text);
}
/** A declared text: a string is translated as it is, a text rendered. */
export function declaredText(host, inner, text, what) {
    return shownText(host, inner, text, undefined, what);
}
/** Property values with their text values shown, as the player does first. */
export function shownValues(host, inner, values) {
    const resolved = resolveTexts(values, inner.catalog, host.locale);
    if ("problem" in resolved)
        throw new HostError("invalid", resolved.problem);
    return resolved.value;
}
/** A text of the player itself, in the player's language (see `locale`). */
export function hostText(_host, inner, text, args) {
    const zh = inner.locale === undefined || inner.locale === "zh-Hans";
    const shown = (zh && (HOST_ZH_HANS[text] ?? TEST_ZH_HANS[text])) || text;
    return args === undefined
        ? shown
        : formatTemplate(shown, args);
}
/** A declared menu path and title as the player shows them. */
export function commandDisplay(host, inner, path, title, reason = null, category = null) {
    const keys = menuKeys(path.map((segment) => typeof segment === "string" ? segment : textKey(segment)));
    const display = {
        title: host.translate(title),
        // The player's submenus by key, everything else as declared.
        path: keys.map((segment, index) => index < keys.length - 1 &&
            HOST_GROUPS.includes(keys.slice(0, index + 1).join("/"))
            ? hostText(host, inner, segment)
            : host.translate((path[index] ?? segment))),
    };
    // Not enumerable: `deepEqual(display, { title, path })` keeps passing.
    Object.defineProperty(display, "reason", {
        value: reason === null ? null : host.translate(reason),
        enumerable: false,
    });
    Object.defineProperty(display, "category", {
        value: category === null ? null : host.translate(category),
        enumerable: false,
    });
    return display;
}
/** Implements `TestHost.translate`. */
export function translate(host, inner, text, args) {
    if (typeof text !== "string")
        return renderText(text, inner.catalog, host.locale);
    const values = renderTemplateArgs(args, inner.catalog, host.locale);
    return inner.catalog.template(text, values ?? null);
}
/** The default handlers of this area's wire methods, by method name. */
export function i18nHandlers(host, _inner) {
    return {
        "appearance.setLanguage": (p) => {
            const language = String(p.language);
            if (language !== "system" &&
                !LOCALES.includes(language))
                throw new HostError("invalid", `unknown language ${language}`);
            if (language !== host.language)
                host.setLanguage(language);
        },
        "appearance.set": (p) => {
            // Like the player: an id it does not have selects the default (the
            // SDK's own `set` refuses one before it gets here).
            const theme = THEMES.find((t) => t === p.theme) ?? THEMES[0];
            if (theme !== host.appearance)
                host.setAppearance(theme);
        },
    };
}
