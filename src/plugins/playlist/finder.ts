import { spawn } from "node:child_process";
import { stat } from "node:fs/promises";
import { basename, dirname } from "node:path";

/** Runs `/usr/bin/open` with `args` (no shell); rejects when it fails or takes over 10 s. */
export type OpenRunner = (args: string[]) => Promise<void>;

export const runOpen: OpenRunner = (args) =>
  new Promise((resolve, reject) => {
    const child = spawn("open", args, { shell: false, stdio: "ignore" });
    const timer = setTimeout(() => child.kill("SIGKILL"), 10_000);
    child.on("error", reject);
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new Error(`open exited with ${code}`));
    });
  });

export const REVEAL_LIMIT = 20;

const exists = async (path: string, folder: boolean) => {
  try {
    const info = await stat(path);
    return folder ? info.isDirectory() : true;
  } catch {
    return false;
  }
};

/**
 * C23: reveal local files in Finder, one `open -R` per folder; a missing file shows
 * its folder; returns the notices to show (missing folders, the 20-file limit).
 */
export async function reveal(paths: string[], run: OpenRunner): Promise<string[]> {
  const notices: string[] = [];
  if (paths.length > REVEAL_LIMIT) notices.push(`只显示前 ${REVEAL_LIMIT} 个文件`);
  const folders = new Map<string, string[]>();
  for (const path of paths.slice(0, REVEAL_LIMIT)) {
    const dir = dirname(path);
    folders.set(dir, [...(folders.get(dir) ?? []), path]);
  }
  for (const [dir, files] of folders) {
    const present: string[] = [];
    for (const file of files) if (await exists(file, false)) present.push(file);
    if (present.length) await run(["-R", ...present]);
    else if (await exists(dir, true)) await run([dir]);
    else notices.push(`找不到文件：${basename(files[0]!)}`);
  }
  return notices;
}
