import type { ScopeImpl } from "./scope.ts";
import type { Osd } from "./types.ts";
import type { TemplateArgsJson } from "../wire.ts";
/** Copy and check template arguments: an array or object of ≤ 16 strings and numbers. */
export declare function templateArgs(args: unknown): TemplateArgsJson | undefined;
export declare const osd: (scope: ScopeImpl) => Osd;
