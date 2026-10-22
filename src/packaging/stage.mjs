/**
 * What both packages share: resources (ui, runtime, plugins) and licenses/.
 * bundle-macos.mjs and bundle-windows.mjs call stageResources and
 * stageLicenses with their own directories (proposal §3.1, §4.2).
 */
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { basename, dirname, join } from "node:path";
import { loadThirdParty } from "./check-licenses.mjs";
import { copy, exists, hostVersion, readJson, run, source } from "./lib.mjs";
import { stagePlugins } from "../scripts/stage-plugins.mjs";
import { stageRuntime } from "../scripts/stage-runtime.mjs";

const sdkPackage = dirname(
  createRequire(join(source, "package.json")).resolve(
    "@vestra-player/sdk/package.json",
  ),
);

/** <resources>/ui, runtime, plugins. */
export async function stageResources(resources) {
  await copy(join(source, "host/ui"), join(resources, "ui"));
  await stageRuntime(join(resources, "runtime"));
  // The SDK's Apache-2.0 LICENSE travels with its code (stage-runtime adds
  // it once the host branch lands; until then it is added here).
  const sdkLicense = join(
    resources,
    "runtime/node_modules/@vestra-player/sdk/LICENSE",
  );
  if (!exists(sdkLicense)) await copy(join(sdkPackage, "LICENSE"), sdkLicense);
  const plugins = await stagePlugins(join(resources, "plugins"));
  console.log(
    `staged ui, runtime and ${plugins.length} built-in plugins in ${resources}`,
  );
}

/** Rust crate licenses (cargo-about) for one target, as HTML. */
export function rustLicenses(target, out) {
  run("cargo", [
    "about",
    "generate",
    "--locked",
    "--fail",
    "--manifest-path",
    join(source, "host/Cargo.toml"),
    "-c",
    join(source, "packaging/about.toml"),
    "--target",
    target,
    "-o",
    out,
    join(source, "packaging/about.hbs"),
  ]);
}

/**
 * <licenses>/: LICENSE, rust-crates.html, third-party/… and index.html.
 * `nodeLicense` is the LICENSE beside the fetched node, `deps` the deps
 * directory (licenses/ and components.json).
 */
export async function stageLicenses(licenses, { rustHtml, nodeLicense, deps }) {
  await mkdir(licenses, { recursive: true });
  const config = await loadThirdParty();
  for (const component of config.component)
    for (const [index, from] of (component.license_from ?? []).entries())
      await copy(
        join(source, from),
        join(licenses, component.license_files[index]),
      );
  await copy(rustHtml, join(licenses, "rust-crates.html"));
  await copy(nodeLicense, join(licenses, "third-party/node/LICENSE"));
  await copy(
    join(sdkPackage, "LICENSE"),
    join(licenses, "third-party/vestra-player-sdk/LICENSE"),
  );
  await copy(
    join(deps, "licenses"),
    join(licenses, "third-party/deps/licenses"),
  );
  await copy(
    join(deps, "components.json"),
    join(licenses, "third-party/deps/components.json"),
  );
  await writeFile(
    join(licenses, "index.html"),
    await indexHtml(config, join(deps, "components.json")),
  );
}

const escape = (text) =>
  String(text ?? "").replace(
    /[&<>"]/g,
    (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[char],
  );

async function indexHtml(config, componentsPath) {
  const version = await hostVersion();
  const node = await readJson(join(source, "packaging/node.json"));
  const deps = await readJson(componentsPath);
  const link = (file) =>
    `<a href="${escape(encodeURI(file))}">${escape(basename(file))}</a>`;
  const rows = [];
  for (const component of config.component) {
    if (component.deps) continue;
    const files = (component.license_files ?? []).map(link).join(", ");
    const inside = component.vxp
      ? ` (inside the ${escape(component.vxp)} plugin package: ${component.vxp_license_files.map(escape).join(", ")})`
      : "";
    const ver = component.version === "node.json" ? node.version : "";
    rows.push(
      `<tr><td>${escape(component.name)} ${escape(ver)}</td><td>${escape(component.license)}</td><td>${files}${inside}${component.note ? `<br><small>${escape(component.note)}</small>` : ""}${component.source ? `<br><small>${link2(component.source)}</small>` : ""}</td></tr>`,
    );
  }
  for (const component of deps.components)
    rows.push(
      `<tr><td>${escape(component.name)} ${escape(component.version)}</td><td>${escape(component.license)}</td><td>${component.license_files.map((file) => link(`third-party/deps/${file}`)).join(", ")}${component.note ? `<br><small>${escape(component.note)}</small>` : ""}<br><small>${link2(component.source?.url)}</small></td></tr>`,
    );
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Vestra Player licenses</title>
<style>
  body { font: 14px/1.5 -apple-system, "Segoe UI", sans-serif; max-width: 64em; margin: 2em auto; padding: 0 1em; color: #222; background: #fff; }
  @media (prefers-color-scheme: dark) { body { color: #ddd; background: #1b1b1f; } a { color: #8ab4ff; } }
  table { border-collapse: collapse; width: 100%; }
  td { border-top: 1px solid #8884; padding: 0.4em 0.6em 0.4em 0; vertical-align: top; }
</style>
</head>
<body>
<h1>Vestra Player ${escape(version)}</h1>
<p>Vestra Player is free software under the GNU General Public License, version 3 or later: ${link("LICENSE")}.
It includes the third-party software below, each under its own license.</p>
<p>Rust crates linked into the player: ${link("rust-crates.html")}.</p>
<table>
<tr><th align="left">Component</th><th align="left">License</th><th align="left">License text, source</th></tr>
${rows.join("\n")}
</table>
</body>
</html>
`;
}

const link2 = (url) =>
  url ? `<a href="${escape(url)}">${escape(url)}</a>` : "";

/** Info for file names: VestraPlayer-<version>-<platform>. */
export async function artifactBase(platform) {
  return `VestraPlayer-${await hostVersion()}-${platform}`;
}
