import type { Item } from "./catalog.ts";
import { compareVersions, isVersion } from "./semver.ts";
import type { Store } from "./store.ts";
import { list, msg, raw, type Msg } from "./text.ts";

/** Identifies one revocation hit: repository, plugin, installed version, range. */
export const revocationKey = (repo: string, id: string, version: string, range: string) =>
  JSON.stringify([repo, id, version, range]);

const RETRY_MS = 60_000;

/**
 * Revoked versions (design §3.10, decision 2.2-4). A plugin installed from a
 * repository whose `revoked` list holds its version is disabled once, with a
 * notice; the detail page keeps a red warning. Enabling it again needs a
 * risk confirmation (the store's 「仍然启用…」, or after the fact when it was
 * enabled elsewhere, e.g. in 插件管理); once accepted the store never
 * disables it again. Only the repository the plugin came from can revoke it.
 */
export class Revocations {
  readonly #store: Store;
  #running = false;
  #again = false;
  #retry: ReturnType<typeof setTimeout> | null = null;
  /** Keys asked about in this run (one question per re-enable). */
  readonly #asking = new Set<string>();

  constructor(store: Store) {
    this.#store = store;
    store.plugin.onDispose(() => {
      if (this.#retry) clearTimeout(this.#retry);
    });
  }

  #state(key: string) {
    return this.#store.settings.value.revocations[key];
  }

  #mark(key: string, value: "disabled" | "accepted") {
    const settings = this.#store.settings;
    settings.save({ ...settings.value, revocations: { ...settings.value.revocations, [key]: value } });
  }

  #key(item: Item): string {
    return revocationKey(item.repo!.url, item.id, item.installed!.version ?? "", item.revocation!.versions);
  }

  /** After every refresh and install-library change. */
  async check(): Promise<void> {
    if (this.#running) {
      this.#again = true;
      return;
    }
    this.#running = true;
    try {
      do {
        this.#again = false;
        await this.#pass();
      } while (this.#again);
    } finally {
      this.#running = false;
    }
  }

  async #pass() {
    const store = this.#store;
    const hits = store.items.filter((item) => item.state === "revoked" && item.repo && item.revocation && item.installed);
    const notices: Item[] = [];
    for (const item of hits) {
      const key = this.#key(item);
      const state = this.#state(key);
      const info = item.installed!;
      if (state === undefined) {
        if (info.state !== "disabled") {
          try {
            await store.plugin.packages.setEnabled(item.id, false);
          } catch (error) {
            // busy (an install in progress) or gone: try again on the next pass.
            store.deps.log(`disable revoked ${item.id}: ${(error as Error).message}`);
            continue;
          }
        }
        this.#mark(key, "disabled");
        notices.push(item);
      } else if (state === "disabled" && (info.state === "running" || info.state === "starting") && !this.#asking.has(key)) {
        // Enabled again somewhere else (插件管理): confirm the risk, or disable it again.
        this.#asking.add(key);
        void this.#confirmReenabled(item, key);
      }
    }
    if (notices.length) await this.#notify(notices);
  }

  async #notify(items: Item[]) {
    const store = this.#store;
    const lines = items.map((item) => {
      const fix = this.#fix(item);
      const { name, installed, revocation } = item;
      return fix
        ? msg("“{}” {}: {}\nYou can update to {} in the Plugin Store.", name, installed!.version ?? "", revocation!.reason, fix)
        : msg("“{}” {}: {}", name, installed!.version ?? "", revocation!.reason);
    });
    const first = items[0]!;
    const answer = await store.ask({
      message:
        items.length === 1
          ? msg("Disabled the revoked plugin “{}” {}", first.name, first.installed!.version ?? "")
          : msg("Disabled {} revoked plugins", items.length, ""),
      detail: msg(
        "The repository revoked these versions. Reasons:\n\n{2}\n\nTo keep using one anyway, choose “Enable Anyway…” on its page in the Plugin Store.",
        // One reason after another.
        list(lines, "\n\n"),
      ),
      confirm: "Open Plugin Store",
      cancel: "OK",
    });
    if (answer === null) {
      store.osd(msg("Disabled the revoked plugin “{}”", first.name));
      // Another dialog was open: show the notice again a little later.
      if (store.lastDialogError === "busy" && !this.#retry)
        this.#retry = setTimeout(() => {
          this.#retry = null;
          void this.#notify(items);
        }, RETRY_MS);
      return;
    }
    if (answer.confirmed) {
      await store.open(false);
      store.select(first.key);
    }
  }

  /** A newer version of the same repository that is not revoked. */
  #fix(item: Item): string | null {
    const best = item.best?.version;
    const current = item.installed?.version;
    return best && current && isVersion(current) && compareVersions(best, current) > 0 ? best : null;
  }

  /** The risk explained, after the message's two arguments (name and version). */
  #riskDetail(item: Item, again: boolean): Msg {
    const fix = this.#fix(item);
    const reason = item.revocation!.reason;
    if (again)
      return fix
        ? msg(
            "It was just enabled again. The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.\n\nUpdating to {3} instead is recommended.",
            reason,
            fix,
          )
        : msg(
            "It was just enabled again. The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.",
            reason,
          );
    return fix
      ? msg(
          "The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.\n\nUpdating to {3} instead is recommended.",
          reason,
          fix,
        )
      : msg(
          "The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.",
          reason,
        );
  }

  /** "Enable Anyway…" in the detail page. */
  async enable(item: Item) {
    const store = this.#store;
    if (!item.revocation || !item.installed || !item.repo) return;
    const answer = await store.ask({
      message: msg("Enable the revoked “{}” {} anyway?", item.name, item.installed.version ?? ""),
      detail: this.#riskDetail(item, false),
      confirm: "Enable Anyway",
      destructive: true,
    });
    if (!answer?.confirmed) return;
    const key = this.#key(item);
    this.#mark(key, "accepted");
    try {
      await store.plugin.packages.setEnabled(item.id, true);
      store.report(msg("Enabled “{}” again", item.name));
    } catch (error) {
      store.report(msg("Couldn't enable “{}”: {}", item.name, raw((error as Error).message)), true);
    }
  }

  async #confirmReenabled(item: Item, key: string) {
    const store = this.#store;
    const answer = await store.ask({
      message: msg("“{}” {} has been revoked. Keep using it?", item.name, item.installed!.version ?? ""),
      detail: this.#riskDetail(item, true),
      confirm: "Keep Using",
      cancel: "Disable",
      destructive: true,
    });
    if (answer === null) {
      // No dialog now: ask again on a later pass.
      this.#asking.delete(key);
      return;
    }
    if (answer.confirmed) {
      this.#mark(key, "accepted");
      return;
    }
    try {
      await store.plugin.packages.setEnabled(item.id, false);
    } catch (error) {
      store.deps.log(`disable revoked ${item.id}: ${(error as Error).message}`);
    }
    this.#asking.delete(key);
  }
}
