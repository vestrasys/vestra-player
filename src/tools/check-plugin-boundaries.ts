// 实验版本：静态依赖检查不是权限沙箱，与最终插件 API 可能相差很大。
import { readdir, readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const sourceRoot = fileURLToPath(new URL("../", import.meta.url));
const pluginsRoot = path.join(sourceRoot, "plugins");
const sdkRoot = path.join(sourceRoot, "sdk");
const issues: string[] = [];

function within(file: string, root: string): boolean {
  const relative = path.relative(root, file);
  return (
    relative === "" ||
    (!relative.startsWith(`..${path.sep}`) &&
      relative !== ".." &&
      !path.isAbsolute(relative))
  );
}

async function filesIn(directory: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (["node_modules", "target", ".cache", "evidence"].includes(entry.name))
      continue;
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesIn(file)));
    else if (entry.isFile() && /\.(?:[cm]?[jt]sx?|slint)$/.test(entry.name))
      files.push(file);
    else if (entry.isSymbolicLink())
      issues.push(`${file}: 插件源码不允许通过 symlink 绕过依赖边界`);
  }
  return files;
}

async function checkSpecifier(
  file: string,
  specifier: string,
  pluginRoot: string,
): Promise<void> {
  if (
    !specifier.startsWith(".") &&
    !path.isAbsolute(specifier) &&
    !specifier.startsWith("file:")
  )
    return;
  const resolved = specifier.startsWith("file:")
    ? fileURLToPath(specifier)
    : path.resolve(path.dirname(file), specifier);
  // 没有扩展名的模块仍需交给 TS 编译器检查；这里保留相同的路径边界判断。
  const canonical = await realpath(resolved).catch(() => resolved);
  if (!within(canonical, pluginRoot) && !within(canonical, sdkRoot)) {
    issues.push(
      `${path.relative(sourceRoot, file)}: 非公开/跨插件依赖 ${specifier}`,
    );
  }
}

async function checkFile(file: string, pluginRoot: string): Promise<void> {
  const contents = await readFile(file, "utf8");
  if (file.endsWith(".slint")) {
    for (const match of contents.matchAll(/\bfrom\s+"([^"]+)"/g)) {
      if (match[1] !== "std-widgets.slint")
        await checkSpecifier(file, match[1], pluginRoot);
    }
    return;
  }
  const source = ts.createSourceFile(
    file,
    contents,
    ts.ScriptTarget.Latest,
    true,
  );
  const specifiers: string[] = [];
  function walk(node: ts.Node): void {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteral(node.moduleSpecifier)
    ) {
      specifiers.push(node.moduleSpecifier.text);
    }
    if (
      ts.isCallExpression(node) &&
      (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) &&
          node.expression.text === "require"))
    ) {
      const argument = node.arguments[0];
      if (argument && ts.isStringLiteralLike(argument))
        specifiers.push(argument.text);
      else
        issues.push(
          `${path.relative(sourceRoot, file)}: 动态模块路径无法验证；须使用可审查的字面量`,
        );
    }
    if (node.kind === ts.SyntaxKind.AnyKeyword)
      issues.push(`${path.relative(sourceRoot, file)}: 显式 any 绕过公共契约`);
    ts.forEachChild(node, walk);
  }
  walk(source);
  for (const specifier of specifiers)
    await checkSpecifier(file, specifier, pluginRoot);
}

let pluginCount = 0;
for (const entry of await readdir(pluginsRoot, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  const pluginRoot = path.join(pluginsRoot, entry.name);
  for (const file of await filesIn(pluginRoot))
    await checkFile(file, pluginRoot);
  pluginCount += 1;
}
if (issues.length) {
  process.stderr.write(`${issues.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(
    `插件源码静态依赖检查通过：${pluginCount} 个目录。运行时文件访问/通信仍需独立验收审查。\n`,
  );
}
