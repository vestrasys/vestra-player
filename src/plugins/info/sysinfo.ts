import { cpus, freemem, totalmem } from "node:os";
import type { App, DisplayInfo, I18n, SystemInfo, TextArg } from "@vestra-player/sdk";
import type { SystemFacts } from "./properties.ts";

/**
 * I05 System page: the system's name, version, model and displays from the player
 * (`plugin.app.system()`), CPU and memory from Node's `os`. A failed read
 * leaves the player's facts unknown.
 */
export async function readFacts(i18n: I18n, app: Pick<App, "system">): Promise<SystemFacts> {
  const info = await app.system().catch((error) => {
    console.error(`[info] app.system: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  });
  const cpu = cpus();
  const brand = cpu[0]?.model.trim();
  return {
    os: info ? systemVersion(i18n, info) : null,
    model: info?.model ?? null,
    cpu: brand || null,
    cores: brand ? cpu.length : 0,
    memory: totalmem(),
    freeMemory: freemem(),
    displays: info?.displays.map(displayLine) ?? [],
  };
}

/** `macOS 26.2 (25C56)`, `Windows 11 23H2 (22631.4460)`; parts the system does not report are left out. */
export function systemVersion(i18n: I18n, info: Pick<SystemInfo, "name" | "version" | "build">): TextArg | null {
  const name = [info.name, info.version].filter(Boolean).join(" ");
  return name && info.build ? i18n.text("{} ({})", [name, info.build]) : name || info.build || null;
}

/** `Built-in Retina Display · 3024 x 1964 · 1512 x 982 @ 120 Hz`: pixels, then points. */
export function displayLine(display: DisplayInfo) {
  const scale = display.scale > 0 ? display.scale : 1;
  const points = `${Math.round(display.width / scale)} x ${Math.round(display.height / scale)}`;
  const rate = display.refreshRate ? ` @ ${Number(display.refreshRate.toFixed(2))} Hz` : "";
  return [display.name, `${display.width} x ${display.height}`, points + rate].filter(Boolean).join(" · ");
}
