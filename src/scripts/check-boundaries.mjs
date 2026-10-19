import { readdir, readFile } from "node:fs/promises";
import { resolve, dirname, extname, relative, sep } from "node:path";
import ts from "typescript";
const source = resolve(import.meta.dirname, "..");
const sdk = resolve(source, "sdk");
async function files(directory) {
  const items = await readdir(directory, { withFileTypes: true });
  return (
    await Promise.all(
      items.map(async (item) => {
        const path = resolve(directory, item.name);
        if (item.isDirectory()) return files(path);
        return [".ts", ".js", ".mjs"].includes(extname(path)) ? [path] : [];
      }),
    )
  ).flat();
}
function inside(path, root) {
  return path === root || path.startsWith(root + sep);
}
for (const collection of ["examples", "plugins"]) {
  const root = resolve(source, collection);
  let packages;
  try {
    packages = await readdir(root, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") continue;
    throw error;
  }
  for (const item of packages.filter((item) => item.isDirectory())) {
    const own = resolve(root, item.name);
    for (const path of await files(own)) {
      const parsed = ts.createSourceFile(
        path,
        await readFile(path, "utf8"),
        ts.ScriptTarget.Latest,
        true,
      );
      function check(specifier) {
        if (!ts.isStringLiteralLike(specifier))
          throw new Error(`${path}: nonliteral dynamic imports require review`);
        const target = specifier.text;
        if (target.startsWith(".") || target.startsWith("/")) {
          const resolved = resolve(dirname(path), target);
          if (!inside(resolved, own) && !inside(resolved, sdk))
            throw new Error(
              `${relative(source, path)} imports outside its package/SDK: ${target}`,
            );
        }
      }
      function visit(node) {
        if (
          (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
          node.moduleSpecifier
        )
          check(node.moduleSpecifier);
        if (
          ts.isCallExpression(node) &&
          node.expression.kind === ts.SyntaxKind.ImportKeyword
        )
          check(node.arguments[0]);
        ts.forEachChild(node, visit);
      }
      visit(parsed);
    }
  }
}
console.log(
  "Plugin import boundaries pass (static imports; trusted runtime is not a sandbox).",
);
