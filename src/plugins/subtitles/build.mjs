import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, cp, rm, mkdtemp } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const directory = dirname(fileURLToPath(import.meta.url));
const toolchain = process.env.VESTRA_TYPESCRIPT_TOOLCHAIN ?? "/tmp/vestra-rebuild-toolchain-20261018/node_modules";
const sdk = process.env.VESTRA_SDK_DIRECTORY ?? "/tmp/vestra-current-runtime-cache/sdk";
const temp = await mkdtemp(resolve(directory, ".typecheck-"));
const source = await readFile(resolve(directory, "main.ts"), "utf8");
try {
  await writeFile(resolve(temp, "main.ts"), source.replaceAll('"../../sdk/', `"${sdk}/`));
  const result = spawnSync(process.execPath, [resolve(toolchain, "typescript/lib/_tsc.js"), "--target", "ES2022", "--module", "ESNext", "--moduleResolution", "Bundler", "--allowImportingTsExtensions", "--noEmit", "--strict", "--skipLibCheck", "--types", "node", "--typeRoots", resolve(toolchain, "@types"), resolve(temp, "main.ts")], { stdio: "inherit" });
  if (result.status !== 0) throw new Error(`Strict typecheck failed: ${result.status}`);
} finally { await rm(temp, { recursive: true, force: true }); }
const ts = createRequire(import.meta.url)(resolve(toolchain, "typescript/lib/typescript.js"));
const js = ts.transpileModule(source, { fileName: "main.ts", compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
if (/from\s*["'][^"']*(?:sdk|\.ts["'])/.test(js)) throw new Error("Unexpected runtime SDK/TS import");
const manifest = JSON.parse(await readFile(resolve(directory, "manifest.json"), "utf8"));
if (!/^\d+\.\d+\.\d+$/.test(manifest.version)) throw new Error("Strict x.y.z version required");
const dist = resolve(directory, "dist");
await rm(dist, { force: true, recursive: true }); await mkdir(dist, { recursive: true });
await writeFile(resolve(dist, "main.js"), js);
await writeFile(resolve(dist, "manifest.json"), JSON.stringify({ ...manifest, entry: "main.js" }, null, 2) + "\n");
await cp(resolve(directory, "ui"), resolve(dist, "ui"), { recursive: true });
console.error(`strict typecheck exit 0; built ${manifest.version}, TS ${ts.version}: ${dist}`);
