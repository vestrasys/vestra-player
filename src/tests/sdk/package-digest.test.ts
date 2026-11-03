/**
 * `builtin.json` schema 3 records each built-in's package digest, computed
 * by `scripts/stage-plugins.mjs` and checked by the host
 * (`package_stage::read_tree`) before it runs the plugin in the shared runner
 * (doc/architecture/README.md §2). Both implementations
 * hash `tests/fixtures/package-digest.json`; the host's
 * `digest_matches_the_shared_fixture` test reads the same file. A drift
 * fails both sides.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
// @ts-expect-error: a JavaScript build script without declarations.
import { packageDigest } from "../../scripts/stage-plugins.mjs";

const fixture = JSON.parse(
  readFileSync(
    join(import.meta.dirname, "..", "fixtures", "package-digest.json"),
    "utf8",
  ),
) as { files: Record<string, string>; digest: string };

const tree = (files: Record<string, string>) =>
  new Map(
    Object.entries(files).map(([name, text]) => [
      name,
      Buffer.from(text, "utf8"),
    ]),
  );

test("the staging digest is the host's digest of the shared fixture", () => {
  assert.match(fixture.digest, /^[0-9a-f]{64}$/);
  assert.equal(packageDigest(tree(fixture.files)), fixture.digest);
});

test("the digest binds names, bytes and order-independent listing", () => {
  const reversed = Object.fromEntries(Object.entries(fixture.files).reverse());
  assert.equal(packageDigest(tree(reversed)), fixture.digest);
  const changed = { ...fixture.files, "main.js": "export {};\n" };
  assert.notEqual(packageDigest(tree(changed)), fixture.digest);
  const { "ui.json": moved, ...rest } = fixture.files;
  assert.notEqual(
    packageDigest(tree({ ...rest, "ui2.json": moved })),
    fixture.digest,
  );
  // Name and content boundaries are length-prefixed: moving a byte from a
  // name into the content is a different package.
  assert.notEqual(
    packageDigest(tree({ ab: "c" })),
    packageDigest(tree({ a: "bc" })),
  );
});
