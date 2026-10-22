import type { PluginInfo } from "@vestra-player/sdk";
import type { IndexEntry, IndexVersion, Revocation } from "./index-format.ts";
import type { Repo } from "./repos.ts";
import { compareVersions, isVersion, playerAtLeast, satisfies } from "./semver.ts";

/**
 * Repositories × the install library → the store's rows and their states
 * (design §2.3), version choice (§3.5) and the update count (§3.6).
 */

/** The store is a vestra/1 plugin: a player that runs it runs vestra/1. */
export const API = "vestra/1";

export type VersionCheck =
  | { ok: true }
  | { ok: false; why: "api" | "player"; text: string }
  | { ok: false; why: "revoked"; text: string; revocation: Revocation };

/** The first `revoked` entry of the same id whose range holds `version`. */
export function revocationOf(id: string, version: string, revoked: readonly Revocation[]): Revocation | null {
  if (!isVersion(version)) return null;
  return revoked.find((r) => r.id === id && safeSatisfies(version, r.versions)) ?? null;
}

function safeSatisfies(version: string, range: string): boolean {
  try {
    return satisfies(version, range);
  } catch {
    return false;
  }
}

export function checkVersion(id: string, v: IndexVersion, revoked: readonly Revocation[], player: string): VersionCheck {
  if (v.apiVersion !== API) return { ok: false, why: "api", text: `需要更新的插件 API（${v.apiVersion}）` };
  if (v.minPlayerVersion !== null && !playerAtLeast(player, v.minPlayerVersion))
    return { ok: false, why: "player", text: `需要 Vestra Player ≥ ${v.minPlayerVersion}（当前 ${player}）` };
  const revocation = revocationOf(id, v.version, revoked);
  if (revocation) return { ok: false, why: "revoked", text: `已撤回：${revocation.reason}`, revocation };
  return { ok: true };
}

/** v*: the newest version that this player can run and that is not revoked. */
export function bestVersion(entry: IndexEntry, revoked: readonly Revocation[], player: string): IndexVersion | null {
  return entry.versions.find((v) => checkVersion(entry.id, v, revoked, player).ok) ?? null;
}

export type ItemState =
  | "available" // not installed
  | "installed" // from this repository, up to date
  | "update" // from this repository, v* is newer
  | "revoked" // from this repository, the installed version is revoked
  | "otherSource" // the id is installed from somewhere else
  | "builtin" // the id is a built-in plugin
  | "incompatible" // no version this player can run
  | "allRevoked" // every compatible version is revoked
  | "local"; // installed, not in any repository (or its repository was removed)

/** Display fields of an installed plugin, read from its manifest. */
export interface Display {
  name: string;
  author: string | null;
  punchline: string | null;
  description: string | null;
  tags: string[];
  license: string | null;
  sourceUrl: string | null;
  icon: string | null;
  readme: string | null;
  changelog: string | null;
}

export interface Item {
  /** `<repository url>\n<id>`, or `\n<id>` for a local row. */
  key: string;
  id: string;
  repo: Repo | null;
  entry: IndexEntry | null;
  /** The installed plugin with this id (installed origin), if any. */
  installed: PluginInfo | null;
  /** A development load of the same id runs in this session. */
  development: boolean;
  best: IndexVersion | null;
  state: ItemState;
  /** The installed version is revoked by this row's repository. */
  revocation: Revocation | null;
  /** Why nothing can be installed (incompatible / all revoked). */
  blocked: string | null;
  name: string;
  author: string | null;
  punchline: string | null;
  tags: string[];
  published: string | null;
  local: Display | null;
  /** A local row installed from a configured repository that does not list it any more. */
  delisted?: boolean;
}

export const sameRepository = (info: PluginInfo | null, repo: Repo) =>
  info?.source?.kind === "repository" && info.source.url === repo.url;

export function buildItems(
  repos: readonly Repo[],
  plugins: readonly PluginInfo[],
  player: string,
  display: (info: PluginInfo) => Display,
): Item[] {
  const installed = new Map(plugins.filter((p) => p.origin === "installed").map((p) => [p.id, p]));
  const development = new Set(plugins.filter((p) => p.origin === "development").map((p) => p.id));
  const covered = new Set<string>();
  const items: Item[] = [];
  for (const repo of repos) {
    const revoked = repo.index?.revoked ?? [];
    for (const entry of repo.index?.plugins ?? []) {
      const info = installed.get(entry.id) ?? null;
      const best = bestVersion(entry, revoked, player);
      let state: ItemState;
      let revocation: Revocation | null = null;
      let blocked: string | null = null;
      if (info?.builtin) state = "builtin";
      else if (info && sameRepository(info, repo)) {
        covered.add(info.id);
        revocation = info.version ? revocationOf(entry.id, info.version, revoked) : null;
        if (revocation) state = "revoked";
        else if (best && info.version && isVersion(info.version) && compareVersions(best.version, info.version) > 0)
          state = "update";
        else state = "installed";
      } else if (info) state = "otherSource";
      else if (best) state = "available";
      else {
        const compatible = entry.versions.filter((v) => checkVersion(entry.id, v, [], player).ok);
        state = compatible.length ? "allRevoked" : "incompatible";
        blocked = compatible.length
          ? `此插件已被撤回：${revocationOf(entry.id, compatible[0]!.version, revoked)?.reason ?? ""}`
          : (checkVersion(entry.id, entry.versions[0]!, [], player) as { text: string }).text;
      }
      items.push({
        key: `${repo.url}\n${entry.id}`,
        id: entry.id,
        repo,
        entry,
        installed: info,
        development: development.has(entry.id),
        best,
        state,
        revocation,
        blocked,
        name: entry.name,
        author: entry.author,
        punchline: entry.punchline,
        tags: entry.tags,
        published: (best ?? entry.versions[0])?.published ?? null,
        local: null,
      });
    }
  }
  for (const info of installed.values()) {
    if (covered.has(info.id)) continue;
    const local = display(info);
    // Installed from a repository that no longer lists it (delisted, or all
    // its entries unusable): that repository's `revoked` still applies.
    const source = info.source?.kind === "repository" ? info.source.url : null;
    const repo = source ? (repos.find((r) => r.url === source) ?? null) : null;
    const revocation =
      repo?.index && info.version && !info.builtin ? revocationOf(info.id, info.version, repo.index.revoked) : null;
    items.push({
      key: revocation ? `${repo!.url}\n${info.id}` : `\n${info.id}`,
      id: info.id,
      repo: revocation ? repo : null,
      entry: null,
      installed: info,
      development: development.has(info.id),
      best: null,
      state: info.builtin ? "builtin" : revocation ? "revoked" : "local",
      revocation,
      blocked: null,
      name: local.name,
      author: local.author,
      punchline: local.punchline,
      tags: local.tags,
      published: null,
      local,
      delisted: !revocation && repo?.index != null,
    });
  }
  return items;
}

/** Rows that stand for the installed instance (the 「已安装」 filter). */
export const isInstalledRow = (item: Item) =>
  item.state === "installed" ||
  item.state === "update" ||
  item.state === "revoked" ||
  item.state === "local" ||
  (item.state === "builtin" && item.repo === null);

export const updateCount = (items: readonly Item[]) => items.filter((item) => item.state === "update").length;
