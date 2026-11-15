import { randomUUID } from "node:crypto";
import { mkdir, rename, rm } from "node:fs/promises";
import { basename, join } from "node:path";
import type { PackageCandidate, PackageChange, PackageSource, PluginInfo } from "@vestra-player/sdk";
import type { Item } from "./catalog.ts";
import type { IndexVersion } from "./index-format.ts";
import { MAX_PACKAGE } from "./index-format.ts";
import { addedBy, bulletList, categoryList, reservedId, reservedText, sourceName } from "./labels.ts";
import { get, isHttps, NetError } from "./net.ts";
import { compareVersions, isVersion } from "./semver.ts";
import type { Store } from "./store.ts";
import { joined, msg, raw, type Arg, type Msg } from "./text.ts";

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

/**
 * What the plugin will be able to do: the categories the official
 * repository's scan found in its code (`scan`, from its signed index).
 * Without a scan (an index from before it): `null`, nothing is said; the
 * capabilities a package declares are not shown.
 */
function willBeAble(scan: { uses: string[] } | null): Msg | null {
  if (!scan) return null;
  return scan.uses.length
    ? msg("It will be able to:\n{}", categoryList(scan.uses))
    : msg("It uses none of the player's sensitive features.");
}

/**
 * Said in place of what it will be able to do for every package not from
 * the official repository (another repository, a file, a link; user ruling
 * 2026-11-13): nothing checked its code. Another repository's own scan is
 * its claim, so it is not shown.
 */
const UNCHECKED = msg("It is not from the official repository, so its permissions were not checked.");

