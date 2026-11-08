import type { TemplateArgs, TextValue } from "../types.ts";
import type { TextJson } from "../../i18n.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** Texts only the test host uses, with their Chinese. */
export declare const TEST_ZH_HANS: Readonly<Record<string, string>>;
/**
 * A text of a request as the player shows it, refused as the player
 * refuses it: a string is a template (formatted with `args`, if given), a
 * text value carries its own arguments.
 */
export declare function shownText(host: TestHost, _inner: Internals, text: unknown, args: unknown, what: string): string;
/** A declared text: a string is translated as it is, a text rendered. */
export declare function declaredText(host: TestHost, inner: Internals, text: unknown, what: string): string;
/** Property values with their text values shown, as the player does first. */
export declare function shownValues<T>(host: TestHost, inner: Internals, values: T): T;
/** A text of the player itself, in the player's language (see `locale`). */
export declare function hostText(_host: TestHost, inner: Internals, text: string, args?: TemplateArgs): string;
/** A declared menu path and title as the player shows them. */
export declare function commandDisplay(host: TestHost, inner: Internals, path: readonly (string | TextJson)[], title: string | TextJson, reason?: TextValue | null, category?: string | TextJson | null): {
    title: string;
    path: string[];
    reason: string | null;
};
/** Implements `TestHost.translate`. */
export declare function translate(host: TestHost, inner: Internals, text: TextValue, args?: TemplateArgs): string;
/** The default handlers of this area's wire methods, by method name. */
export declare function i18nHandlers(host: TestHost, _inner: Internals): Record<string, Handler>;
