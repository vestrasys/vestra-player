import { randomUUID } from "node:crypto";
import { mkdir, rename, rm } from "node:fs/promises";
import { basename, join } from "node:path";
import type { PackageCandidate, PackageChange, PackageSource, PluginInfo } from "@vestra-player/sdk";
import type { Item } from "./catalog.ts";
import type { IndexVersion } from "./index-format.ts";
import { MAX_PACKAGE } from "./index-format.ts";
import { permissionList, reservedId, reservedText, sourceName, TRUST_NOTE } from "./labels.ts";
import { get, isHttps, NetError } from "./net.ts";
import { compareVersions, isVersion } from "./semver.ts";
import type { Store } from "./store.ts";

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
export function failureText(store: Store, error: unknown, id?: string): string {
  if (error instanceof NetError) {
    switch (error.code) {
      case "size":
        return "下载的文件与仓库描述不符（大小），已删除";
      case "sha256":
        return "下载的文件与仓库描述不符（校验和），已删除";
      case "tooLarge":
        return "插件包超过 64 MB，已停止下载";
      case "protocol":
      case "redirects":
        return "下载地址不安全或跳转过多，已停止";
      case "aborted":
        return "已取消下载";
      case "disk":
        return `无法写入下载文件：${error.message}`;
      default:
        return `下载失败：${error.message}`;
    }
  }
  switch (errorCode(error)) {
    case "invalid":
      return `这不是有效的插件包：${(error as Error).message}`;
    case "versionMismatch": {
      const needed = /(\d+\.\d+\.\d+)/.exec((error as Error).message)?.[1];
      return needed
        ? `这个插件需要 Vestra Player ≥ ${needed}（当前 ${store.plugin.app.version}）`
        : `这个插件需要更新的 Vestra Player（当前 ${store.plugin.app.version}）`;
    }
    case "reserved":
      return (error as Error).message || reservedText(id ?? "这个插件");
    case "unsupported":
      return `「${id ?? "这个插件"}」与播放器内置插件同 id，不能安装`;
    case "busy":
      return "这个插件正在安装或回退，请稍后再试";
    case "capability":
      return "插件商城缺少「管理插件」权限";
    case "missing":
      return "这个插件已不存在";
    default:
      return (error as Error).message || String(error);
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

function replaceDetail(info: PluginInfo, name: string): string {
  return `当前的「${name}」${info.version ?? ""} 来自${sourceName(info.source)}。替换后，若之前从这个来源安装过，会接上当时的数据；否则使用新的空数据。原插件的数据保留，以后从原来源重新安装时接上。`;
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
  const unofficial = repo.official
    ? ""
    : `仓库：${repo.label}（${repo.url}）。这个仓库未经 Vestra Player 审核，插件可能读取你的文件或联网。只安装你信任的作者的插件。${repo.local ? "这是本机上的测试仓库，没有加密。" : ""}\n\n`;
  const perms = `它将可以：\n${permissionList(version.capabilities)}\n\n${TRUST_NOTE}`;
  let question: { message: string; detail: string; confirm: string; destructive?: boolean };
  if (replace) {
    question = {
      message: `替换已安装的「${name}」？`,
      detail: `${replaceDetail(info, name)}\n\n将安装${repo.official ? "官方仓库" : "非官方仓库"}的 ${version.version}。\n\n${unofficial}${perms}`,
      confirm: "替换",
      destructive: true,
    };
  } else if (sameSource && info.version && isVersion(info.version)) {
    const order = compareVersions(version.version, info.version);
    const added = version.capabilities.filter((c) => !info.capabilities.includes(c));
    if (order > 0)
      question = added.length
        ? {
            message: `更新「${name}」到 ${version.version}？新版本需要更多权限`,
            detail: `新增：\n${permissionList(added)}\n\n当前 ${info.version}。更新后可以在商城回退到 ${info.version}。\n\n${TRUST_NOTE}`,
            confirm: "更新",
          }
        : {
            message: `更新「${name}」到 ${version.version}？`,
            detail: `当前 ${info.version}。更新后可以在商城回退到 ${info.version}。`,
            confirm: "更新",
          };
    else
      question = {
        message: `安装「${name}」${version.version}（${order < 0 ? "旧版本" : "重新安装"}）？`,
        detail: `当前 ${info.version}。安装后可以在商城${order < 0 ? "恢复" : "回退"}到 ${info.version}。${added.length ? `\n\n这个版本需要更多权限：\n${permissionList(added)}` : ""}`,
        confirm: "安装",
      };
  } else
    question = repo.official
      ? {
          message: `安装「${name}」${version.version}？`,
          detail: `来自 Vestra Player 官方仓库，作者 ${item.author ?? "未知"}。\n\n${perms}`,
          confirm: "安装",
        }
      : {
          message: `安装来自非官方仓库的「${name}」${version.version}？`,
          detail: `${unofficial}${perms}`,
          confirm: "仍然安装",
          destructive: true,
        };
  if (store.busy) return store.report("正在进行另一项安装，请等它结束", true);
  const answer = await store.ask(question);
  if (!answer?.confirmed) return;

  const controller = new AbortController();
  if (!store.begin(`正在下载 ${name} ${version.version}`, () => controller.abort())) return;
  let file: string | null = null;
  try {
    file = await download(store, version.downloadUrl, `${name} ${version.version}`, controller.signal, {
      exactSize: version.size,
      sha256: version.sha256,
      allowLocalHttp: repo.local,
    });
    store.progress(`正在检查 ${name}…`, null);
    const candidate = await store.plugin.packages.validateArchive(file);
    const mismatch = compare(candidate, item.id, version);
    if (mismatch) return store.report(`插件包与仓库描述不符（${mismatch}），未安装`, true);
    await installCandidate(store, candidate, { kind: "repository", url: repo.url }, replace, name, sameSource ? info : null);
  } catch (error) {
    store.report(failureText(store, error, item.id), true);
  } finally {
    store.end();
    if (file) await rm(file, { force: true }).catch(() => {});
  }
}

/** What differs between the package and the index entry the user agreed to. */
function compare(candidate: PackageCandidate, id: string, version: IndexVersion): string | null {
  const manifest = candidate.manifest;
  if (manifest.id !== id) return "id";
  if (manifest.version !== version.version) return "版本";
  const declared = new Set(version.capabilities);
  const actual = new Set(manifest.capabilities);
  if (declared.size !== actual.size || [...actual].some((c) => !declared.has(c))) return "权限";
  return null;
}

async function download(
  store: Store,
  url: string,
  label: string,
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
      store.progress(`正在下载 ${label}（${percent}%）`);
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
  store.progress(`正在安装 ${name}…`, null);
  let change: PackageChange;
  try {
    change = await candidate.install({ source, ...(replace ? { replace: true } : {}) });
  } catch (error) {
    if (errorCode(error) !== "conflict" || replace) throw error;
    // Installed from somewhere else since we asked: ask about replacing.
    const info = installedOf(store, candidate.manifest.id);
    const answer = await store.ask({
      message: `替换已安装的「${name}」？`,
      detail: info ? replaceDetail(info, name) : `这个插件已从其它来源安装。替换后，若之前从这个来源安装过，会接上当时的数据；否则使用新的空数据。原插件的数据保留，以后从原来源重新安装时接上。`,
      confirm: "替换",
      destructive: true,
    });
    if (!answer?.confirmed) return store.report("未安装");
    change = await candidate.install({ source, replace: true });
  }
  await follow(store, change, () => {
    const upgraded = previous?.version && isVersion(previous.version) && compareVersions(version, previous.version) > 0;
    return upgraded ? `已更新 ${name} 到 ${version}` : `已安装 ${name} ${version}`;
  });
  store.select(selectKey(store, candidate.manifest.id, source));
}

/** After an install: select the row that now stands for it. */
function selectKey(store: Store, id: string, source: PackageSource): string {
  if (source.kind === "repository") return `${source.url}\n${id}`;
  return `\n${id}`;
}

async function follow(store: Store, change: PackageChange, committed: () => string, verb = "安装") {
  store.progress(`正在${verb}…（${change.status === "activating" ? "激活中" : "准备中"}）`, () => void change.cancel().catch(() => {}));
  change.onChange((status) => {
    if (status === "activating") store.progress(`正在${verb}…（激活中）`);
  });
  const status = await change.done;
  switch (status) {
    case "committed":
      return store.report(committed());
    case "tooLate":
      return store.report(`已无法取消，${verb}继续进行`);
    case "failedDegraded":
      return store.report(`${verb}失败，已恢复原来的版本`, true);
    case "cancelled":
      return store.report(`已取消${verb}`);
    default:
      return store.report(`${verb}失败：${change.message ?? status}`, true);
  }
}

// ------------------------------------------------------------ side-loading

function sideloadQuestion(store: Store, candidate: PackageCandidate, from: string) {
  const manifest = candidate.manifest;
  const name = manifest.name ?? manifest.id;
  const info = installedOf(store, manifest.id);
  const perms = `它将可以：\n${permissionList(manifest.capabilities)}\n\n${TRUST_NOTE}`;
  const warning = `来自${from}，未经任何仓库审核，也不会自动更新。只安装你信任的作者的插件。`;
  if (info && identity(info.source) !== "manual")
    return {
      replace: true,
      question: {
        message: `用插件包「${name}」${manifest.version} 替换已安装的版本？`,
        detail: `${replaceDetail(info, name)}\n\n${warning}\n\n${perms}`,
        confirm: "替换",
        destructive: true,
      },
    };
  const current = info ? `当前已安装 ${info.version ?? ""}（同一来源，安装后可以回退）。\n\n` : "";
  return {
    replace: false,
    question: {
      message: `安装插件包「${name}」${manifest.version}？`,
      detail: `${warning}\n\n${current}${perms}`,
      confirm: "仍然安装",
      destructive: true,
    },
  };
}

/** 「从文件安装…」 and a `.vxp` dropped on the video area. */
export async function installFile(store: Store, path: string) {
  if (!store.begin(`正在检查 ${basename(path)}…`)) return;
  try {
    let candidate: PackageCandidate;
    try {
      candidate = await store.plugin.packages.validateArchive(path);
    } catch (error) {
      return store.report(failureText(store, error), true);
    }
    // validateArchive already refused reserved ids, so built-in ones too.
    const info = installedOf(store, candidate.manifest.id);
    const { replace, question } = sideloadQuestion(store, candidate, `文件 ${basename(path)}`);
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

/** 「从链接安装…」: https only, at most 64 MB; the permissions are known only after the download. */
export async function installFromLink(store: Store, text: string) {
  let url: string;
  try {
    url = new URL(text.trim()).href;
  } catch {
    return store.report("只能从 https:// 链接安装", true);
  }
  if (!isHttps(url)) return store.report("只能从 https:// 链接安装", true);
  const controller = new AbortController();
  if (!store.begin("正在下载插件包", () => controller.abort())) return;
  let file: string | null = null;
  try {
    file = await download(store, url, "插件包", controller.signal, {});
    store.progress("正在检查插件包…", null);
    const candidate = await store.plugin.packages.validateArchive(file);
    const info = installedOf(store, candidate.manifest.id);
    const { replace, question } = sideloadQuestion(store, candidate, `链接 ${url}`);
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

// ------------------------------------------------------------ rollback / uninstall

/** 「回退」 to an older previous version, 「恢复」 to a newer one (after installing an older version or a rollback). */
export function rollbackVerb(current: string | null, previous: string): "回退" | "恢复" {
  return current && isVersion(current) && isVersion(previous) && compareVersions(previous, current) > 0 ? "恢复" : "回退";
}

export async function rollback(store: Store, item: Item) {
  const info = item.installed;
  if (!info?.previousVersion) return;
  if (!store.begin(`正在${rollbackVerb(info.version, info.previousVersion)} ${item.name}…`)) return;
  try {
    const change = await store.plugin.packages.rollback(info.id);
    const verb = rollbackVerb(info.version, info.previousVersion);
    await follow(store, change, () => `已${verb} ${item.name} 到 ${info.previousVersion}`, verb);
  } catch (error) {
    store.report(errorCode(error) === "unsupported" ? "这个插件不能回退" : failureText(store, error, info.id), true);
  } finally {
    store.end();
  }
}

export async function uninstall(store: Store, item: Item) {
  const info = item.installed;
  if (!info) return;
  const answer = await store.ask({
    message: `卸载「${item.name}」？`,
    detail: "它的菜单、窗口和处理效果会立即撤销。",
    confirm: "卸载",
    destructive: true,
    checkbox: "同时删除它的设置与数据",
  });
  if (!answer?.confirmed) return;
  if (!store.begin(`正在卸载 ${item.name}…`)) return;
  try {
    const change = await store.plugin.packages.uninstall(info.id, { removeData: answer.checked });
    await follow(
      store,
      change,
      () => `已卸载 ${item.name}（${answer.checked ? "数据已删除" : "数据已保留"}）`,
      "卸载",
    );
  } catch (error) {
    store.report(errorCode(error) === "unsupported" ? "内置插件只能停用" : failureText(store, error, info.id), true);
  } finally {
    store.end();
  }
}

