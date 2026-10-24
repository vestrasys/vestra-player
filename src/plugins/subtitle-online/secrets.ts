// API keys and passwords: the player's secret store (`plugin.secrets`: the
// macOS keychain, the Windows Credential Manager), or `secrets.json` (mode
// 0600) in the data directory when the system has none. Never in settings.json.
import { chmodSync, existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Secrets as Store, SystemInfo } from "@vestra-player/sdk";

export type SecretName = "opensubtitles-api-key" | "opensubtitles-password" | "assrt-token";

/** Where secrets are kept; the windows name it (ui/messages.slint `Messages.location`). */
export type Location = { kind: "macos" | "windows" | "system" | "file" | "memory"; value: string };
const STORES: Partial<Record<SystemInfo["os"], Location["kind"]>> = { macos: "macos", windows: "windows" };
/** The system has no usable store: fall back to the file (or this run). */
const noStore = (e: unknown) => {
  const code = (e as { code?: string })?.code;
  return code === "unsupported" || code === "unavailable";
};

export class Secrets {
  readonly #file: string | null;
  #store: Store | null;
  readonly #os: SystemInfo["os"];
  readonly #id: string;
  readonly #memory = new Map<string, string>();
  /**
   * `store` is `plugin.secrets` (`null` without the capability); `os` names
   * it in `location`. Without a data directory and without a store, secrets
   * last for this run only.
   */
  constructor(directory: string | null, store: Store | null, os: SystemInfo["os"], id = "vestra.subtitle-online") {
    this.#file = directory ? join(directory, "secrets.json") : null;
    this.#os = os;
    this.#id = id;
    this.#store = store;
  }
  /** Where secrets are kept, for the settings windows. */
  get location(): Location {
    if (this.#store) return { kind: STORES[this.#os] ?? "system", value: this.#id };
    return this.#file ? { kind: "file", value: this.#file } : { kind: "memory", value: "" };
  }
  async get(name: SecretName): Promise<string> {
    if (this.#store) {
      try {
        const value = await this.#store.get(name);
        if (value !== null) return value;
      } catch (e) {
        if (noStore(e)) this.#store = null;
      }
    }
    return this.#readFile()[name] ?? this.#memory.get(name) ?? "";
  }
  /** Store `value`; an empty value deletes the secret. */
  async set(name: SecretName, value: string): Promise<void> {
    if (this.#store) {
      try {
        if (value) await this.#store.set(name, value);
        else await this.#store.delete(name);
        this.#writeFile(name, ""); // never leave an older copy in the file
        return;
      } catch (e) {
        if (!noStore(e)) throw e;
        this.#store = null; // no store after all: fall back to the file
      }
    }
    this.#writeFile(name, value);
  }
  #readFile(): Record<string, string> {
    if (!this.#file || !existsSync(this.#file)) return {};
    try {
      const value = JSON.parse(readFileSync(this.#file, "utf8"));
      return value && typeof value === "object" ? value : {};
    } catch {
      return {};
    }
  }
  #writeFile(name: string, value: string) {
    if (!this.#file) {
      if (value) this.#memory.set(name, value);
      else this.#memory.delete(name);
      return;
    }
    const all = this.#readFile();
    if (!value && !(name in all)) return;
    if (value) all[name] = value;
    else delete all[name];
    const temporary = `${this.#file}.tmp`;
    writeFileSync(temporary, JSON.stringify(all, null, 2), { mode: 0o600 });
    chmodSync(temporary, 0o600);
    renameSync(temporary, this.#file);
  }
}
