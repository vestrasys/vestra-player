// Plugin-author documentation lives on the SDK's site only; see
// scripts/check-docs.mjs for the rules (doc/plugins holds only README.md, no
// copies of the site's capability, limits or platform tables in doc/).
import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { runNode } from "./children.ts";

test("doc/ does not copy the SDK site's author documentation", () => {
  const result = runNode([
    fileURLToPath(new URL("../../scripts/check-docs.mjs", import.meta.url)),
  ]);
  assert.equal(result.status, 0, result.stderr);
});
