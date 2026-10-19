import { cp, mkdir, readFile, rm, writeFile, rename } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const directory = dirname(fileURLToPath(import.meta.url));
const version = process.argv[2] ?? "0.1.0";
if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)
  || version.split(".").some(part => Number(part) > 0xffff_ffff))
  throw new Error("Expected MAJOR.MINOR.PATCH with u32 components and no leading zeroes");

const compiler = resolve(process.env.VESTRA_TYPESCRIPT ?? join(directory, "../../node_modules/typescript/lib/_tsc.js"));
const sdk = resolve(process.env.VESTRA_SDK ?? join(directory, "../../sdk"));
const typeRoots = resolve(process.env.VESTRA_TYPE_ROOTS ?? join(directory, "../../node_modules/@types"));
const input = join(directory, ".build-input");
const output = join(directory, ".build");
await rm(input, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
await mkdir(join(input, "plugins/playlist"), { recursive: true });
// Type imports resolve against ordinary copies. Distribution contains no SDK JS.
await cp(sdk, join(input, "sdk"), { recursive: true });
for (const file of ["main.ts", "queue.ts", "projection.ts"])
  await cp(join(directory, file), join(input, "plugins/playlist", file));
await cp(join(directory, "tests"), join(input, "plugins/playlist/tests"), { recursive: true });
await writeFile(join(input, "package.json"), '{"type":"module"}\n');
const configuration = JSON.parse(await readFile(join(directory, "tsconfig.build.json"), "utf8"));
configuration.compilerOptions = { ...configuration.compilerOptions, rootDir: ".", outDir: "../.build", typeRoots: [typeRoots] };
delete configuration.files;
configuration.include = ["plugins/playlist/**/*.ts"];
await writeFile(join(input, "tsconfig.json"), JSON.stringify(configuration, null, 2));
execFileSync(process.execPath, [compiler, "-p", join(input, "tsconfig.json")], { stdio: "inherit" });
execFileSync(process.execPath, ["--test", ...["consumer", "projection"].map(name => join(output, "plugins/playlist/tests", `${name}.test.js`))], { stdio: "inherit" });
const nextDist = join(directory, ".dist-next");
await rm(nextDist, { recursive: true, force: true });
await mkdir(nextDist, { recursive: true });
for (const file of ["main.js", "queue.js", "projection.js"])
  await cp(join(output, "plugins/playlist", file), join(nextDist, file));
await cp(join(directory, "ui"), join(nextDist, "ui"), { recursive: true });
const manifest = JSON.parse(await readFile(join(directory, "manifest.json"), "utf8"));
await writeFile(join(nextDist, "manifest.json"), JSON.stringify({ ...manifest, version, entry: "main.js" }, null, 2) + "\n");
await writeFile(join(nextDist, "package.json"), '{"type":"module"}\n');
const dist = join(directory, "dist");
await rm(dist, { recursive: true, force: true });
await rename(nextDist, dist);
await rm(input, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
console.log(dist);
