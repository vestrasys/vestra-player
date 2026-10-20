import { execFile } from "node:child_process";
import { cpus, freemem, totalmem } from "node:os";
import type { SystemFacts } from "./properties.ts";

/**
 * I05 系统信息 from macOS commands (`sw_vers`, `sysctl`, `system_profiler`):
 * only this plugin needs them, so they stay here (round 2 task list §4). Never
 * through a shell, with a timeout and the caller's signal; a command that
 * fails leaves its fact unknown. Tests replace `sysinfo.run`.
 */
export const sysinfo = {
  run(program: string, args: string[], signal: AbortSignal): Promise<string> {
    return new Promise((resolve, reject) => {
      execFile(program, args, { shell: false, timeout: 10_000, signal, encoding: "utf8", maxBuffer: 4 * 1024 * 1024 }, (error, stdout) =>
        error ? reject(error) : resolve(stdout),
      );
    });
  },

  async read(signal: AbortSignal): Promise<SystemFacts> {
    const text = (program: string, ...args: string[]) =>
      this.run(program, args, signal).then((out) => out.trim() || null, () => null);
    const [version, build, model, brand, displays] = await Promise.all([
      text("/usr/bin/sw_vers", "-productVersion"),
      text("/usr/bin/sw_vers", "-buildVersion"),
      text("/usr/sbin/sysctl", "-n", "hw.model"),
      text("/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string"),
      text("/usr/sbin/system_profiler", "SPDisplaysDataType", "-json"),
    ]);
    const cores = cpus().length;
    return {
      macos: version ? `${version}${build ? `（${build}）` : ""}` : null,
      model,
      cpu: brand ? `${brand}${cores ? `（${cores} 核）` : ""}` : null,
      memory: totalmem(),
      freeMemory: freemem(),
      displays: parseDisplays(displays),
    };
  },
};

/** `Color LCD · 2940 x 1912 · 1470 x 956 @ 60.00Hz` per display in `system_profiler SPDisplaysDataType -json`. */
export function parseDisplays(json: string | null): string[] {
  if (!json) return [];
  try {
    const gpus = (JSON.parse(json) as { SPDisplaysDataType?: { spdisplays_ndrvs?: Record<string, string>[] }[] }).SPDisplaysDataType ?? [];
    return gpus.flatMap((gpu) => gpu.spdisplays_ndrvs ?? []).map((d) =>
      [d._name, d._spdisplays_pixels, d._spdisplays_resolution].filter(Boolean).join(" · ") || "未知显示器",
    );
  } catch {
    return [];
  }
}
