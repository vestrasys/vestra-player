import { type ChildProcess, execFile, spawn } from "node:child_process";

/**
 * The macOS commands this plugin runs (doc/product/feature-requirements.md
 * appendix B.4): never through a shell, always with a timeout. Tests replace
 * the members of `system` with fakes.
 */
export const system = {
  /** First lines of the clipboard text; empty on any failure (A02). */
  readClipboard(): Promise<string> {
    return new Promise((resolve) => {
      execFile("/usr/bin/pbpaste", [], { timeout: 1000, encoding: "utf8", env: utf8Env() }, (error, stdout) =>
        resolve(error ? "" : stdout),
      );
    });
  },
  /** Put `text` on the clipboard (I08). */
  writeClipboard(text: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const child = spawn("/usr/bin/pbcopy", [], { shell: false, stdio: ["pipe", "ignore", "ignore"], env: utf8Env() });
      const timer = setTimeout(() => {
        child.kill();
        reject(new Error("超时"));
      }, 2000);
      child.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
      child.once("close", (code) => {
        clearTimeout(timer);
        if (code === 0) resolve();
        else reject(new Error(`pbcopy 退出码 ${code}`));
      });
      child.stdin!.on("error", () => {}); // a failed spawn reports through "error"
      child.stdin!.end(text, "utf8");
    });
  },
  /** A power assertion held until the child exits (B29). */
  caffeinate(args: string[]): ChildProcess {
    return spawn("/usr/bin/caffeinate", args, { shell: false, stdio: "ignore" });
  },
};

function utf8Env() {
  return { ...process.env, LANG: "en_US.UTF-8" };
}
