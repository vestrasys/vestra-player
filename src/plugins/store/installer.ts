import { randomUUID } from "node:crypto";
import { mkdir, rename, rm } from "node:fs/promises";
import { basename, join } from "node:path";
import type { PackageCandidate, PackageChange, PackageSource, PluginInfo } from "@vestra-player/sdk";
import type { Item } from "./catalog.ts";
import type { IndexVersion } from "./index-format.ts";
import { MAX_PACKAGE } from "./index-format.ts";
import { permissionList, reservedId, reservedText, sourceName } from "./labels.ts";
import { get, isHttps, NetError } from "./net.ts";
import { compareVersions, isVersion } from "./semver.ts";
import type { Store } from "./store.ts";
import { msg, raw, type Msg } from "./text.ts";

/**
 * Installing (design §3.8, §3.9): every path asks first, downloads with
 * limits, checks size and sha256 against the index, lets the player check
 * the archive (`validateArchive`), compares the package with what the user
 * agreed to, installs with the right source and follows the change to its
 * end. A plugin installed from another source is replaced only after the
 * user agreed to that too.
 */

const errorCode = (error: unknown) => (error as { code?: string }).code;

/** User-facing text for a refused or failed step. */
export function failureText(store: Store, error: unknown, id?: string): Msg {
  if (error instanceof NetError) {
    switch (error.code) {
      case "size":
        return msg("The downloaded file does not match the repository's description (size); deleted");
      case "sha256":
        return msg("The downloaded file does not match the repository's description (checksum); deleted");
      case "tooLarge":
        return msg("The plugin package exceeds 64 MB; download stopped");
      case "protocol":
      case "redirects":
        return msg("The download address is insecure or redirects too often; stopped");
      case "aborted":
        return msg("Download cancelled");
      case "disk":
        return msg("Couldn't write the download file: {}", error.reason);
      default:
        return msg("Download failed: {}", error.reason);
    }
  }
  const message = (error as Error).message;
  switch (errorCode(error)) {
    case "invalid":
      return msg("This is not a valid plugin package: {}", raw(message));
    case "versionMismatch": {
      const needed = /(\d+\.\d+\.\d+)/.exec(message)?.[1];
      return needed
        ? msg("This plugin needs Vestra Player ≥ {} (current {})", needed, store.plugin.app.version)
        : msg("This plugin needs a newer Vestra Player (current {})", store.plugin.app.version);
    }
    case "reserved":
      // The player's own wording, in the player's language.
      return message ? raw(message) : reservedText(id);
    case "unsupported":
      return id === undefined
        ? msg("This plugin has the same id as a built-in plugin and cannot be installed")
        : msg("“{}” has the same id as a built-in plugin and cannot be installed", id);
    case "busy":
      return msg("Plugins are being installed or cleaned up; try again later");
    case "capability":
      return msg("The Plugin Store lacks the “Manage plugins” permission");
    case "missing":
      return msg("This plugin no longer exists");
    default:
      return raw(message || String(error));
  }
}

/** The installed plugin with this id (installed origin). */
const installedOf = (store: Store, id: string): PluginInfo | null =>
  store.packages.find((p) => p.id === id && p.origin === "installed") ?? null;

const identity = (source: PackageSource | null) =>
  !source || source.kind === "file" || source.kind === "link"
    ? "manual"
    : source.kind === "repository"
      ? `repository ${source.url}`
      : source.kind;

function replaceDetail(info: PluginInfo, name: string): Msg {
  return msg(
    "The current “{}” {} comes from {}. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data. The original plugin's data is kept and is used again when you reinstall it from the original source.",
    name,
    info.version ?? "",
    sourceName(info.source),
  );
}

/** What the plugin will be able to do, and what permissions do not limit. */
const willBeAble = (capabilities: readonly string[]) =>
  msg(
    "It will be able to:\n{}\n\nA plugin is a complete program: the permissions above only limit what it can do with the player, not reading and writing files or using the network.",
    permissionList(capabilities),
  );

// ------------------------------------------------------------ repository