/** Said of every plugin, before what it will be able to do (an unofficial repository's own text says it already). */
function withAccess(able: Msg | null): Arg {
  const access = msg("Plugins may read your files or use the network.");
  return able ? joined([access, able], "{}\n\n{}") : access;
}

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
  // Only the official repository's scan is shown; another repository's package gets the note instead.
  const perms = repo.official ? willBeAble(version.scan) : null;
  let question: Question;
  if (replace) {
    question = {
      message: msg("Replace the installed “{}”?", name),
      detail: repo.official
        ? msg("{1}\n\nWill install {2} from the official repository.\n\n{3}", replaceDetail(info, name), version.version, withAccess(perms))
        : msg(
            "{1}\n\nWill install {2} from the unofficial repository.\n\n{3}",
            replaceDetail(info, name),
            version.version,
            joined([unofficial, UNCHECKED], "{}\n\n{}"),
          ),
      confirm: "Replace",
      destructive: true,
    };
  } else if (sameSource && info.version && isVersion(info.version)) {
    const order = compareVersions(version.version, info.version);
    // What the window's notice names too (labels.ts); another repository's scans are not compared.
    const lines = repo.official ? addedBy(entry.versions, info, version) : [];
    // Another repository's update or reinstall: the sentence and the note, after the current version.
    const note = repo.official ? null : withAccess(UNCHECKED);
    const added = lines.length ? bulletList(lines) : null;
    const current = info.version;
    if (order > 0)
      question = added
        ? {
            message: msg("Update “{}” to {}? The new version needs more permissions", name, version.version),
            detail: msg(
              "Added:\n{2}\n\nCurrent: {3}.",
              added,
              current,
            ),
            confirm: "Update",
          }
        : {
            message: msg("Update “{}” to {}?", name, version.version),
            detail: note ? msg("Current: {2}.\n\n{3}", current, note) : msg("Current: {2}.", current),
            confirm: "Update",
          };
    // Never older than the installed version from the same source (the Versions tab offers none): uninstall, then install a .vxp.
    else if (order < 0) return;
    else
      question = {
        message: msg("Install “{}” {} (reinstall)?", name, version.version),
        detail: added
          ? msg("Current: {2}.\n\nThis version needs more permissions:\n{3}", current, added)
          : note
            ? msg("Current: {2}.\n\n{3}", current, note)
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
            withAccess(perms),
          ),
          confirm: "Install",
        }
      : {
          message: msg("Install “{}” {} from an unofficial repository?", name, version.version),
          detail: msg("{2}\n\n{3}", unofficial, UNCHECKED),
          confirm: "Install Anyway",
          destructive: true,
        };
  if (store.busy) return store.report(msg("Another install is in progress; wait for it to finish"), true);
  const answer = await store.ask(question);
  if (!answer?.confirmed) return;

  const controller = new AbortController();
  if (!store.begin(msg("Downloading {} {}", name, version.version), () => controller.abort())) return;
  // Its card shows the progress in place of its button (design store-v4 §3); a replacement from another
  // repository shows it on the installed plugin's card, which stands for this entry under All.
  store.installing = { key: item.key, id: item.id, percent: null };
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
    // A revoked version the store disabled: its fix runs (design §3.10).
    const enable = store.revocations.disabledByStore(item);
    await installCandidate(store, candidate, { kind: "repository", url: repo.url }, replace, name, sameSource ? info : null, enable);
  } catch (error) {
    store.report(failureText(store, error, item.id), true);
  } finally {
    store.installing = null;
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
      if (label && store.installing) store.installing.percent = percent;
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
 *
 * A plugin the user disabled stays disabled after an update or reinstall
 * (also one that replaces it from another source), as in VS Code. Players
 * after 0.6.3 keep the disabled flag across an install themselves; older
 * ones clear it when the install commits, so the store disables the plugin
 * again right after the commit (harmless on a newer player). `enable` is
 * for a revoked version the store disabled, whose fix is meant to run: the
 * store enables it after the commit (a newer player kept it disabled; on an
 * older one it already runs), waits for it to leave `starting` and reports
 * an activation failure. The result is described from the plugin list read
 * again after the commit, not from what the store saw before.
 */
async function installCandidate(
  store: Store,
  candidate: PackageCandidate,
  source: PackageSource,
  replace: boolean,
  name: string,
  previous: PluginInfo | null,
  enable = false,
) {
  const version = candidate.manifest.version;
  const id = candidate.manifest.id;
  // Whatever is installed now (any source; read again, the list may be old): a disabled plugin stays disabled.
  await store.reread();
  const keepDisabled = !enable && installedOf(store, id)?.state === "disabled";
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
  if (await follow(store, change, "install")) {
    const upgraded = previous?.version && isVersion(previous.version) && compareVersions(version, previous.version) > 0;
    let failure: Msg | null = null;
    if (enable || keepDisabled)
      try {
        await store.plugin.packages.setEnabled(id, enable);
      } catch (error) {
        failure = failureText(store, error, id);
      }
    // The player's word on the result, not the list from before the install. Enabling only starts the
    // plugin: wait for it to leave `starting` (it may still fail to activate).
    const now = await settled(store, id);
    const disabled = now?.state === "disabled";
    if (failure === null && enable && now?.state === "failed") failure = now.error ? raw(now.error) : msg("Failed");
    if (failure !== null)
      store.report(
        enable
          ? upgraded
            ? msg("Updated {} to {}, but couldn't enable it: {}", name, version, failure)
            : msg("Installed {} {}, but couldn't enable it: {}", name, version, failure)
          : upgraded
            ? msg("Updated {} to {}, but couldn't keep it disabled: {}", name, version, failure)
            : msg("Installed {} {}, but couldn't keep it disabled: {}", name, version, failure),
        true,
      );
    else if (disabled)
      store.report(
        upgraded ? msg("Updated {} to {} (still disabled)", name, version) : msg("Installed {} {} (still disabled)", name, version),
      );
    else store.report(upgraded ? msg("Updated {} to {}", name, version) : msg("Installed {} {}", name, version));
  }
  // From a repository the row was already selected; it stays on its tab (the list picks its neighbour if it
  // left). A file or link has no row yet: show it where it lands.
  if (source.kind !== "repository") store.select(selectKey(store, candidate.manifest.id, source));
}

/** How long the store waits for an enabled plugin to leave `starting`, and how often it reads the list meanwhile. */
export const installTiming = { startWait: 5_000, poll: 250 };

/** The installed plugin `id` as the player reports it, once it left `starting` (or after `installTiming.startWait`). */
async function settled(store: Store, id: string): Promise<PluginInfo | null> {
  await store.reread();
  for (const deadline = Date.now() + installTiming.startWait; installedOf(store, id)?.state === "starting" && Date.now() < deadline; ) {
    await new Promise((resolve) => setTimeout(resolve, installTiming.poll));
    await store.reread();
  }
  return installedOf(store, id);
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

/** Follow a change to its end; reports every outcome but a commit, which the caller reports (true). */
async function follow(store: Store, change: PackageChange, verb: Verb): Promise<boolean> {
  const words = FOLLOW[verb];
  store.progress(change.status === "activating" ? words.activating : words.preparing, () => void change.cancel().catch(() => {}));
  change.onChange((status) => {
    if (status === "activating") store.progress(words.activating);
  });
  const status = await change.done;
  switch (status) {
    case "committed":
      return true;
    case "tooLate":
      store.report(words.tooLate);
      return false;
    case "failedDegraded":
      // Only an install reports this (the player kept what was installed before); an uninstall never does.
      if ("failedDegraded" in words) {
        store.report(words.failedDegraded, true);
        return false;
      }
      break;
    case "cancelled":
      store.report(words.cancelled);
      return false;
  }
  store.report(words.failed(raw(change.message ?? status)), true);
  return false;
}

// ------------------------------------------------------------ side-loading

function sideloadQuestion(store: Store, candidate: PackageCandidate, from: { file: string } | { link: string }) {
  const manifest = candidate.manifest;
  const name = manifest.name ?? manifest.id;
  const info = installedOf(store, manifest.id);
  // A package from a file or a link has no scan: the sentence and the note.
  const perms = withAccess(UNCHECKED);
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
    if (await follow(store, change, "uninstall"))
      store.report(answer.checked ? msg("Uninstalled {} (data deleted)", item.name) : msg("Uninstalled {} (data kept)", item.name));
  } catch (error) {
    store.report(errorCode(error) === "unsupported" ? msg("Built-in plugins cannot be uninstalled") : failureText(store, error, info.id), true);
    // A refusal (`missing`, `unsupported`, `busy`) changes nothing `packages.onChange` would report: read the list again.
    await store.reread();
  } finally {
    store.end();
  }
}
