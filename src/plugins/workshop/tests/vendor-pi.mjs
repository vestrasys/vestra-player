// Builds the vendored pi SDK the agent worker runs (vendor/pi/) and the
// license files of everything it bundles (licenses/, dependency-inventory.json).
//
//   npm run vendor:pi          rebuild and write into the plugin
//   node tests/vendor-pi.mjs   rebuild into a temporary directory and list differences
//
// Inputs are the exact versions in package-lock.json (pi-coding-agent, pi-ai,
// pi-agent-core 1.4.0). Left out of the bundle (STUBS): the extension loader
// (jiti), syntax highlighting and Mermaid rendering of the terminal UI, the
// model-glob matcher and the pi package manager's git URL parser (minimatch,
// hosted-git-info, lru-cache: BlueOak-1.0.0, not in the player's license
// policy). Bedrock and the CLI entry are never imported by the SDK entry below.
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** The pi API the worker uses; nothing else is reachable from the bundle. */
const ENTRY = `export {
  createAgentSession, ModelRuntime, SessionManager, SettingsManager, createExtensionRuntime,
  createReadToolDefinition, createWriteToolDefinition, createEditToolDefinition,
  createLsToolDefinition, createFindToolDefinition,
} from "@earendil-works/pi-coding-agent";
export { InMemoryCredentialStore } from "@earendil-works/pi-ai";
export { Type } from "typebox";
`;
/** Imported by pi but not needed by an SDK session without extensions or a terminal. */
const STUBS = ["jiti", "highlight.js", "grok-mermaid", "minimatch", "hosted-git-info", "lru-cache"];
/** Stubs that the terminal UI calls while loading (highlight.js registers languages): calls do nothing. */
const QUIET = new Set(["jiti", "highlight.js", "grok-mermaid"]);
/**
 * Packages whose npm tarball has no license file: the text from upstream,
 * kept in tests/upstream/. pi: the monorepo LICENSE (github.com/earendil-works/pi);
 * data-uri-to-buffer: the license section of its README; standardwebhooks:
 * libraries/LICENSE of github.com/standard-webhooks/standard-webhooks.
 */
const UPSTREAM = { pi: "pi-LICENSE", "data-uri-to-buffer": "data-uri-to-buffer-LICENSE", standardwebhooks: "standardwebhooks-LICENSE" };
/**
 * Which of pi's providers the settings page offers: every provider that takes
 * an API key, has models in pi's catalog and a fixed endpoint. Left out:
 * login-only providers (OAuth, subscriptions; user ruling: no subscription
 * login) and providers that need cloud credentials or account-specific
 * endpoints beyond a key (Bedrock, Vertex, Azure, Cloudflare).
 */
const EXCLUDED = new Map([
  // User ruling 2026-10-24: no coding-plan / token-plan / subscription
  // entries; pay-as-you-go APIs only. Users reach a plan themselves through
  // 「自定义 · OpenAI/Anthropic 格式」. Plan names per pi's provider table:
  // https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md
  ["kimi-coding", "Kimi For Coding plan"],
  ["zai", "Z.AI GLM Coding Plan (global); pi's only Z.AI endpoint"],
  ["zai-coding-cn", "Z.AI GLM Coding Plan (China)"],
  ["qwen-token-plan", "Qwen Token Plan"],
  ["qwen-token-plan-cn", "Qwen Token Plan (China)"],
  ["qwen-token-plan-individual", "Qwen Token Plan Individual"],
  ["xiaomi-token-plan-ams", "Xiaomi MiMo Token Plan"],
  ["xiaomi-token-plan-cn", "Xiaomi MiMo Token Plan"],
  ["xiaomi-token-plan-sgp", "Xiaomi MiMo Token Plan"],
  ["opencode-go", "OpenCode Go subscription"],
  ["radius", "Earendil's gateway, set up through /login (account)"],
  ["github-copilot", "subscription login"],
  ["amazon-bedrock", "AWS credentials; its SDK is not bundled"],
  ["google-vertex", "Google Cloud project and location"],
  ["azure", "account-specific endpoint"],
  ["cloudflare-ai-gateway", "account and gateway id in the endpoint"],
  ["cloudflare-workers-ai", "account id in the endpoint"],
]);
/**
 * The model list endpoint of an API family: OpenAI style `GET …/models`
 * (Bearer), Anthropic `GET …/v1/models` (x-api-key), Google `GET …/models`
 * (x-goog-api-key). Null when the family has none we know.
 */
