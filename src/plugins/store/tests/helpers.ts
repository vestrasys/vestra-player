/**
 * Test fixtures: a signing key pair made here (never the real one), signed
 * indexes, fake packages and a fake network keyed by URL. Nothing leaves the
 * machine: the store under test gets `net.fetch` instead of the real fetch.
 */
import { createHash, generateKeyPairSync, sign, type KeyObject } from "node:crypto";
import { readFileSync } from "node:fs";
import type { PackageManifest, Plugin } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import type { Json, Locale } from "@vestra-player/sdk";
import { startStore } from "../main.ts";
import type { Store } from "../store.ts";
import { timing } from "../store.ts";
import { textOf, type Msg } from "../text.ts";

export const directory = new URL("..", import.meta.url);

// The automatic check stays out of the way unless a test shortens it.
Object.assign(timing, { firstCheck: 1_000_000_000, tick: 1_000_000_000, interval: 86_400_000 });

export const pair = generateKeyPairSync("ed25519");
export const other = generateKeyPairSync("ed25519");
export const signWith = (bytes: Uint8Array, key: KeyObject = pair.privateKey) => sign(null, bytes, key).toString("base64");

export const OFFICIAL = "https://plugins.test/index.json";
export const THIRD = "https://third.example/repo/index.json";
export const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** A fake .vxp: the player is simulated, so the "archive" is its manifest as JSON. */
export function vxp(manifest: Partial<PackageManifest> & { id: string; version: string }): Buffer {
  return Buffer.from(
    JSON.stringify({ apiVersion: "vestra/1", entry: "main.js", capabilities: [], ...manifest }),
  );
}

export type VersionSpec = {
  version: string;
  capabilities?: string[];
  minPlayerVersion?: string | null;
  apiVersion?: string;
  published?: string;
  /** Override what the server sends for the package (size and sha256 stay those of the real one). */
  body?: Buffer;
  /** The package's own manifest declares these instead (a package that differs from its entry). */
  packageCapabilities?: string[];
  /** The package's own manifest says this. */
  packageMinPlayerVersion?: string;
  name?: string;
};

export type PluginSpec = {
  id: string;
  name?: string;
  author?: string;
  tags?: string[];
  readme?: string | null;
  changelog?: string | null;
  iconUrl?: string | null;
  /** Served at iconUrl. */
  icon?: Buffer;
  /** Publish iconSha256/iconSize, readmeSha256/… like the current plugins repository. */
  hashes?: boolean;
  versions: VersionSpec[];
};

/** The index JSON for these plugins, with packages served by `net`. */
export function buildIndex(
  net: FakeNet,
  base: string,
  plugins: PluginSpec[],
  revoked: { id: string; versions: string; reason: string }[] = [],
  extra: Record<string, unknown> = {},
): Buffer {
  const root = new URL(".", base).href;
  const index = {
    schema: 1,
    name: "Test 仓库",
    official: true,
    ...extra,
    plugins: plugins.map((p) => {
      const readmeUrl = p.readme === null ? null : `${root}assets/${p.id}/README.md`;
      const changelogUrl = p.changelog === null || p.changelog === undefined ? null : `${root}assets/${p.id}/CHANGELOG.md`;
      const readme = Buffer.from(p.readme ?? `# ${p.name ?? p.id}\n\n说明正文。`);
      if (readmeUrl) net.route(readmeUrl, { body: readme });
      if (changelogUrl) net.route(changelogUrl, { body: p.changelog! });
      if (p.iconUrl && p.icon) net.route(p.iconUrl, { body: p.icon });
      const check = (url: string | null | undefined, bytes: Buffer | undefined) =>
        url && bytes ? { sha256: sha256(bytes), size: bytes.length } : { sha256: null, size: null };
      const hashes = p.hashes
        ? Object.fromEntries(
            (
              [
                ["icon", check(p.iconUrl, p.icon)],
                ["readme", check(readmeUrl, readme)],
                ["changelog", check(changelogUrl, p.changelog ? Buffer.from(p.changelog) : undefined)],
              ] as const
            ).flatMap(([name, c]) => [
              [`${name}Sha256`, c.sha256],
              [`${name}Size`, c.size],
            ]),
          )
        : {};
      return {
        id: p.id,
        name: p.name ?? p.id,
        author: p.author ?? "Someone",
        punchline: `${p.name ?? p.id} 的简介`,
        description: null,
        tags: p.tags ?? [],
        license: "MIT",
        iconUrl: p.iconUrl ?? null,
        readmeUrl,
        changelogUrl,
        ...hashes,
        sourceUrl: `https://github.com/example/${p.id}`,
        versions: p.versions.map((v) => {
          const capabilities = v.capabilities ?? ["commands"];
          const body = vxp({
            id: p.id,
            version: v.version,
            capabilities: v.packageCapabilities ?? capabilities,
            name: v.name ?? p.name ?? p.id,
            ...(v.packageMinPlayerVersion ? { minPlayerVersion: v.packageMinPlayerVersion } : {}),
          });
          const url = `${root}download/${p.id}-${v.version}.vxp`;
          net.route(url, { body: v.body ?? body });
          return {
            version: v.version,
            apiVersion: v.apiVersion ?? "vestra/1",
            minPlayerVersion: v.minPlayerVersion === undefined ? "0.2.0" : v.minPlayerVersion,
            capabilities,
            downloadUrl: url,
            sha256: sha256(body),
            size: body.length,
            published: v.published ?? "2026-10-21T00:00:00.000Z",
          };
        }),
      };
    }),
    revoked,
    extensions: {},
  };
  return Buffer.from(`${JSON.stringify(index, null, 2)}\n`);
}

