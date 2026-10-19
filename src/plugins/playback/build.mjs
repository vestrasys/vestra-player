import { spawnSync } from "node:child_process";
import { access, cp, mkdir, readFile, rm, writeFile, rename } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
const directory = path.dirname(fileURLToPath(import.meta.url));
const version = process.argv[2] ?? "0.1.1";
if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version) || version.split(".").some(part => BigInt(part) > 4294967295n)) throw new Error("Version must contain three u32 decimal components without leading zeros");
async function choose(explicit, candidates) {
  if (explicit) { await access(explicit); return explicit; }
  for (const candidate of candidates) { try { await access(candidate); return candidate; } catch {} }
  throw new Error(`Missing build dependency; supply explicit VESTRA_SDK or VESTRA_TSC: ${candidates.join(", ")}`);
}
const sdk = await choose(process.env.VESTRA_SDK, [path.resolve(directory, "../../sdk"), "/tmp/vestra-current-runtime-cache/sdk"]);
const compiler = await choose(process.env.VESTRA_TSC, [path.resolve(directory, "../../node_modules/typescript/lib/_tsc.js"), "/tmp/vestra-rebuild-toolchain-20261018/node_modules/typescript/lib/_tsc.js"]);
const typeRoots = process.env.VESTRA_TYPE_ROOTS ?? path.resolve(path.dirname(compiler), "../../@types");
const input = path.join(directory, ".build-input"), output = path.join(directory, ".build-output"), next = path.join(directory, ".dist-next");
await rm(input, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
await mkdir(path.join(input, "plugins/playback"), { recursive: true });
await cp(sdk, path.join(input, "sdk"), { recursive: true });
for (const name of ["main.ts", "interaction.ts", "tests"]) await cp(path.join(directory, name), path.join(input, "plugins/playback", name), { recursive: true });
await writeFile(path.join(input, "package.json"), '{"type":"module"}\n');
const result = spawnSync(process.execPath, [compiler, "-p", path.join(directory, "tsconfig.build.json"), "--typeRoots", typeRoots], { stdio: "inherit" });
if (result.status !== 0) process.exit(result.status ?? 1);
await rm(next, { recursive: true, force: true });
await mkdir(next, { recursive: true });
for (const name of ["main.js", "interaction.js"]) await cp(path.join(output, "plugins/playback", name), path.join(next, name));
await cp(path.join(directory, "ui"), path.join(next, "ui"), { recursive: true });
await writeFile(path.join(next, "package.json"), '{"type":"module"}\n');
const manifest = JSON.parse(await readFile(path.join(directory, "manifest.json"), "utf8"));
await writeFile(path.join(next, "manifest.json"), JSON.stringify({ ...manifest, version, entry: "main.js" }, null, 2) + "\n");
await rm(path.join(directory, "dist"), { recursive: true, force: true });
await rename(next, path.join(directory, "dist"));
await rm(input, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
console.log(`Built ${manifest.id}@${version}; types and package tests checked; dist has no SDK dependency`);
