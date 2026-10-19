/** Real product-package integration; no private product state or synthetic UI events. */
import { mkdtemp, mkdir, cp, writeFile, readFile } from "node:fs/promises";
import { resolve, basename } from "node:path";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
const directories = process.argv.slice(2);
const holdSeconds = Math.min(
  180,
  Math.max(0, Number(process.env.VESTRA_SLICE_HOLD_SECONDS ?? 0)),
);

if (!directories.length)
  throw Error("Pass one or more frozen product package directories");
const root = await mkdtemp("/tmp/vestra-product-slices-");
const bundleId =
  "local.vestra.product-slices-" + basename(root).replace(/[^a-zA-Z0-9-]/g, "");
const contents = resolve(root, "Vestra Product Slices.app/Contents"),
  resources = resolve(contents, "Resources");
await mkdir(resolve(contents, "MacOS"), { recursive: true });
await mkdir(resources, { recursive: true });
const resourceCache = process.env.VESTRA_SLICE_RESOURCE_CACHE;

const binary = resolve(contents, "MacOS/host");
await cp(
  resourceCache
    ? resolve(resourceCache, "host")
    : resolve("src/host/target/debug/vestra-host"),
  binary,
);
for (const name of ["sdk", "runtime"])
  await cp(
    resourceCache ? resolve(resourceCache, name) : resolve("src", name),
    resolve(resources, name),
    { recursive: true },
  );
