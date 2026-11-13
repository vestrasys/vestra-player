export declare const MANIFEST_LIMITS: Readonly<{
    idBytes: 128;
    name: 64;
    author: 64;
    punchline: 120;
    description: 2000;
    tags: 8;
    tag: 24;
    license: 64;
    urlBytes: 2048;
    iconBytes: number;
    iconSide: 1024;
    documentBytes: number;
}>;
/** Every field a manifest may have; anything else is rejected by the player. */
export declare const MANIFEST_FIELDS: readonly string[];
export declare const validVersion: (value: unknown) => value is string;
/**
 * Why a manifest with `capabilities` is refused (SDK 1.9.0, player 0.9.0):
 * the field is gone, and what it switched on is two calls.
 */
export declare const CAPABILITIES_REMOVED = "manifest.json has \"capabilities\", which SDK 1.9.0 (player 0.9.0) removed: delete the field. What it switched on is now a call, first in activate(): plugin.player.subscribe() for \"events\", await plugin.player.read() for \"playback.read\"; everything else needs nothing (the store shows what the code uses, from its scan)";
/** Problems with `manifest` (empty: valid) and hints that do not fail. */
export declare function manifestProblems(manifest: Record<string, unknown>, directory: string): {
    problems: string[];
    hints: string[];
};
/** Width and height of a PNG or JPEG, or null for anything else. */
export declare function imageSize(bytes: Uint8Array): {
    width: number;
    height: number;
} | null;
