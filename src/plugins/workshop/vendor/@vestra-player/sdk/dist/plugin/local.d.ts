import type { LocalServices } from "../wire.ts";
/**
 * The file names a tool can have: `ffmpeg` on macOS and Linux; on Windows
 * `ffmpeg` with each `PATHEXT` extension (`ffmpeg.exe` first by default).
 */
export declare function executableNames(tool: string, platform?: NodeJS.Platform, pathext?: string | undefined): string[];
/**
 * Where a local stream listens: a Unix socket file `<name>.sock` in
 * `directory` on macOS and Linux; on Windows a named pipe
 * (`\\.\pipe\<directory's name>-<name>`), which Node's `net` listens on and
 * connects to by that path (a socket file there fails with `EACCES`).
 */
export declare function localSocketPath(directory: string, name: string, platform?: NodeJS.Platform): string;
export declare const systemServices: LocalServices;
