// Moves the recorded plugin props (src/scripts/ui-record.mjs, VESTRA_UI_RECORD)
// between runners: the ts job records them on Linux, the Windows build-test
// checks them (`recorded_plugin_props_match_their_slint`). Each line names its
// plugin by absolute path, so the recording job makes the paths relative to its
// checkout and the checking job makes them absolute in its own.
//
//   node ui-record-paths.mjs relative <dir> <checkout>
//   node ui-record-paths.mjs absolute <dir> <checkout>
//
// Prints the number of records; exits 1 on a path outside the checkout.
// Ends by process.exitCode, never process.exit(), which can deadlock Node 26
// (#174; src/packaging/lib.mjs, reportErrors).
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

function main([mode, dir, checkout]) {
  if (!["relative", "absolute"].includes(mode) || !dir || !checkout) {
    console.error(
      "usage: ui-record-paths.mjs relative|absolute <dir> <checkout>",
    );
    return 2;
  }
  const root = resolve(checkout);
  let count = 0;
  for (const name of readdirSync(dir).filter((n) => n.endsWith(".jsonl"))) {
    const file = join(dir, name);
    const lines = readFileSync(file, "utf8").split("\n").filter(Boolean);
    const out = [];
    for (const line of lines) {
      const record = JSON.parse(line);
      if (mode === "relative") {
        const path = relative(root, record.plugin);
        if (isAbsolute(path) || path.startsWith("..")) {
          console.error(`${file}: ${record.plugin} is outside ${root}`);
          return 1;
        }
        record.plugin = path.split(sep).join("/");
      } else {
        record.plugin = join(root, ...record.plugin.split("/"));
      }
      count++;
      out.push(JSON.stringify(record));
    }
    writeFileSync(file, out.map((l) => `${l}\n`).join(""));
  }
  console.log(`${count} recorded values in ${dir} (${mode} paths)`);
  return 0;
}

process.exitCode = main(process.argv.slice(2));
