import type { Plugin } from "@vestra-player/sdk";
import { OFFICIAL_URL, type Deps } from "./repos.ts";
import { officialKeys } from "./signing.ts";
import { Store } from "./store.ts";

/**
 * 插件商城 (vestra.store, design doc/plugins/store-design.md): top-level
 * 「插件商城…」 (order 822, group windows), the store window, the settings
 * page 「选项 ▶ 插件 ▶ 插件商城」 and `.vxp` files dropped on the video area.
 * Network, signature checks and downloads run in this plugin's process
 * (Node fetch and crypto); installing goes through `plugin.packages`.
 */
export default async function activate(plugin: Plugin) {
  await startStore(plugin, {
    fetch: (input, init) => fetch(input, init),
    keys: officialKeys(),
    now: () => Date.now(),
    officialUrl: OFFICIAL_URL,
    log: (line) => console.warn(`vestra.store: ${line}`),
  });
}

/** Tests start the store with their own network, keys and clock. */
export async function startStore(plugin: Plugin, deps: Omit<Deps, "userAgent">) {
  const store = new Store(plugin, deps);
  plugin.onError((error) => store.deps.log(error.message));
  await store.start();
  return store;
}