export function listing(api, base) {
  if (api === "openai-completions" || api === "openai-responses") return { style: "openai", url: `${base}/models` };
  if (api === "mistral-conversations") return { style: "openai", url: `${base.replace(/\/v1$/, "")}/v1/models` };
  if (api === "anthropic-messages") return { style: "anthropic", url: `${base.replace(/\/v1$/, "")}/v1/models` };
  if (api === "google-generative-ai") return { style: "google", url: `${base}/models` };
  return null;
}
/** A provider pi adds later that calls itself a plan or subscription is left out too, until reviewed. */
const PLAN = /plan|coding|subscription/i;
export function offered(provider, models) {
  return (
    !!provider.auth?.apiKey &&
    !EXCLUDED.has(provider.id) &&
    !PLAN.test(`${provider.id} ${provider.name}`) &&
    models.length > 0 &&
    models.every((m) => m.baseUrl && !m.baseUrl.includes("{") && !PLAN.test(m.baseUrl))
  );
}

const stubPlugin = {
  name: "stub",
  setup(build) {
    const filter = new RegExp(`^(${STUBS.map((s) => s.replace(/[.]/g, "\\.")).join("|")})(/.*)?$`);
    build.onResolve({ filter }, (args) => ({ path: args.path, namespace: "stub" }));
    build.onLoad({ filter: /.*/, namespace: "stub" }, (args) => ({
      // Imported at load time, never needed by an SDK session; a call to a
      // loud stub fails, a call to a quiet one does nothing.
      contents: `const fail = ${QUIET.has(args.path.split("/")[0]) ? "() => stub" : `() => { throw new Error(${JSON.stringify(`${args.path} is not part of the vendored pi SDK`)}); }`};
const stub = new Proxy(function () {}, { get: (_, key) => (key === "__esModule" ? undefined : stub), apply: fail, construct: fail });
module.exports = stub;`,
      loader: "js",
    }));
  },
};

function packageOf(input) {
  const at = input.lastIndexOf("node_modules/");
  if (at < 0) return null;
  const rest = input.slice(at + "node_modules/".length).split("/");
  const name = rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0];
  return { name, directory: path.join(plugin, input.slice(0, at + "node_modules/".length) + name) };
}

async function licenseFile(directory) {
  const names = (await fs.readdir(directory)).filter((n) => /^(licen[cs]e(-mit)?|copying)(\.(md|txt))?$/i.test(n)).sort();
  return names[0] ?? null;
}