/** Serve a signed index at `url` (and `url.sig`). */
export function publish(
  net: FakeNet,
  url: string,
  plugins: PluginSpec[],
  options: { revoked?: { id: string; versions: string; reason: string }[]; signature?: string | null; extra?: Record<string, unknown> } = {},
): Buffer {
  const bytes = buildIndex(net, url, plugins, options.revoked, options.extra);
  net.route(url, { body: bytes, headers: { etag: `"${sha256(bytes).slice(0, 8)}"` } });
  if (options.signature === null) net.route(`${url}.sig`, { status: 404 });
  else net.route(`${url}.sig`, { body: options.signature ?? `${signWith(bytes)}\n` });
  return bytes;
}

export type Route = {
  status?: number;
  headers?: Record<string, string>;
  body?: Buffer | string;
  /** Sent as separate chunks (a streamed body). */
  chunks?: Buffer[];
  /** Wait before answering (ms). */
  delay?: number;
  /** Omit Content-Length. */
  noLength?: boolean;
};

/** A fetch that answers from a route table; unknown addresses are 404. */
export class FakeNet {
  readonly routes = new Map<string, Route | ((headers: Record<string, string>) => Route)>();
  readonly requests: { url: string; headers: Record<string, string> }[] = [];
  offline = false;

  route(url: string, route: Route | ((headers: Record<string, string>) => Route)) {
    this.routes.set(url, route);
  }

  count(url: string) {
    return this.requests.filter((r) => r.url === url).length;
  }

  fetch = async (url: string, init: RequestInit): Promise<Response> => {
    const headers = Object.fromEntries(
      Object.entries((init.headers ?? {}) as Record<string, string>).map(([k, v]) => [k.toLowerCase(), v]),
    );
    this.requests.push({ url, headers });
    if (this.offline) throw new TypeError("fetch failed", { cause: { code: "ENOTFOUND", message: "getaddrinfo ENOTFOUND" } });
    const entry = this.routes.get(url);
    const route = typeof entry === "function" ? entry(headers) : (entry ?? { status: 404 });
    if (route.delay)
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(resolve, route.delay);
        init.signal?.addEventListener("abort", () => {
          clearTimeout(timer);
          reject(new DOMException("aborted", "AbortError"));
        });
      });
    const chunks = route.chunks ?? (route.body === undefined ? [] : [Buffer.from(route.body)]);
    const length = chunks.reduce((n, c) => n + c.length, 0);
    const status = route.status ?? 200;
    const nobody = status === 304 || status === 204 || (status >= 300 && status < 400);
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(new Uint8Array(chunk));
        controller.close();
      },
    });
    return new Response(nobody ? null : stream, {
      status,
      headers: { ...(route.noLength || nobody ? {} : { "content-length": String(length) }), ...route.headers },
    });
  };
}

export type Info = TestHost["packages"][number];
export const info = (id: string, patch: Partial<Info> = {}): Info => ({
  id,
  version: "0.1.0",
  origin: "installed",
  state: "running",
  error: null,
  capabilities: ["commands"],
  directory: `/packages/${id}`,
  previousVersion: null,
  builtin: false,
  source: { kind: "file" },
  dataDirectory: null,
  ...patch,
});

export const BUILTIN = info("vestra.video", { version: "0.2.0", builtin: true, source: { kind: "bundled" } });

/**
 * The store on a TestHost (player 0.2.0) with the fake network and the test
 * key. `validateArchive` reads the fake package file the store downloaded
 * (and refuses reserved `vestra.` ids like the player);
 * `rollback` swaps current and previous like the player.
 */