export async function installFromRepository(store: Store, item: Item, version: IndexVersion) {
  const repo = item.repo;
  const entry = item.entry;
  if (!repo || !entry) return;
  // Refused before asking or downloading, whichever repository lists it.
  if (reservedId(item.id)) return store.report(reservedText(item.id), true);
  const info = installedOf(store, item.id);
  const name = item.name;
  const sameSource = info !== null && identity(info.source) === identity({ kind: "repository", url: repo.url });
  const replace = info !== null && !sameSource;
  const unofficial = repo.local
    ? msg(
        "Repository: {} ({}). This repository is not reviewed by Vestra Player; its plugins may read your files or use the network. Only install plugins from authors you trust. This is a test repository on this machine, without encryption.",
        repo.label,
        repo.url,
      )
    : msg(
        "Repository: {} ({}). This repository is not reviewed by Vestra Player; its plugins may read your files or use the network. Only install plugins from authors you trust.",
        repo.label,
        repo.url,
      );
  const perms = willBeAble(version.capabilities);
  let question: Question;
  if (replace) {
    question = {
      message: msg("Replace the installed “{}”?", name),
      detail: repo.official
        ? msg("{1}\n\nWill install {2} from the official repository.\n\n{3}", replaceDetail(info, name), version.version, perms)
        : msg(
            "{1}\n\nWill install {2} from the unofficial repository.\n\n{4}\n\n{3}",
            replaceDetail(info, name),
            version.version,
            perms,
            unofficial,
          ),
      confirm: "Replace",
      destructive: true,
    };
  } else if (sameSource && info.version && isVersion(info.version)) {
    const order = compareVersions(version.version, info.version);
    const added = version.capabilities.filter((c) => !info.capabilities.includes(c));
    const current = info.version;
    if (order > 0)
      question = added.length
        ? {
            message: msg("Update “{}” to {}? The new version needs more permissions", name, version.version),
            detail: msg(
              "Added:\n{2}\n\nCurrent: {3}.\n\nA plugin is a complete program: the permissions above only limit what it can do with the player, not reading and writing files or using the network.",
              permissionList(added),
              current,
            ),
            confirm: "Update",
          }
        : {
            message: msg("Update “{}” to {}?", name, version.version),
            detail: msg("Current: {2}.", current),
            confirm: "Update",
          };
    // Never older than the installed version from the same source (the Versions tab offers none): uninstall, then install a .vxp.
    else if (order < 0) return;
    else
      question = {
        message: msg("Install “{}” {} (reinstall)?", name, version.version),
        detail: added.length
          ? msg("Current: {2}.\n\nThis version needs more permissions:\n{3}", current, permissionList(added))
          : msg("Current: {2}.", current),
        confirm: "Install",
      };
  } else
    question = repo.official
      ? {
          message: msg("Install “{}” {}?", name, version.version),
          detail: msg(
            "From the Vestra Player official repository, author {2}.\n\n{3}",
            item.author ?? msg("Unknown"),
            perms,
          ),
          confirm: "Install",
        }
      : {
          message: msg("Install “{}” {} from an unofficial repository?", name, version.version),
          detail: msg("{2}\n\n{3}", unofficial, perms),
          confirm: "Install Anyway",
          destructive: true,
        };
  if (store.busy) return store.report(msg("Another install is in progress; wait for it to finish"), true);
  const answer = await store.ask(question);
  if (!answer?.confirmed) return;

  const controller = new AbortController();
  if (!store.begin(msg("Downloading {} {}", name, version.version), () => controller.abort())) return;
  let file: string | null = null;
  try {
    file = await download(store, version.downloadUrl, { name, version: version.version }, controller.signal, {
      exactSize: version.size,
      sha256: version.sha256,
      allowLocalHttp: repo.local,
    });
    store.progress(msg("Checking {}…", name), null);
    const candidate = await store.plugin.packages.validateArchive(file);
    const mismatch = compare(candidate, item.id, version);
    if (mismatch) return store.report(mismatch, true);
    await installCandidate(store, candidate, { kind: "repository", url: repo.url }, replace, name, sameSource ? info : null);
  } catch (error) {
    store.report(failureText(store, error, item.id), true);
  } finally {
    store.end();
    if (file) await rm(file, { force: true }).catch(() => {});
  }
}

/**
 * A confirmation: `message` and `detail` share one list of arguments, so a
 * `detail` template numbers its placeholders after the message's arguments.
 */
export type Question = Parameters<Store["ask"]>[0];

/** What differs between the package and the index entry the user agreed to. */
function compare(candidate: PackageCandidate, id: string, version: IndexVersion): Msg | null {
  const manifest = candidate.manifest;
  if (manifest.id !== id)
    return msg("The plugin package does not match the repository's description (id); not installed");
  if (manifest.version !== version.version)
    return msg("The plugin package does not match the repository's description (version); not installed");
  const declared = new Set(version.capabilities);
  const actual = new Set(manifest.capabilities);
  if (declared.size !== actual.size || [...actual].some((c) => !declared.has(c)))
    return msg("The plugin package does not match the repository's description (permissions); not installed");
  return null;
}

