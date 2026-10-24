import { hostOf, reasonOf } from "./labels.ts";
import { MAX_REPOS, normalizeRepoUrl } from "./repos.ts";
import type { Store } from "./store.ts";
import { msg, slint, type Msg } from "./text.ts";

/**
 * Options ▸ Plugins ▸ Plugin Store (design §4): automatic checks on or off,
 * the official repository (fixed) and up to 16 third-party ones. Adding
 * reads the repository first and asks with a warning; removing keeps what
 * was installed from it (it becomes a local row).
 */
export class SettingsPage {
  static readonly ID = "store.repos";
  readonly #store: Store;
  #adding = false;
  #working = false;
  #message: { text: Msg | null; warn: boolean } = { text: null, warn: false };

  constructor(store: Store) {
    this.#store = store;
    const page = store.plugin.settings.page("ui/settings.slint", {
      id: SettingsPage.ID,
      category: "plugins",
      title: "Plugin Store",
      order: 20,
    });
    page.on("toggleAuto", () => {
      const settings = store.settings;
      settings.save({ ...settings.value, autoCheck: !settings.value.autoCheck });
    });
    page.on("startAdd", () => {
      this.#adding = true;
      page.set({ addText: "" });
      this.#say(null);
    });
    page.on("cancelAdd", () => {
      this.#adding = false;
    });
    page.on("add", (text: string) => this.add(text));
    page.on("remove", (url: string) => this.remove(url));
    page.bind(() => this.#props());
  }

  #say(text: Msg | null, warn = false) {
    this.#message = { text, warn };
    this.#store.changed();
  }

  #props() {
    const store = this.#store;
    return {
      autoCheck: store.settings.value.autoCheck,
      repos: store.repos.list.map((repo) => ({
        name: slint(repo.official ? msg("Vestra Player official repository") : repo.origin),
        url: repo.url,
        note: slint(repo.status.text),
        tone: repo.status.warn ? 2 : 0,
        removable: !repo.official,
      })),
      adding: this.#adding,
      canAdd: store.settings.value.repos.length < MAX_REPOS && !this.#working,
      message: slint(this.#message.text),
      messageWarn: this.#message.warn,
    };
  }

  async add(text: string) {
    const store = this.#store;
    if (this.#working) return;
    const normalized = normalizeRepoUrl(text);
    if ("error" in normalized) return this.#say(normalized.error, true);
    const { url, local } = normalized;
    if (url === store.repos.official.url || store.settings.value.repos.includes(url))
      return this.#say(msg("This repository has already been added"), true);
    if (store.settings.value.repos.length >= MAX_REPOS) return this.#say(msg("At most {} repositories can be added", MAX_REPOS), true);
    this.#working = true;
    this.#say(msg("Reading {}…", hostOf(url)));
    try {
      let name: string;
      try {
        const index = await store.repos.probe(url);
        name = index.name;
      } catch (error) {
        return this.#say(
          msg(
            "Couldn't read this repository: {}. Check the address (the repository's root address or the full address of index.json)",
            reasonOf(error),
          ),
          true,
        );
      }
      const answer = await store.ask({
        message: name
          ? msg("Add the unofficial repository “{}”?", name)
          : msg("Add the unofficial repository “Unnamed repository”?", ""),
        detail: local
          ? msg(
              "Address: {1}\n\nIts plugins are not reviewed by Vestra Player and may read your files or use the network. The store shows this repository's plugins but does not judge for you whether they can be trusted.\n\nThis is a test repository on this machine, without encryption.",
              url,
            )
          : msg(
              "Address: {1}\n\nIts plugins are not reviewed by Vestra Player and may read your files or use the network. The store shows this repository's plugins but does not judge for you whether they can be trusted.",
              url,
            ),
        confirm: "Add",
      });
      if (!answer?.confirmed) return this.#say(null);
      store.settings.save({ ...store.settings.value, repos: [...store.settings.value.repos, url] });
      store.repos.sync(store.settings.value.repos);
      this.#adding = false;
      const repo = store.repos.find(url)!;
      await store.repos.refresh(repo);
      store.rebuild(true);
      this.#say(msg("Added {}", repo.label));
    } finally {
      this.#working = false;
      store.changed();
    }
  }

  async remove(url: string) {
    const store = this.#store;
    const repo = store.repos.find(url);
    if (!repo || repo.official) return;
    const name = repo.index?.name;
    const host = hostOf(repo.url);
    const answer = await store.ask({
      message: name
        ? msg("Remove the repository “{} ({})”?", name, host)
        : msg("Remove the repository “Unnamed repository ({1})”?", "", host),
      detail: msg(
        "The store will stop reading this repository. Plugins installed from it stay, but the store no longer checks them for updates and revocations.",
      ),
      confirm: "Remove",
      destructive: true,
    });
    if (!answer?.confirmed) return;
    store.settings.save({ ...store.settings.value, repos: store.settings.value.repos.filter((each) => each !== url) });
    store.repos.sync(store.settings.value.repos);
    store.repos.forget(url);
    store.rebuild();
    this.#say(msg("Removed {}", repo.label));
  }
}
