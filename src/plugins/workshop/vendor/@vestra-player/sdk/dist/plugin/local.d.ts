import type { LocalServices } from "../wire.ts";
/**
 * The file names a tool can have: `ffmpeg` on macOS and Linux; on Windows
 * `ffmpeg` with each `PATHEXT` extension (`ffmpeg.exe` first by default).
 */
export declare function executableNames(tool: string, platform?: NodeJS.Platform, pathext?: string | undefined): string[];
export declare const systemServices: LocalServices;
