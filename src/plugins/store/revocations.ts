import type { Item } from "./catalog.ts";
import { compareVersions, isVersion } from "./semver.ts";
import type { Store } from "./store.ts";

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
      return `「${item.name}」${item.installed!.version}：${item.revocation!.reason}${fix ? `\n可以在插件商城更新到 ${fix}。` : ""}`;
    });
    const first = items[0]!;
    const answer = await store.ask({
      message:
        items.length === 1
          ? `已停用被撤回的插件「${first.name}」${first.installed!.version}`
          : `已停用 ${items.length} 个被撤回的插件`,
      detail: `仓库撤回了这些版本，原因：\n\n${lines.join("\n\n")}\n\n如果仍要使用，可以在插件商城的详情页选择「仍然启用…」。`,
      confirm: "打开插件商城",
      cancel: "知道了",
    });
    if (answer === null) {
      store.osd(`已停用被撤回的插件「${first.name}」`);
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

  #riskDetail(item: Item): string {
    const fix = this.#fix(item);
    return (
      `仓库撤回这个版本的原因：${item.revocation!.reason}\n\n` +
      "继续使用可能有风险。确认后商城不会再自动停用它，详情页会一直显示警告。" +
      (fix ? `\n\n建议改为更新到 ${fix}。` : "")
    );
  }

  /** 「仍然启用…」 in the detail page. */
  async enable(item: Item) {
    const store = this.#store;
    if (!item.revocation || !item.installed || !item.repo) return;
    const answer = await store.ask({
      message: `仍然启用被撤回的「${item.name}」${item.installed.version}？`,
      detail: this.#riskDetail(item),
      confirm: "仍然启用",
      destructive: true,
    });
    if (!answer?.confirmed) return;
    const key = this.#key(item);
    this.#mark(key, "accepted");
    try {
      await store.plugin.packages.setEnabled(item.id, true);
      store.report(`已重新启用「${item.name}」`);
    } catch (error) {
      store.report(`无法启用「${item.name}」：${(error as Error).message}`, true);
    }
  }

  async #confirmReenabled(item: Item, key: string) {
    const store = this.#store;
    const answer = await store.ask({
      message: `「${item.name}」${item.installed!.version} 已被撤回，确定继续使用吗？`,
      detail: `它刚被重新启用。${this.#riskDetail(item)}`,
      confirm: "继续使用",
      cancel: "停用",
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
