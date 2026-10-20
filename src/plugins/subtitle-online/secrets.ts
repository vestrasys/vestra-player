// API keys and passwords: the macOS keychain (generic passwords, service
// `vestra.subtitle-online`), or `secrets.json` (mode 0600) in the data
// directory when the keychain cannot be used. Never in settings.json.
import { execFile } from "node:child_process";
import { chmodSync, existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const SERVICE = "vestra.subtitle-online";
const SECURITY = "/usr/bin/security";

export type SecretName = "opensubtitles-api-key" | "opensubtitles-password" | "assrt-token";

function security(args: string[]): Promise<{ ok: boolean; stdout: string }> {
  return new Promise((resolve) => {
    execFile(SECURITY, args, { timeout: 10_000 }, (error, stdout) => resolve({ ok: !error, stdout: String(stdout) }));
  });
}

export class Secrets {
  readonly #file: string | null;
  #keychain: boolean;
  readonly #memory = new Map<string, string>();
  /**
   * `useKeychain: false` keeps everything in the file (tests, or
   * `VESTRA_SUBTITLE_ONLINE_KEYCHAIN=off`). Without a data directory and
   * without the keychain, secrets last for this run only.
   */
  constructor(directory: string | null, useKeychain = process.platform === "darwin" && existsSync(SECURITY)) {
    this.#file = directory ? join(directory, "secrets.json") : null;
    this.#keychain = useKeychain;
  }
  /** Where secrets are kept, for the settings window. */
  get location() {
    return this.#keychain ? "macOS 钥匙串（vestra.subtitle-online）" : this.#file ? `${this.#file}（仅本人可读）` : "仅本次运行";
  }
  async get(name: SecretName): Promise<string> {
    if (this.#keychain) {
      const result = await security(["find-generic-password", "-s", SERVICE, "-a", name, "-w"]);
      if (result.ok) return result.stdout.replace(/\n$/, "");
    }
    return this.#readFile()[name] ?? this.#memory.get(name) ?? "";
  }
  /** Store `value`; an empty value deletes the secret. */
  async set(name: SecretName, value: string): Promise<void> {
    if (this.#keychain) {
      const result = value
        ? await security(["add-generic-password", "-U", "-s", SERVICE, "-a", name, "-l", `Vestra 在线字幕（${name}）`, "-w", value])
        : await security(["delete-generic-password", "-s", SERVICE, "-a", name]);
      if (result.ok || !value) {
        this.#writeFile(name, ""); // never leave an older copy in the file
        return;
      }
      this.#keychain = false; // the keychain refused: fall back to the file
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
