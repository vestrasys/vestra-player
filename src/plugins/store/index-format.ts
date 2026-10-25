import { isHttps, isLocalHttp } from "./net.ts";
import { compareVersions, isVersion, parseRange } from "./semver.ts";
import { msg, raw, type Msg } from "./text.ts";

/**
 * Reading a repository's index.json (ecosystem plan 1.3; the plugins
 * repository's `indexProblems`), but lenient where that is safe (design
 * §3.4): unknown keys are ignored, a bad plugin, version or revocation entry
 * is dropped on its own, display text is cut to the manifest limits. Only the
 * top level (`schema`, `plugins`, `revoked`) rejects the whole file.
 */

export const SCHEMA = 1;
export const MAX_PACKAGE = 64 * 1024 * 1024;
const MAX_PLUGINS = 2000;
const MAX_VERSIONS = 200;
const MAX_REVOKED = 1000;
const ID = /^[A-Za-z0-9._-]{1,128}$/;
const SHA256 = /^[0-9a-f]{64}$/;

export interface IndexVersion {
  version: string;
  apiVersion: string;
  minPlayerVersion: string | null;
  capabilities: string[];
  downloadUrl: string;
  sha256: string;
  size: number;
  published: string;
}

/** Expected size and sha256 of a store asset (README, changelog, icon); `null`: not given (older index). */
export type AssetCheck = { sha256: string; size: number } | null;

export interface IndexEntry {
  id: string;
  name: string;
  author: string | null;
  punchline: string | null;
  description: string | null;
  tags: string[];
  license: string | null;
  iconUrl: string | null;
  readmeUrl: string | null;
  changelogUrl: string | null;
  sourceUrl: string | null;
  iconCheck: AssetCheck;
  readmeCheck: AssetCheck;
  changelogCheck: AssetCheck;
  /** Newest first (sorted here, not trusted from the server). */
  versions: IndexVersion[];
}

export interface Revocation {
  id: string;
  versions: string;
  reason: string;
}

export interface RepoIndex {
  /** The publisher's sequence number; 0 when missing or not a positive integer (an index from before the field). */
  generated: number;
  name: string;
  plugins: IndexEntry[];
  revoked: Revocation[];
}

export class IndexError extends Error {
  readonly kind: "format" | "schema";
  /** The reason for the user; `message` is the same in English, for logs. */
  readonly reason: Msg;
  constructor(kind: "format" | "schema", reason: Msg) {
    super(reason.text.replace("{}", String(reason.args[0] ?? "")));
    this.kind = kind;
    this.reason = reason;
  }
}

