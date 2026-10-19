/** Self-contained current-source macOS evidence bundle; does not build or install. */
import { mkdtemp, mkdir, cp, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { resolve, join } from "node:path";
import { tmpdir } from "node:os";
const root = resolve(import.meta.dirname, "../..");
const work = await mkdtemp(join(tmpdir(), "vestra-native-shared-"));
const bundle = join(work, "Vestra Shared UI.app");
const contents = join(bundle, "Contents"),
  resources = join(contents, "Resources");
await mkdir(join(contents, "MacOS"), { recursive: true });
await mkdir(join(resources, "examples"), { recursive: true });
await cp(
  join(root, "src/host/target/debug/vestra-host"),
  join(contents, "MacOS/host"),
);
for (const name of ["sdk", "runtime"])
  await cp(join(root, "src", name), join(resources, name), { recursive: true });
await cp(
  join(root, "src/examples/host-ui-demo"),
  join(resources, "examples/host-ui-demo"),
  { recursive: true },
);
await cp(join(root, "src/host/ui"), join(resources, "ui"), { recursive: true });
await cp(
  join(root, "src/examples/mount-demo"),
  join(resources, "examples/mount-demo"),
  { recursive: true },
);
const bad = join(resources, "examples/bad-mount");
await mkdir(bad, { recursive: true });
await writeFile(
  join(bad, "manifest.json"),
  JSON.stringify({
    id: "example.bad-mount",
    apiVersion: "vestra.experimental/0.1",
    entry: "main.js",
    capabilities: ["ui.mount"],
  }),
);
await writeFile(
  join(bad, "main.js"),
  `export async function activate(host) { await host.ui.mount({slot:"sidebar",file:"bad.slint",component:"Broken",global:"Bridge",properties:{}}); }`,
);
await writeFile(
  join(bad, "bad.slint"),
  `export global Bridge { } export component Broken inherits Rectangle { invalid syntax here`,
);
const id = "local.vestra.rebuild.shared-ui-v12";
await writeFile(
  join(contents, "Info.plist"),
  `<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>host</string><key>CFBundleIdentifier</key><string>${id}</string><key>CFBundleName</key><string>Vestra Shared UI</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>`,
);
const args = [
  "-n",
  bundle,
  "--env",
  "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
  "--env",
  `VESTRA_DEMO_MEDIA=${process.argv[2] ?? "/tmp/vestra-bootstrap-long.mp4"}`,
  "--env",
  "VESTRA_TRACE=1",
  "--env",
  `VESTRA_SIDEBAR_PACKAGE=${join(resources, "examples/mount-demo/sidebar")}`,
  "--env",
  `VESTRA_BAD_MOUNT_PLUGIN=${bad}`,
  "--stdout",
  join(work, "stdout.log"),
  "--stderr",
  join(work, "stderr.log"),
  "--args",
  "--plugin",
  join(resources, "examples/host-ui-demo"),
  "--plugin",
  join(resources, "examples/mount-demo/bottom"),
  "--plugin",
  join(resources, "examples/mount-demo/sidebar"),
  "--seconds",
  "600",
];
await writeFile(
  join(work, "launch.json"),
  JSON.stringify({ work, bundle, id, command: ["open", ...args] }, null, 2) +
    "\n",
);
const result = spawnSync("open", args, { encoding: "utf8" });
console.log(
  JSON.stringify({
    work,
    bundle,
    id,
    openExit: result.status,
    error: result.stderr,
  }),
);
process.exitCode = result.status ?? 1;