async function download(
  store: Store,
  url: string,
  /** What is downloading (shown with the percentage); `null`: a package from a link. */
  label: { name: string; version: string } | null,
  signal: AbortSignal,
  limits: { exactSize?: number; sha256?: string; allowLocalHttp?: boolean },
): Promise<string> {
  const directory = join(store.plugin.dataDirectory, "downloads");
  await mkdir(directory, { recursive: true });
  const base = join(directory, randomUUID());
  const part = `${base}.vxp.part`;
  let shown = -1;
  const size = limits.exactSize;
  await get(store.deps.fetch, url, {
    maxBytes: size ?? MAX_PACKAGE,
    ...(size === undefined ? {} : { exactSize: size }),
    ...(limits.sha256 === undefined ? {} : { sha256: limits.sha256 }),
    timeoutMs: Math.max(60_000, ((size ?? MAX_PACKAGE) / 100_000) * 1000),
    headers: { "User-Agent": store.deps.userAgent },
    signal,
    file: part,
    allowLocalHttp: limits.allowLocalHttp === true,
    onProgress: (received, total) => {
      const whole = size ?? total;
      if (!whole) return;
      const percent = Math.floor((received / whole) * 100);
      if (percent === shown) return;
      shown = percent;
      store.progress(
        label
          ? msg("Downloading {} {} ({}%)", label.name, label.version, percent)
          : msg("Downloading the plugin package ({}%)", percent),
      );
    },
  });
  // validateArchive wants the .vxp extension.
  const file = `${base}.vxp`;
  await rename(part, file);
  return file;
}

/**
 * `install` with the conflict → confirm → replace flow, then follow the
 * change. `previous` is the same-source version being upgraded (for the
 * wording), `replace` when the user already agreed to replace.
 */
async function installCandidate(
  store: Store,
  candidate: PackageCandidate,
  source: PackageSource,
  replace: boolean,
  name: string,
  previous: PluginInfo | null,
) {
  const version = candidate.manifest.version;
  store.progress(msg("Installing {}…", name), null);
  let change: PackageChange;
  try {
    change = await candidate.install({ source, ...(replace ? { replace: true } : {}) });
  } catch (error) {
    if (errorCode(error) !== "conflict" || replace) throw error;
    // Installed from somewhere else since we asked: ask about replacing.
    const info = installedOf(store, candidate.manifest.id);
    const answer = await store.ask({
      message: msg("Replace the installed “{}”?", name),
      detail: info
        ? msg("{1}", replaceDetail(info, name))
        : msg(
            "This plugin has already been installed from another source. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data. The original plugin's data is kept and is used again when you reinstall it from the original source.",
          ),
      confirm: "Replace",
      destructive: true,
    });
    if (!answer?.confirmed) return store.report(msg("Not installed"));
    change = await candidate.install({ source, replace: true });
  }
  await follow(store, change, "install", () => {
    const upgraded = previous?.version && isVersion(previous.version) && compareVersions(version, previous.version) > 0;
    return upgraded ? msg("Updated {} to {}", name, version) : msg("Installed {} {}", name, version);
  });
  store.select(selectKey(store, candidate.manifest.id, source));
}

/** After an install: select the row that now stands for it. */
function selectKey(store: Store, id: string, source: PackageSource): string {
  if (source.kind === "repository") return `${source.url}\n${id}`;
  return `\n${id}`;
}

type Verb = "install" | "uninstall";

/** What `follow` shows for each verb (one whole sentence per verb and outcome). */
const FOLLOW = {
  install: {
    preparing: msg("Installing… (preparing)"),
    activating: msg("Installing… (activating)"),
    tooLate: msg("Can no longer cancel; the installation continues"),
    failedDegraded: msg("Installation failed; the previous version was restored"),
    cancelled: msg("Installation cancelled"),
    failed: (why: Msg) => msg("Installation failed: {}", why),
  },
  uninstall: {
    preparing: msg("Uninstalling… (preparing)"),
    activating: msg("Uninstalling… (activating)"),
    tooLate: msg("Can no longer cancel; the uninstallation continues"),
    cancelled: msg("Uninstallation cancelled"),
    failed: (why: Msg) => msg("Uninstallation failed: {}", why),
  },
} satisfies Record<Verb, unknown>;

async function follow(store: Store, change: PackageChange, verb: Verb, committed: () => Msg) {
  const words = FOLLOW[verb];
  store.progress(change.status === "activating" ? words.activating : words.preparing, () => void change.cancel().catch(() => {}));
  change.onChange((status) => {
    if (status === "activating") store.progress(words.activating);
  });
  const status = await change.done;
  switch (status) {
    case "committed":
      return store.report(committed());
    case "tooLate":
      return store.report(words.tooLate);
    case "failedDegraded":
      // Only an install reports this (the player kept what was installed before); an uninstall never does.
      if ("failedDegraded" in words) return store.report(words.failedDegraded, true);
      break;
    case "cancelled":
      return store.report(words.cancelled);
  }
  return store.report(words.failed(raw(change.message ?? status)), true);
}