export async function start(
  options: {
    net?: FakeNet;
    packages?: Info[];
    dataDirectory?: string;
    player?: string;
    host?: TestHost;
    now?: () => number;
    keys?: KeyObject[];
    /** The interface language; the store's tests read Simplified Chinese unless they say otherwise. */
    locale?: Locale;
  } = {},
) {
  const net = options.net ?? new FakeNet();
  const host = options.host ?? new TestHost();
  host.app.version = options.player ?? "0.2.0";
  host.packages = options.packages ?? [BUILTIN];
  const logs: string[] = [];
  host.handle("packages.validateArchive", (p) => {
    const file = String(p.file);
    let manifest = host.candidates.get(file);
    if (!manifest) {
      try {
        manifest = JSON.parse(readFileSync(file, "utf8")) as PackageManifest;
      } catch {
        throw new HostError("invalid", "not a Vestra plugin package");
      }
    }
    // Like the player (0.4.1): vestra / vestra.* belong to built-in plugins.
    if (/^vestra(\.|$)/i.test(manifest.id))
      throw new HostError("reserved", `「${manifest.id}」：vestra. 开头的 id 只留给播放器内置插件，不能安装或试载`);
    if (manifest.minPlayerVersion && manifest.minPlayerVersion > host.app.version)
      throw new HostError("versionMismatch", `requires Vestra Player ≥ ${manifest.minPlayerVersion}`);
    host.candidates.set(file, manifest);
    return { token: file, digest: "0".repeat(64), apiVersion: "vestra/1", manifest, checks: [], trustedExecution: true };
  });
  host.handle("packages.rollback", (p) => {
    const entry = host.packages.find((each) => each.id === p.id);
    if (!entry?.previousVersion) throw new HostError("missing", "no previous successful version");
    host.setPackages(
      host.packages.map((each) =>
        each.id === p.id ? { ...each, version: entry.previousVersion, previousVersion: entry.version } : each,
      ),
    );
    return { operationId: String(p.requestId), status: "committed" };
  });
  let store!: Store;
  const started = await startPlugin(
    async (plugin: Plugin) => {
      store = await startStore(plugin, {
        fetch: net.fetch,
        keys: options.keys ?? [pair.publicKey],
        now: options.now ?? (() => Date.now()),
        officialUrl: OFFICIAL,
        log: (line) => logs.push(line),
      });
    },
    {
      directory,
      host,
      locale: options.locale ?? "zh-Hans",
      ...(options.dataDirectory ? { dataDirectory: options.dataDirectory } : {}),
    },
  );
  await host.settle();
  latest = { host, plugin: started.plugin };
  return { host, net, store, logs, plugin: started.plugin, stop: started.stop };
}

/** Wait until `check()` holds (the store's own network work is not awaited by settle). */
export async function until(host: TestHost, check: () => boolean, what = "condition") {
  // A time deadline, not a count of settle rounds: a loaded CI runner makes each round slower.
  // Once it holds, settle: the SDK writes changed window properties one host call at a time.
  for (const deadline = Date.now() + 15_000; Date.now() < deadline; ) {
    if (check()) return host.settle();
    await host.settle(10);
  }
  if (check()) return host.settle();
  throw new Error(`timed out waiting for ${what}`);
}

export const yes = { confirmed: true, checked: false };
export const no = { confirmed: false, checked: false };

/** Open the window and wait for the first refresh. */
export async function openStore(host: TestHost) {
  await host.command("open").run();
  await host.settle();
  const window = host.window("store");
  return window;
}

// ------------------------------------------------------------ window texts as shown

let latest: { host: TestHost; plugin: Plugin } | null = null;

/** A `Msg` as the player shows it, in the language of the store started last. */
export function said(value: Msg | null): string {
  const shown = textOf(latest!.plugin.i18n, value);
  return shown === "" ? "" : latest!.host.translate(shown);
}

/** The store window's props as shown (the test host renders text values), plus the status line's text. */
export function props(host: TestHost): Record<string, Json> {
  const out = { ...host.window("store").props } as Record<string, Json>;
  out["status-text"] = out["status"] ?? "";
  return out;
}

/** The settings page's props as shown. */
export const pageProps = (host: TestHost) => host.settingsPage("store.repos").props as Record<string, Json>;

export const rows = (host: TestHost) =>
  (props(host)["rows"] as { key: string; name: string; badge: string; version: string }[]) ?? [];

/** Select the row with this name in the store window. */
export async function choose(host: TestHost, name: string) {
  const row = rows(host).find((r) => r.name === name);
  if (!row) throw new Error(`no row ${name}: ${rows(host).map((r) => r.name).join(", ")}`);
  host.window("store").emit("select", row.key);
  await host.settle();
  return row;
}

export const actions = (host: TestHost) => (props(host)["actions"] as { id: string; text: string; enabled: boolean }[]) ?? [];
export const banners = (host: TestHost) => ((props(host)["banners"] as { text: string }[]) ?? []).map((b) => b.text);
