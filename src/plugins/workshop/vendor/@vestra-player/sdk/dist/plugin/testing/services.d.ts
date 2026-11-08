import type { Json } from "../../wire.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** A `plugin.storage.session` key, checked as the player does. */
export declare function sessionKey(key: Json | undefined): string;
/** Code point (UTF-8 byte) order, as the player sorts session keys. */
export declare const byCodePoint: (a: string, b: string) => 0 | 1 | -1;
/**
 * A JSON value as the player hands it back: the player does not keep the
 * order of object properties, it sorts them.
 */
export declare function sortedJson(value: Json): Json;
/** What one session entry counts towards the player's quota. */
export declare const sessionCost: (key: string, value: Json) => number;
export declare function validatePackage(host: TestHost, _inner: Internals, path: string): {
    token: string;
    digest: string;
    apiVersion: string;
    manifest: import("../types.ts").PackageManifest;
    checks: string[];
    trustedExecution: boolean;
};
/** Implements `TestHost.drop`. */
export declare function drop(host: TestHost, _inner: Internals, paths: string[], options?: {
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
/** Implements `TestHost.setMainWindow`. */
export declare function setMainWindow(host: TestHost, inner: Internals, patch: Partial<{
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
}>): void;
/** The default handlers of this area's wire methods, by method name. */
export declare function servicesHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
