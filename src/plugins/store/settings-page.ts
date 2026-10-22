import { hostOf } from "./labels.ts";
import { MAX_REPOS, normalizeRepoUrl } from "./repos.ts";
import type { Store } from "./store.ts";

/**
 * 「选项 ▶ 插件 ▶ 插件商城」 (design §4): automatic checks on or off, the
 * official repository (fixed) and up to 16 third-party ones. Adding reads the
 * repository first and asks with a warning; removing keeps what was
 * installed from it (it becomes 「只在本机」).
 */
export class SettingsPage {
  static readonly ID = "store.repos";
  readonly #store: Store;
  #adding = false;
  #working = false;
  #message = { text: "", warn: false };

  constructor(store: Store) {
    this.#store = store;
    const page = store.plugin.settings.page("ui/settings.slint", {
      id: SettingsPage.ID,
      category: "plugins",
      title: "插件商城",
      order: 20,
    });
    page.on("toggleAuto", () => {
      const settings = store.settings;
      settings.save({ ...settings.value, autoCheck: !settings.value.autoCheck });
    });
    page.on("startAdd", () => {
      this.#adding = true;
      page.set({ addText: "" });
      this.#say("");
    });
    page.on("cancelAdd", () => {
      this.#adding = false;
    });
    page.on("add", (text: string) => this.add(text));
    page.on("remove", (url: string) => this.remove(url));
    page.bind(() => this.#props());
  }

  #say(text: string, warn = false) {
    this.#message = { text, warn };
    this.#store.changed();
  }

  #props() {
    const store = this.#store;
    return {
      autoCheck: store.settings.value.autoCheck,
      repos: store.repos.list.map((repo) => ({
        name: repo.official ? "Vestra Player 官方仓库" : `非官方：${repo.label}`,
        url: repo.url,
        note: repo.status.text,
        tone: repo.status.warn ? 2 : 0,
        removable: !repo.official,
      })),
      adding: this.#adding,
      canAdd: store.settings.value.repos.length < MAX_REPOS && !this.#working,
      message: this.#message.text,
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
      return this.#say("这个仓库已经添加过了", true);
    if (store.settings.value.repos.length >= MAX_REPOS) return this.#say(`最多添加 ${MAX_REPOS} 个仓库`, true);
    this.#working = true;
    this.#say(`正在读取 ${hostOf(url)}…`);
    try {
      let name: string;
      try {
        const index = await store.repos.probe(url);
        name = index.name || "未命名仓库";
      } catch (error) {
        return this.#say(
          `无法读取这个仓库：${(error as Error).message}。请检查地址（仓库根地址或 index.json 的完整地址）`,
          true,
        );
      }
      const answer = await store.ask({
        message: `添加非官方仓库「${name}」？`,
        detail:
          `地址：${url}\n\n其中的插件未经 Vestra Player 审核，可能读取你的文件或联网。商城会显示这个仓库的插件，但不会替你判断它们是否可信。` +
          (local ? "\n\n这是本机上的测试仓库，没有加密。" : ""),
        confirm: "添加",
      });
      if (!answer?.confirmed) return this.#say("");
      store.settings.save({ ...store.settings.value, repos: [...store.settings.value.repos, url] });
      store.repos.sync(store.settings.value.repos);
      this.#adding = false;
      const repo = store.repos.find(url)!;
      await store.repos.refresh(repo);
      store.rebuild(true);
      this.#say(`已添加 ${repo.label}`);
    } finally {
      this.#working = false;
      store.changed();
    }
  }

  async remove(url: string) {
    const store = this.#store;
    const repo = store.repos.find(url);
    if (!repo || repo.official) return;
    const answer = await store.ask({
      message: `移除仓库「${repo.label}」？`,
      detail: "移除后商城不再读取这个仓库。从它安装的插件会保留，但商城不再为它们检查更新和撤回。",
      confirm: "移除",
      destructive: true,
    });
    if (!answer?.confirmed) return;
    store.settings.save({ ...store.settings.value, repos: store.settings.value.repos.filter((each) => each !== url) });
    store.repos.sync(store.settings.value.repos);
    store.repos.forget(url);
    store.rebuild();
    this.#say(`已移除 ${repo.label}`);
  }
}
