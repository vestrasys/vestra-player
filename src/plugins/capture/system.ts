import { execFile } from "node:child_process";

/**
 * The macOS commands this plugin runs (doc/product/feature-requirements.md
 * appendix B.4): never through a shell, always with a timeout. Kept in one
 * place so a later core clipboard / image API can replace them; tests replace
 * the members of `system` with fakes.
 */
export const system = {
  /** Open a file or folder with the system opener (F09, 打开输出). */
  reveal(path: string): Promise<void> {
    const program = process.platform === "darwin" ? "/usr/bin/open" : process.platform === "win32" ? "explorer.exe" : "xdg-open";
    const args = process.platform === "darwin" ? ["--", path] : [path];
    return run(program, args, 10_000).catch(() => {
      throw new Error("无法打开文件夹。");
    });
  },
  /** Put a PNG file on the clipboard as an image (F03). */
  copyImage(png: string): Promise<void> {
    const literal = png.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    return run("/usr/bin/osascript", ["-e", `set the clipboard to (read (POSIX file "${literal}") as «class PNGf»)`], 10_000);
  },
  /** Convert a PNG to JPEG (quality 1–100) or BMP with `sips` (F07); the caller checks the output exists. */
  convertImage(png: string, target: string, format: "jpeg" | "bmp", quality: number): Promise<void> {
    const options = format === "jpeg" ? ["-s", "format", "jpeg", "-s", "formatOptions", String(quality)] : ["-s", "format", "bmp"];
    return run("/usr/bin/sips", [...options, png, "--out", target], 30_000);
  },
};

function run(program: string, args: string[], timeout: number) {
  return new Promise<void>((resolve, reject) => {
    execFile(program, args, { shell: false, timeout, encoding: "utf8" }, (error, _stdout, stderr) => {
      if (!error) return resolve();
      console.error(`[capture] ${program} failed: ${(stderr || error.message).trim()}`);
      reject(error);
    });
  });
}