/** Writes the vendored files under `out` (a plugin-shaped directory). */
export async function vendor(out) {
  const esbuild = await import("esbuild");
  const result = await esbuild.build({
    stdin: { contents: ENTRY, resolveDir: plugin, loader: "js", sourcefile: "pi-entry.mjs" },
    bundle: true,
    minify: true,
    platform: "node",
    format: "esm",
    target: "node22",
    legalComments: "none",
    metafile: true,
    write: false,
    outfile: path.join(out, "vendor/pi/pi.mjs"),
    plugins: [stubPlugin],
    logLevel: "error",
    banner: {
      js: 'import { createRequire as __createRequire } from "node:module"; const require = __createRequire(import.meta.url);',
    },
  });
  const [bundle] = result.outputFiles;
  await fs.mkdir(path.join(out, "vendor/pi"), { recursive: true });
  await fs.writeFile(path.join(out, "vendor/pi/pi.mjs"), bundle.contents);
  const pi = JSON.parse(await fs.readFile(path.join(plugin, "node_modules/@earendil-works/pi-coding-agent/package.json"), "utf8"));
  // PI_PACKAGE_DIR: pi reads its own package.json at import time.
  await fs.writeFile(
    path.join(out, "vendor/pi/package.json"),
    `${JSON.stringify({ name: pi.name, version: pi.version, private: true, type: "module", description: "Vendored by tests/vendor-pi.mjs" }, null, 2)}\n`,
  );

  // Bundled packages → licenses/<scope>__<name>/ and the inventory.
  const packages = new Map();
  const [output] = Object.values(result.metafile.outputs);
  for (const input of Object.keys(output.inputs)) {
    const found = packageOf(input);
    if (found && !packages.has(found.name)) packages.set(found.name, found.directory);
  }
  const inventory = [];
  await fs.rm(path.join(out, "licenses"), { recursive: true, force: true });
  for (const name of [...packages.keys()].sort()) {
    const directory = packages.get(name);
    const meta = JSON.parse(await fs.readFile(path.join(directory, "package.json"), "utf8"));
    const upstream = UPSTREAM[name.startsWith("@earendil-works/") ? "pi" : name];
    const file = upstream ? "LICENSE" : await licenseFile(directory);
    if (!file) throw new Error(`${name}: no license file; add it to UPSTREAM`);
    const target = path.join(out, "licenses", name.replace("/", "__"));
    await fs.mkdir(target, { recursive: true });
    // LF, as git checks it out (.gitattributes: text=auto eol=lf): some ship CRLF.
    const text = await fs.readFile(upstream ? path.join(plugin, "tests/upstream", upstream) : path.join(directory, file), "utf8");
    await fs.writeFile(path.join(target, file), text.replace(/\r\n/g, "\n"));
    await fs.writeFile(
      path.join(target, "package.json"),
      `${JSON.stringify({ name, version: meta.version, license: meta.license ?? null }, null, 2)}\n`,
    );
    inventory.push({ name, version: meta.version, license: meta.license ?? null, licenseFiles: [file] });
  }
  await fs.writeFile(path.join(out, "dependency-inventory.json"), `${JSON.stringify(inventory, null, 2)}\n`);

  // The model catalog the settings page offers, read from the bundle itself.
  process.env.PI_OFFLINE = "1";
  process.env.PI_TELEMETRY = "0";
  process.env.PI_PACKAGE_DIR = path.join(out, "vendor/pi");
  const built = `${pathToFileURL(path.join(out, "vendor/pi/pi.mjs")).href}?${Date.now()}`;
  const sdk = await import(built); // boundary-reviewed: the bundle just built, inside this package or a scratch copy of it
  const runtime = await sdk.ModelRuntime.create({ credentials: new sdk.InMemoryCredentialStore(), modelsPath: null, refreshOnCreate: false });
  const providers = [];
  for (const provider of runtime.getProviders()) {
    const models = runtime.getModels(provider.id);
    if (!offered(provider, models)) continue;
    // Where the provider lists its models (「获取模型」): from the API most of its models use.
    const count = new Map();
    for (const m of models) count.set(m.api, (count.get(m.api) ?? 0) + 1);
    const api = [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0];
    const base = models.find((m) => m.api === api).baseUrl.replace(/\/+$/, "");
    const list = listing(api, base);
    providers.push({ id: provider.id, name: provider.name, models: models.map((m) => m.id).sort(), ...(list ? { list } : {}) });
  }
  providers.sort((a, b) => a.name.localeCompare(b.name, "en"));
  await fs.writeFile(path.join(out, "vendor/pi/models.json"), `${JSON.stringify({ providers }, null, 2)}\n`);
  return { size: bundle.contents.length, packages: inventory.length };
}

/** Every vendored file, relative, with its digest. */
async function digests(root) {
  const out = new Map();
  const walk = async (rel) => {
    const full = path.join(root, rel);
    if (!existsSync(full)) return;
    for (const item of await fs.readdir(full, { withFileTypes: true })) {
      const child = path.join(rel, item.name);
      if (item.isDirectory()) await walk(child);
      else out.set(child, createHash("sha256").update(await fs.readFile(path.join(root, child))).digest("hex"));
    }
  };
  for (const rel of ["vendor/pi/pi.mjs", "vendor/pi/package.json", "vendor/pi/models.json", "dependency-inventory.json"])
    if (existsSync(path.join(root, rel))) out.set(rel, createHash("sha256").update(await fs.readFile(path.join(root, rel))).digest("hex"));
  await walk("licenses");
  return out;
}

/** Differences between a fresh build and the committed files (empty when current). */
export async function differences() {
  const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-ai-vendor-"));
  try {
    await vendor(scratch);
    const fresh = await digests(scratch);
    const committed = await digests(plugin);
    // licenses/ also holds hand-written notices (pi logo): only bundled packages are compared.
    const names = new Set([...fresh.keys(), ...[...committed.keys()].filter((k) => !k.startsWith(path.join("licenses", "pi-logo")))]);
    return [...names].filter((k) => fresh.get(k) !== committed.get(k)).sort();
  } finally {
    await fs.rm(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes("--write")) {
    // Keep the hand-written logo notice across rewrites of licenses/.
    const notice = path.join(plugin, "licenses/pi-logo");
    const kept = existsSync(notice) ? await fs.mkdtemp(path.join(os.tmpdir(), "vestra-ai-notice-")) : null;
    if (kept) await fs.cp(notice, kept, { recursive: true });
    const { size, packages } = await vendor(plugin);
    if (kept) {
      await fs.cp(kept, notice, { recursive: true });
      await fs.rm(kept, { recursive: true, force: true });
    }
    console.log(`vendor/pi/pi.mjs: ${size} bytes, ${packages} bundled packages`);
  } else {
    const changed = await differences();
    console.log(changed.length ? `out of date:\n${changed.join("\n")}` : "vendored pi is current");
    process.exitCode = changed.length ? 1 : 0;
  }
}
