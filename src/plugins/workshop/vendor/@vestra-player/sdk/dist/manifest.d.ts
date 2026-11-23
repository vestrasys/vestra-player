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
