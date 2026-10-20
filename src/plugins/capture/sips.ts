import { execFile } from "node:child_process";

/**
 * F07: PNG → JPEG (quality 1–100) or BMP with macOS `sips`, never through a
 * shell, with a timeout. Only this plugin converts images, so it stays here
 * (round 2 task list §4); tests replace `sips.convert`. The caller checks the
 * output exists: sips reports some failures only on stderr with status 0.
 */
export const sips = {
  convert(png: string, target: string, format: "jpeg" | "bmp", quality: number): Promise<void> {
    const options = format === "jpeg" ? ["-s", "format", "jpeg", "-s", "formatOptions", String(quality)] : ["-s", "format", "bmp"];
    return new Promise<void>((resolve, reject) => {
      execFile("/usr/bin/sips", [...options, png, "--out", target], { shell: false, timeout: 30_000, encoding: "utf8" }, (error, _stdout, stderr) => {
        if (!error) return resolve();
        console.error(`[capture] sips failed: ${(stderr || error.message).trim()}`);
        reject(error);
      });
    });
  },
};
