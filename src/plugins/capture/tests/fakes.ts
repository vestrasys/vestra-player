import { copyFile, stat } from "node:fs/promises";
import { system } from "../system.ts";

export type Fakes = {
  /** Paths given to reveal, newest last. */
  revealed: string[];
  /** PNG paths put on the clipboard, with whether the file existed at that moment. */
  copied: { path: string; existed: boolean }[];
  /** Conversions asked for, newest last. */
  converted: { source: string; target: string; format: string; quality: number }[];
  failReveal: boolean;
  failCopy: boolean;
  /** "error": the command fails; "silent": it exits 0 and writes nothing (like sips on a bad input). */
  failConvert: "error" | "silent" | null;
  /** While set, copyImage waits for it (to hold a copy in progress). */
  hold: Promise<void> | null;
};

const exists = (path: string) => stat(path).then(() => true, () => false);

/** Replace every system command with an in-memory fake; conversions copy the file so outputs exist. */
export function fakeSystem(): Fakes {
  const fakes: Fakes = { revealed: [], copied: [], converted: [], failReveal: false, failCopy: false, failConvert: null, hold: null };
  system.reveal = async (path) => {
    if (fakes.failReveal) throw new Error("无法打开文件夹。");
    fakes.revealed.push(path);
  };
  system.copyImage = async (path) => {
    await fakes.hold;
    fakes.copied.push({ path, existed: await exists(path) });
    if (fakes.failCopy) throw new Error("osascript failed");
  };
  system.convertImage = async (source, target, format, quality) => {
    fakes.converted.push({ source, target, format, quality });
    if (fakes.failConvert === "error") throw new Error("sips failed");
    if (fakes.failConvert === "silent") return;
    await copyFile(source, target);
  };
  return fakes;
}