// ------------------------------------------------------------ side-loading

function sideloadQuestion(store: Store, candidate: PackageCandidate, from: { file: string } | { link: string }) {
  const manifest = candidate.manifest;
  const name = manifest.name ?? manifest.id;
  const info = installedOf(store, manifest.id);
  const perms = willBeAble(manifest.capabilities);
  const warning =
    "file" in from
      ? msg(
          "From the file {}; not reviewed by any repository and not updated automatically. Only install plugins from authors you trust.",
          from.file,
        )
      : msg(
          "From the link {}; not reviewed by any repository and not updated automatically. Only install plugins from authors you trust.",
          from.link,
        );
  if (info && identity(info.source) !== "manual")
    return {
      replace: true,
      question: {
        message: msg("Replace the installed version with the plugin package “{}” {}?", name, manifest.version),
        detail: msg("{2}\n\n{3}\n\n{4}", replaceDetail(info, name), warning, perms),
        confirm: "Replace",
        destructive: true,
      } satisfies Question,
    };
  return {
    replace: false,
    question: {
      message: msg("Install the plugin package “{}” {}?", name, manifest.version),
      detail: info
        ? msg(
            "{2}\n\nCurrently installed: {3} (same source).\n\n{4}",
            warning,
            info.version ?? "",
            perms,
          )
        : msg("{2}\n\n{3}", warning, perms),
      confirm: "Install Anyway",
      destructive: true,
    } satisfies Question,
  };
}

/** "Install from File…" and a `.vxp` dropped on the video area. */
export async function installFile(store: Store, path: string) {
  if (!store.begin(msg("Checking {}…", basename(path)))) return;
  try {
    let candidate: PackageCandidate;
    try {
      candidate = await store.plugin.packages.validateArchive(path);
    } catch (error) {
      return store.report(failureText(store, error), true);
    }
    // validateArchive already refused reserved ids, so built-in ones too.
    const info = installedOf(store, candidate.manifest.id);
    const { replace, question } = sideloadQuestion(store, candidate, { file: basename(path) });
    const answer = await store.ask(question);
    if (!answer?.confirmed) return;
    const name = candidate.manifest.name ?? candidate.manifest.id;
    await installCandidate(store, candidate, { kind: "file" }, replace, name, replace ? null : info);
  } catch (error) {
    store.report(failureText(store, error), true);
  } finally {
    store.end();
  }
}

/** "Install from Link…": https only, at most 64 MB; the permissions are known only after the download. */
export async function installFromLink(store: Store, text: string) {
  let url: string;
  try {
    url = new URL(text.trim()).href;
  } catch {
    return store.report(msg("You can only install from https:// links"), true);
  }
  if (!isHttps(url)) return store.report(msg("You can only install from https:// links"), true);
  const controller = new AbortController();
  if (!store.begin(msg("Downloading the plugin package"), () => controller.abort())) return;
  let file: string | null = null;
  try {
    file = await download(store, url, null, controller.signal, {});
    store.progress(msg("Checking the plugin package…"), null);
    const candidate = await store.plugin.packages.validateArchive(file);
    const info = installedOf(store, candidate.manifest.id);
    const { replace, question } = sideloadQuestion(store, candidate, { link: url });
    const answer = await store.ask(question);
    if (!answer?.confirmed) return;
    const name = candidate.manifest.name ?? candidate.manifest.id;
    await installCandidate(store, candidate, { kind: "link", url }, replace, name, replace ? null : info);
  } catch (error) {
    store.report(failureText(store, error), true);
  } finally {
    store.end();
    if (file) await rm(file, { force: true }).catch(() => {});
  }
}

// ------------------------------------------------------------ uninstall

export async function uninstall(store: Store, item: Item) {
  const info = item.installed;
  if (!info) return;
  const answer = await store.ask({
    message: msg("Uninstall “{}”?", item.name),
    detail: msg("Its menus, windows and processing effects are removed immediately."),
    confirm: "Uninstall",
    destructive: true,
    checkbox: "Also delete its settings and data?",
  });
  if (!answer?.confirmed) return;
  if (!store.begin(msg("Uninstalling {}…", item.name))) return;
  try {
    const change = await store.plugin.packages.uninstall(info.id, { removeData: answer.checked });
    await follow(store, change, "uninstall", () =>
      answer.checked ? msg("Uninstalled {} (data deleted)", item.name) : msg("Uninstalled {} (data kept)", item.name),
    );
  } catch (error) {
    store.report(errorCode(error) === "unsupported" ? msg("Built-in plugins cannot be uninstalled") : failureText(store, error, info.id), true);
  } finally {
    store.end();
  }
}
