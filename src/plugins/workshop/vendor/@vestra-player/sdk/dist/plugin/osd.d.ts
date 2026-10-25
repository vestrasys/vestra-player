import type { ScopeImpl } from "./scope.ts";
import type { Osd } from "./types.ts";
import type { TemplateArgsJson, TextValueJson } from "../wire.ts";
/**
 * Copy and check template arguments: an array or object of at most 16
 * strings, finite numbers, texts and lists (`TEXT_LIMITS`).
 */
export declare function templateArgs(args: unknown): TemplateArgsJson | undefined;
/**
 * Check a shown text (`osd.show`, dialogs): source text, or a text value
 * that carries its own arguments (then `args` is not given).
 */
export declare function shownText(text: unknown, args: unknown, what?: string): TextValueJson;
export declare const osd: (scope: ScopeImpl) => Osd;
