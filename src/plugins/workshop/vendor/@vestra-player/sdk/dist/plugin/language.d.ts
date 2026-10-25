import type { ScopeImpl } from "./scope.ts";
import type { I18n, ListStyle, TemplateArgs, Text, TextArg, TextError, TextList } from "./types.ts";
/** `plugin.i18n.text`: a plain, frozen text value (no translation here). */
export declare function text(source: string, args?: TemplateArgs): Text;
/** `plugin.i18n.list`: a plain, frozen list value. */
export declare function list(items: readonly TextArg[], style?: ListStyle): TextList;
export declare const error: (source: string, args?: TemplateArgs) => TextError;
export declare const i18n: (scope: ScopeImpl) => I18n;
