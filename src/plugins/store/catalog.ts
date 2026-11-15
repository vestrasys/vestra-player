import type { PluginInfo } from "@vestra-player/sdk";
import { translationFor, type IndexEntry, type IndexVersion, type Revocation } from "./index-format.ts";
import { reservedId, reservedText } from "./labels.ts";
import { splitDescription } from "./open-line.ts";
import type { Repo } from "./repos.ts";
import { compareVersions, isVersion, playerAtLeast, satisfies } from "./semver.ts";
import { msg, type Msg } from "./text.ts";

/**
 * Repositories × the install library → the store's rows and their states
 * (design §2.3), version choice (§3.5) and the update count (§3.6).
 */

/** The store is a vestra/1 plugin: a player that runs it runs vestra/1. */
export const API = "vestra/1";

export type VersionCheck =
  | { ok: true }
  | { ok: false; why: "api" | "player"; text: Msg }
  | { ok: false; why: "revoked"; text: Msg; revocation: Revocation };

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
  if (v.apiVersion !== API) return { ok: false, why: "api", text: msg("Needs a newer plugin API ({})", v.apiVersion) };
  if (v.minPlayerVersion !== null && !playerAtLeast(player, v.minPlayerVersion))
    return { ok: false, why: "player", text: msg("Needs Vestra Player ≥ {} (current {})", v.minPlayerVersion, player) };
  const revocation = revocationOf(id, v.version, revoked);
  if (revocation) return { ok: false, why: "revoked", text: msg("Revoked: {}", revocation.reason), revocation };
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
  | "reserved" // a repository lists a `vestra.` id: only built-ins may use it
  | "incompatible" // no version this player can run
  | "allRevoked" // every compatible version is revoked
  | "local"; // installed, not in any repository (or its repository was removed)

/** Display fields of an installed plugin: the player's translation (`PluginInfo.display`), else its manifest. */
export interface Display {
  name: string;
  author: string | null;
  punchline: string | null;
  description: string | null;
  tags: string[];
  /** The manifest's own (English) name, punchline and tags: search and tags match these too. */
  english: string[];
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
  /** The installed plugin with this id (installed origin), if any; on a development row, that load. */
  installed: PluginInfo | null;
  /** A development load of the same id runs in this session. */
  development: boolean;
  best: IndexVersion | null;
  state: ItemState;
  /** The installed version is revoked by this row's repository. */
  revocation: Revocation | null;
  /** Why nothing can be installed (incompatible / all revoked / reserved). */
  blocked: Msg | null;
  /** Shown in the interface language when the index or the player translated it. */
  name: string;
  author: string | null;
  punchline: string | null;
  /** The description without its "how to open" line (open-line.ts). */
  description: string | null;
  /** The description's "how to open" line (open-line.ts), `null` none. */
  open: string | null;
  tags: string[];
  /** The English name, punchline and tags (search and the category filter match these too). */
  english: string[];
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
  locale = "en",
): Item[] {
  const installed = new Map(plugins.filter((p) => p.origin === "installed").map((p) => [p.id, p]));
  // Whatever is not from the install library is a development load (`--plugin`).
  const loads = plugins.filter((p) => p.origin !== "installed");
  const development = new Set(loads.map((p) => p.id));
  const covered = new Set<string>();
  const items: Item[] = [];
  for (const repo of repos) {
    const revoked = repo.index?.revoked ?? [];
    for (const entry of repo.index?.plugins ?? []) {
      const info = installed.get(entry.id) ?? null;
      const best = bestVersion(entry, revoked, player);
      let state: ItemState;
      let revocation: Revocation | null = null;
      let blocked: Msg | null = null;
      if (reservedId(entry.id)) {
        // Never installable from a repository; an installed copy (built-in or
        // left from before the rule) keeps its own local row below.
        state = "reserved";
        blocked = reservedText(entry.id);
      } else if (info?.builtin) state = "builtin";
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
          ? msg("This plugin has been revoked: {}", revocationOf(entry.id, compatible[0]!.version, revoked)?.reason ?? "")
          : (checkVersion(entry.id, entry.versions[0]!, [], player) as { text: Msg }).text;
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
        ...shown(entry, locale),
        author: entry.author,
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
      ...splitDescription(local.description),
      tags: local.tags,
      english: local.english,
      published: null,
      local,
      delisted: !revocation && repo?.index != null,
    });
  }
  // Development loads: listed under Installed (tagged Dev) so that they can be disabled and enabled for this run.
  for (const info of loads) {
    if (installed.has(info.id)) continue;
    const local = display(info);
    items.push({
      key: `\n${info.id}`,
      id: info.id,
      repo: null,
      entry: null,
      installed: info,
      development: true,
      best: null,
      state: "local",
      revocation: null,
      blocked: null,
      name: local.name,
      author: local.author,
      punchline: local.punchline,
      ...splitDescription(local.description),
      tags: local.tags,
      english: local.english,
      published: null,
      local,
    });
  }
  return items;
}

/** An index entry's display fields in `locale` (the index's `translations`), with the English ones for search. */
function shown(entry: IndexEntry, locale: string) {
  const t = translationFor(entry.translations, locale);
  return {
    name: t.name ?? entry.name,
    punchline: t.punchline ?? entry.punchline,
    ...splitDescription(t.description ?? entry.description),
    tags: t.tags ?? entry.tags,
    english: [entry.name, entry.punchline ?? "", ...entry.tags],
  };
}

/** A development load's own row (`--plugin`): disabling it lasts for this run only; it cannot be uninstalled. */
export const isDevelopmentRow = (item: Item) => item.installed !== null && item.installed.origin !== "installed";

/** Rows that stand for the installed instance or a development load (the Installed tab). */
export const isInstalledRow = (item: Item) =>
  item.state === "installed" ||
  item.state === "update" ||
  item.state === "revoked" ||
  item.state === "local" ||
  (item.state === "builtin" && item.repo === null);

/**
 * A repository's entry whose id has no installed instance, no built-in and no
 * development load, and that this player can install (the Not installed tab):
 * what could still be added. Entries that cannot be installed (incompatible,
 * every version revoked, a reserved `vestra.` id) stay under All with their
 * reason; listing them here would offer nothing to do.
 */
export const isNotInstalledRow = (item: Item) => item.entry !== null && item.state === "available" && !item.development;

/**
 * A built-in plugin's row (shipped with the player; it cannot be uninstalled
 * or disabled): its own row under Installed and All, and a repository's entry
 * for it under All ("builtin", or "reserved" for a `vestra.` id).
 */
export const isBuiltinRow = (item: Item) => item.installed?.builtin === true;

export const updateCount = (items: readonly Item[]) => items.filter((item) => item.state === "update").length;

/**
 * A failed installed row without a package directory: a record the player set aside at start (player 0.9.0 sets
 * aside packages built before it, whose manifest still has `capabilities`), or one whose library copy could not be
 * read this run. Nothing of it can run now, so it offers no Disable and no Reload: only Uninstall… (which removes a
 * set-aside record completely, player 0.10.1) and, from its repository, Update or a reinstall.
 */
export const isStrandedRow = (item: Item) =>
  item.installed !== null &&
  item.installed.origin === "installed" &&
  !item.installed.builtin &&
  item.installed.state === "failed" &&
  !item.installed.directory;
