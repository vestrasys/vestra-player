import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
export declare function coreFacts(host: TestHost, _inner: Internals): {
    ready: boolean;
    ended: boolean;
    toggle: boolean;
    step: boolean;
    rate: boolean;
    close: boolean;
};
/** Why one frame that way is impossible (B07), or null; the player's rules. */
export declare function frameCheck(host: TestHost, inner: Internals, forward: boolean): string | null;
/** The same enabled rules and reasons as the player's core actions. */
export declare function coreState(host: TestHost, inner: Internals, action: string): {
    enabled: boolean;
    checked: boolean | null;
    busy: boolean;
    reason: string | null;
};
export declare function registerCoreActions(host: TestHost, inner: Internals): void;
export declare function runCore(host: TestHost, inner: Internals, action: string, seconds?: number): void;
export declare function windowFacts(host: TestHost, _inner: Internals): {
    fullscreen: boolean;
    maximized: boolean;
    videoArea: {
        width: number;
        height: number;
    };
    pictureInPicture: {
        supported: boolean;
        active: boolean;
    };
    appearance: import("../types.ts").ThemeId;
    fileTypes: string[];
    language: import("../types.ts").LanguagePreference;
    locale: import("../types.ts").Locale;
};
/** The default handlers of this area's wire methods, by method name. */
export declare function coreActionsHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
