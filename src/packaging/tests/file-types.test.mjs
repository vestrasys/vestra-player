// node --test packaging/tests/ (npm run test:packaging)
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  AUDIO,
  MACOS_IMPORTED,
  VIDEO,
  WINDOWS_INCLUDE,
  windowsRegistry,
} from "../file-types.mjs";
import { packaging, source } from "../lib.mjs";
import { infoPlist, infoPlistEntries } from "../macos-plist.mjs";

const KINDS = [...VIDEO, ...AUDIO];

test("only kinds the built-in playlist plays, each once", async () => {
  const media = await readFile(
    join(source, "plugins/playlist/media.ts"),
    "utf8",
  );
  const set = (name) =>
    new Set(
      media
        .match(new RegExp(`export const ${name} = new Set\\(\\s*"([^"]+)"`))[1]
        .split(" "),
    );
  const [video, audio] = [set("VIDEO"), set("AUDIO")];
  for (const kind of VIDEO) assert.ok(video.has(kind), kind);
  for (const kind of AUDIO) assert.ok(audio.has(kind), kind);
  assert.equal(new Set(KINDS).size, KINDS.length);
});

test("Info.plist: Viewer at Alternate rank for movies and audio", () => {
  const entries = infoPlistEntries("1.2.3", "14.0");
  const types = entries.CFBundleDocumentTypes;
  assert.deepEqual(
    types.map((type) => type.CFBundleTypeName),
    ["Video", "Audio"],
  );
  for (const type of types) {
    assert.equal(type.CFBundleTypeRole, "Viewer");
    assert.equal(type.LSHandlerRank, "Alternate");
  }
  assert.ok(types[0].LSItemContentTypes.includes("public.movie"));
  assert.ok(types[1].LSItemContentTypes.includes("public.audio"));
  // Every imported type is a known kind and opens as video or audio.
  const listed = types.flatMap((type) => type.LSItemContentTypes);
  const imported = entries.UTImportedTypeDeclarations;
  assert.equal(imported.length, MACOS_IMPORTED.length);
  for (const declaration of imported) {
    assert.ok(listed.includes(declaration.UTTypeIdentifier));
    assert.ok(
      ["public.movie", "public.audio"].includes(
        declaration.UTTypeConformsTo[0],
      ),
    );
    for (const extension of declaration.UTTypeTagSpecification[
      "public.filename-extension"
    ])
      assert.ok(KINDS.includes(extension), extension);
  }
  // Not an editor, not the owner of any type.
  assert.equal(entries.UTExportedTypeDeclarations, undefined);
  assert.equal(entries.CFBundleExecutable, "vestra-host");
});

test(
  "Info.plist is a property list macOS reads back",
  { skip: process.platform !== "darwin" && "plutil is macOS only" },
  async () => {
    const directory = await mkdtemp(join(tmpdir(), "vestra-plist-"));
    try {
      const file = join(directory, "Info.plist");
      await writeFile(file, infoPlist("1.2.3", "14.0"));
      const read = spawnSync("plutil", ["-convert", "json", "-o", "-", file], {
        encoding: "utf8",
      });
      assert.equal(read.status, 0, read.stderr);
      assert.deepEqual(
        JSON.parse(read.stdout),
        JSON.parse(JSON.stringify(infoPlistEntries("1.2.3", "14.0"))),
      );
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  },
);

test("windows/file-types.iss is generated from the list", async () => {
  assert.equal(await readFile(WINDOWS_INCLUDE, "utf8"), windowsRegistry());
});

test("installer: Open with for every kind, the default-app offer behind the task", async () => {
  const lines = windowsRegistry()
    .split("\n")
    .filter((line) => line.startsWith("Root: "));
  const field = (line, name) =>
    line.match(new RegExp(`(?:^|; )${name}: ("(?:[^"]|"")*"|[^;]+)`))?.[1];
  const has = (subkey, name, value) =>
    lines.find(
      (line) =>
        field(line, "Subkey") === `"${subkey}"` &&
        field(line, "ValueName") === `"${name}"` &&
        (value === undefined || field(line, "ValueData") === `"${value}"`),
    );
  for (const kind of KINDS) {
    const progid = VIDEO.includes(kind)
      ? "VestraPlayer.Video"
      : "VestraPlayer.Audio";
    const openWith = has(
      `Software\\Classes\\.${kind}\\OpenWithProgids`,
      progid,
    );
    assert.ok(openWith, kind);
    assert.equal(field(openWith, "Flags"), "uninsdeletevalue");
    assert.equal(field(openWith, "Tasks"), undefined);
    assert.ok(
      has(
        "Software\\Classes\\Applications\\vestra-host.exe\\SupportedTypes",
        `.${kind}`,
      ),
      kind,
    );
    const default_ = has(
      "Software\\VestraPlayer\\Capabilities\\FileAssociations",
      `.${kind}`,
      progid,
    );
    assert.equal(field(default_, "Tasks"), "associate");
  }
  // Nothing the system's own types keep: only our ProgIDs, our application
  // key and values beside other apps' (OpenWithProgids, RegisteredApplications).
  for (const line of lines) {
    const subkey = field(line, "Subkey");
    if (/^"Software\\Classes\\\.\w+\\OpenWithProgids"$/.test(subkey)) continue;
    assert.match(
      subkey,
      /^"Software\\(Classes\\(VestraPlayer\.(Video|Audio)|Applications\\vestra-host\.exe)|VestraPlayer|RegisteredApplications)(\\|")/,
    );
  }
  const registered = has(
    "Software\\RegisteredApplications",
    "Vestra Player",
    "Software\\VestraPlayer\\Capabilities",
  );
  assert.equal(field(registered, "Tasks"), "associate");
  assert.equal(field(registered, "Flags"), "uninsdeletevalue");
  // The uninstaller deletes the keys it created.
  for (const subkey of [
    "Software\\Classes\\VestraPlayer.Video",
    "Software\\Classes\\VestraPlayer.Audio",
    "Software\\Classes\\Applications\\vestra-host.exe",
    "Software\\VestraPlayer\\Capabilities",
  ])
    assert.ok(
      lines.some(
        (line) =>
          field(line, "Subkey") === `"${subkey}"` &&
          field(line, "Flags") === "uninsdeletekey",
      ),
      subkey,
    );
  const open = has(
    "Software\\Classes\\VestraPlayer.Video\\shell\\open\\command",
    "",
  );
  assert.equal(field(open, "ValueData"), '"""{app}\\vestra-host.exe"" ""%1"""');

  const iss = await readFile(join(packaging, "windows/vestra.iss"), "utf8");
  assert.match(
    iss,
    /^Name: "associate"; Description: "[^"]+"; GroupDescription: "[^"]+"; Flags: unchecked$/m,
  );
  assert.match(iss, /^#include "file-types\.iss"$/m);
  assert.match(iss, /^ChangesAssociations=yes$/m);
  assert.match(iss, /not WizardIsTaskSelected\('associate'\)/);
});