await cp(
  resourceCache ? resolve(resourceCache, "ui") : resolve("src/host/ui"),
  resolve(resources, "ui"),
  { recursive: true },
);
await writeFile(
  resolve(contents, "Info.plist"),
  '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>host</string><key>CFBundleIdentifier</key><string>' +
    bundleId +
    "</string><key>CFBundleName</key><string>Vestra Product Slices " +
    basename(root) +
    "</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>",
);
const packages = [];
for (const [i, directory] of directories.entries()) {
  const path = resolve(root, "packages", String(i));
  await mkdir(resolve(root, "packages"), { recursive: true });
  await cp(resolve(directory), path, {
    recursive: true,
    filter: (p) =>
      !p
        .slice(resolve(directory).length)
        .split("/")
        .some((x) => ["node_modules", ".build", "dist", "tests"].includes(x)) &&
      !p.endsWith(".test.ts"),
  });
  const manifest = JSON.parse(
    await readFile(resolve(path, "manifest.json"), "utf8"),
  );
  packages.push({ directory: path, source: resolve(directory), manifest });
}
await writeFile(
  resolve(root, "packages.json"),
  JSON.stringify(packages, null, 2),
);
async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path");
  const root = process.env.VESTRA_SLICE_ROOT;
  const packages = JSON.parse(
    await fs.readFile(path.join(root, "packages.json"), "utf8"),
  );
  const observations = [];
  let passed = true;
  const wait = () => new Promise((r) => setTimeout(r, 30));
  for (const [index, pkg] of packages.entries()) {
    const observation = {
      id: pkg.manifest.id,
      source: pkg.source,
      mode: pkg.manifest.version ? "productionManaged" : "debugLoad",
      trace: [],
    };
    try {
      let owner;
      if (pkg.manifest.version) {
        observation.validation = await host.packages.validate(pkg.directory);
        let op = await host.packages.install(
          observation.validation.token,
          0,
          "product-slice-" + index,
        );
        const deadline = Date.now() + 30000;
        while (
          ["preparing", "activating", "restoring"].includes(op.status) &&
          Date.now() < deadline
        ) {
          observation.trace.push(op);
          await wait();
          op = await host.packages.operation(op.operationId);
        }
        observation.operation = op;
        if (op.status !== "committed")
          throw Error(
            "Product installation did not commit: " + JSON.stringify(op),
          );
        owner = op.owner;
      } else {
        const loaded = await host.plugins.load(pkg.directory, 30000);
        observation.accepted = loaded;
        owner = loaded.owner;
      }
      const deadline = Date.now() + 30000;
      let state;
      do {
        state = await host.plugins.activation(owner);
        observation.trace.push(state);
        if (state.activation.status !== "pending") break;
        await wait();
      } while (Date.now() < deadline);
      observation.activation = state;
      if (!state.running || state.activation.status !== "ready")
        throw Error("Product activation not ready: " + JSON.stringify(state));
      observation.passed = true;
    } catch (error) {
      passed = false;
      observation.passed = false;
      observation.error = { code: error.code ?? null, message: String(error) };
    }
    observations.push(observation);
  }
  const result = {
    passed,
    observations,
    commands: await host.commands.snapshot(),
    window: await host.window.state(),
    source: await host.source.state(),
    limits: [
      "Actual product JS/TS activation and Slint preparation only. No visible clicking or product feature acceptance.",
      "Production packages use the real managed API in an independent temporary store.",
    ],
  };
  await fs.writeFile(
    path.join(root, "result.json"),
    JSON.stringify(result, null, 2),
  );
}
const manager = resolve(root, "manager");
await mkdir(manager);
await writeFile(
  resolve(manager, "manifest.json"),
  JSON.stringify({
    id: "integration.product-slices",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.mjs",
    capabilities: ["plugins.manage", "commands", "ui.window", "playback.read"],
  }),
);
await writeFile(resolve(manager, "main.mjs"), "export " + activate.toString());
const args = [
  "--silent",
  "--seconds",
  String(Math.max(65, holdSeconds + 30)),
  "--plugin",
  manager,
];
await writeFile(
  resolve(root, "command.json"),
  JSON.stringify(
    {
      binary,
      resourceCache: resourceCache ?? null,
      binarySha256: createHash("sha256")
        .update(await readFile(binary))
        .digest("hex"),
      args,
      env: {
        VESTRA_SLICE_ROOT: root,
        VESTRA_PACKAGE_STORE: resolve(root, "store"),
      },
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify({
    root,
    packages: packages.map((x) => ({
      id: x.manifest.id,
      source: x.source,
      version: x.manifest.version ?? null,
    })),
  }),
);
const child = spawn(binary, args, {
  env: {
    ...process.env,
    VESTRA_SLICE_ROOT: root,
    VESTRA_PACKAGE_STORE: resolve(root, "store"),
  },
  stdio: ["ignore", "pipe", "pipe"],
});
let stdout = "",
  stderr = "";
child.stdout.on("data", (b) => (stdout += b));
child.stderr.on("data", (b) => (stderr += b));
const closed = new Promise((r) =>
  child.once("close", (code, signal) => r({ code, signal })),
);
child.once("error", (e) => (stderr += String(e)));
let result;
for (let i = 0; i < 1400; i++) {
  try {
    result = JSON.parse(await readFile(resolve(root, "result.json"), "utf8"));
    break;
  } catch {}
  await new Promise((r) => setTimeout(r, 50));
}
if (result) {
  console.log(
    JSON.stringify({
      root,
      bundleId,
      protocolPassed: result.passed,
      hostPid: child.pid,
      bundle: resolve(root, "Vestra Product Slices.app"),
      holdSeconds,
    }),
  );
}
if (result?.passed && holdSeconds > 0) {
  await Promise.race([
    closed,
    new Promise((r) => setTimeout(r, holdSeconds * 1000)),
  ]);
}
child.kill("SIGTERM");
const exit = await closed;
await writeFile(resolve(root, "host.stdout.log"), stdout);
await writeFile(resolve(root, "host.stderr.log"), stderr);
await writeFile(resolve(root, "host-exit.json"), JSON.stringify(exit));
console.log(
  JSON.stringify({
    root,
    passed: result?.passed ?? false,
    observations: result?.observations?.map((o) => ({
      id: o.id,
      passed: o.passed,
      error: o.error,
    })),
    hostExit: exit,
  }),
);
process.exitCode = result?.passed ? 0 : 1;