/** Bidirectional overrides and zero-width characters: they can make a name read as another one. */
const INVISIBLE = /[\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g;

/** Control and invisible characters out; one line unless `multiline`; at most `max` characters. */
export function clean(value: unknown, max: number, multiline = false): string | null {
  if (typeof value !== "string") return null;
  let text = value.replace(INVISIBLE, "").replace(/\r\n?/g, "\n");
  text = multiline ? text.replace(/[\u0000-\u0009\u000b-\u001f\u007f]/g, "") : text.replace(/[\u0000-\u001f\u007f]+/g, " ");
  text = text.trim();
  if (!text) return null;
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export interface ParseOptions {
  /** The repository is on this machine: its own http://localhost links are fine. */
  allowLocalHttp?: boolean;
}

/** Parses index.json text. Throws `IndexError` only for the top level. */
export function parseIndex(text: string, options: ParseOptions = {}): { index: RepoIndex; dropped: string[] } {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    throw new IndexError("format", msg("Not JSON"));
  }
  if (!object(data)) throw new IndexError("format", msg("The top level is not an object"));
  if (data.schema !== SCHEMA)
    throw typeof data.schema === "number" && data.schema > SCHEMA
      ? new IndexError("schema", raw(`schema ${data.schema}`))
      : new IndexError("format", msg("Wrong schema"));
  if (!Array.isArray(data.plugins)) throw new IndexError("format", msg("plugins is not an array"));
  if (data.revoked !== undefined && !Array.isArray(data.revoked)) throw new IndexError("format", msg("revoked is not an array"));
  const url = (value: unknown): string | null =>
    typeof value === "string" && value.length <= 2048 && (isHttps(value) || (options.allowLocalHttp === true && isLocalHttp(value)))
      ? new URL(value).href
      : null;
  const dropped: string[] = [];
  const plugins: IndexEntry[] = [];
  const ids = new Set<string>();
  for (const [i, item] of data.plugins.slice(0, MAX_PLUGINS).entries()) {
    if (!object(item) || typeof item.id !== "string" || !ID.test(item.id)) {
      dropped.push(`plugins[${i}]: id`);
      continue;
    }
    if (ids.has(item.id.toLowerCase())) {
      dropped.push(`plugins[${i}]: duplicate ${item.id}`);
      continue;
    }
    const versions: IndexVersion[] = [];
    const seen = new Set<string>();
    for (const [j, entry] of (Array.isArray(item.versions) ? item.versions : []).slice(0, MAX_VERSIONS).entries()) {
      const version = readVersion(entry, url);
      if (!version || seen.has(version.version)) {
        dropped.push(`${item.id} versions[${j}]`);
        continue;
      }
      seen.add(version.version);
      versions.push(version);
    }
    if (!versions.length) {
      dropped.push(`${item.id}: no usable version`);
      continue;
    }
    versions.sort((a, b) => compareVersions(b.version, a.version));
    ids.add(item.id.toLowerCase());
    plugins.push({
      id: item.id,
      name: clean(item.name, 64) ?? item.id,
      author: clean(item.author, 64),
      punchline: clean(item.punchline, 120),
      description: clean(item.description, 2000, true),
      tags: Array.isArray(item.tags)
        ? [...new Set(item.tags.map((tag) => clean(tag, 24)).filter((tag): tag is string => tag !== null))].slice(0, 8)
        : [],
      license: clean(item.license, 64),
      ...asset(item, "icon", url),
      ...asset(item, "readme", url),
      ...asset(item, "changelog", url),
      sourceUrl: url(item.sourceUrl),
      versions,
    });
  }
  const revoked: Revocation[] = [];
  const listed = (data.revoked as unknown[] | undefined) ?? [];
  // Fail closed: ignoring revocations past a cap would let an index hide them.
  if (listed.length > MAX_REVOKED) throw new IndexError("format", msg("revoked has more than {} entries", MAX_REVOKED));
  for (const [i, item] of listed.entries()) {
    const reason = object(item) ? clean(item.reason, 500, true) : null;
    let range = false;
    try {
      range = object(item) && parseRange(item.versions).length > 0;
    } catch {
      range = false;
    }
    if (!object(item) || typeof item.id !== "string" || !ID.test(item.id) || !range || !reason) {
      dropped.push(`revoked[${i}]`);
      continue;
    }
    revoked.push({ id: item.id, versions: item.versions as string, reason });
  }
  const generated = Number.isSafeInteger(data.generated) && (data.generated as number) >= 1 ? (data.generated as number) : 0;
  return { index: { generated, name: clean(data.name, 64) ?? "", plugins, revoked }, dropped };
}

/**
 * `<name>Url` with its `<name>Sha256` / `<name>Size`. Both null or missing:
 * no check (an index from before the fields). Present but malformed (or only
 * one of them): the asset is unusable (URL dropped), the plugin stays.
 */
function asset<N extends "icon" | "readme" | "changelog">(
  item: Record<string, unknown>,
  name: N,
  url: (value: unknown) => string | null,
): Record<`${N}Url`, string | null> & Record<`${N}Check`, AssetCheck> {
  const hash = item[`${name}Sha256`] ?? null;
  const size = item[`${name}Size`] ?? null;
  let address = url(item[`${name}Url`]);
  let check: AssetCheck = null;
  if (hash !== null || size !== null) {
    if (typeof hash === "string" && SHA256.test(hash) && Number.isSafeInteger(size) && (size as number) >= 0)
      check = { sha256: hash, size: size as number };
    else address = null;
  }
  return { [`${name}Url`]: address, [`${name}Check`]: check } as unknown as Record<`${N}Url`, string | null> &
    Record<`${N}Check`, AssetCheck>;
}

function readVersion(entry: unknown, url: (value: unknown) => string | null): IndexVersion | null {
  if (!object(entry) || !isVersion(entry.version)) return null;
  if (typeof entry.apiVersion !== "string" || !entry.apiVersion) return null;
  const min = entry.minPlayerVersion ?? null;
  if (min !== null && !isVersion(min)) return null;
  if (!Array.isArray(entry.capabilities) || entry.capabilities.some((c) => typeof c !== "string")) return null;
  const downloadUrl = url(entry.downloadUrl);
  if (!downloadUrl) return null;
  if (typeof entry.sha256 !== "string" || !SHA256.test(entry.sha256)) return null;
  if (!Number.isSafeInteger(entry.size) || (entry.size as number) <= 0 || (entry.size as number) > MAX_PACKAGE) return null;
  if (typeof entry.published !== "string" || Number.isNaN(Date.parse(entry.published))) return null;
  return {
    version: entry.version,
    apiVersion: entry.apiVersion,
    minPlayerVersion: min as string | null,
    capabilities: [...new Set(entry.capabilities as string[])].slice(0, 64),
    downloadUrl,
    sha256: entry.sha256,
    size: entry.size as number,
    published: entry.published,
  };
}

/**
 * What a repository's signed indexes have shown so far (kept with the cache):
 * every version of every plugin and every revocation (id and range). It only
 * grows; a plugin that disappears keeps its versions here.
 */
export type Seen = { versions: Record<string, string[]>; revoked: string[] };

const revocationId = (r: Revocation) => `${r.id}\n${r.versions}`;

/** `seen` plus what `index` lists. */
export function remember(seen: Seen | undefined, index: RepoIndex): Seen {
  const versions: Record<string, string[]> = { ...(seen?.versions ?? {}) };
  for (const plugin of index.plugins)
    versions[plugin.id] = [...new Set([...(versions[plugin.id] ?? []), ...plugin.versions.map((v) => v.version)])];
  const revoked = [...new Set([...(seen?.revoked ?? []), ...index.revoked.map(revocationId)])];
  return { versions, revoked };
}

/**
 * An older signed index served again (design §3.4, §6.1). The publishing job
 * keeps every version entry of a listed plugin and `revoked` is append-only,
 * so a fresh index never
 * - lists a plugin without a version seen before (a delisted plugin may be
 *   absent, but when it comes back its old versions must be there), or
 * - lacks a revocation seen before.
 * Returns what is missing, or `null`.
 */
export function replayed(seen: Seen | undefined, next: RepoIndex): string | null {
  if (!seen) return null;
  for (const plugin of next.plugins) {
    const before = seen.versions[plugin.id];
    if (!before) continue;
    const now = new Set(plugin.versions.map((v) => v.version));
    const lost = before.find((version) => !now.has(version));
    if (lost) return `${plugin.id} ${lost}`;
  }
  const now = new Set(next.revoked.map(revocationId));
  const lost = seen.revoked.find((r) => !now.has(r));
  if (lost) return `revoked ${lost.replace("\n", " ")}`;
  return null;
}
