import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
const root = path.dirname(new URL(import.meta.url).pathname);
const compiler = '/tmp/vestra-rebuild-toolchain-20261018/node_modules/typescript/lib/typescript.js';
const ts = createRequire(import.meta.url)(compiler);
const check = path.join(root, '.typecheck/src/plugins/nes');
await fs.mkdir(check, { recursive: true });
await fs.mkdir(path.join(root, '.typecheck/src/sdk'), { recursive: true });
await fs.cp('/tmp/vestra-current-runtime-cache/sdk', path.join(root, '.typecheck/src/sdk'), { recursive: true });
for (const file of ['main.ts', 'core.ts']) await fs.copyFile(path.join(root, file), path.join(check, file));
await fs.symlink(path.join(root, 'node_modules'), path.join(root, '.typecheck/node_modules')).catch(error => { if (error.code !== 'EEXIST') throw error; });
execFileSync(process.execPath, ['/tmp/vestra-rebuild-toolchain-20261018/node_modules/typescript/lib/_tsc.js', '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'ESNext', '--moduleResolution', 'Bundler', '--allowImportingTsExtensions', '--typeRoots', '/tmp/vestra-rebuild-toolchain-20261018/node_modules/@types', path.join(check, 'main.ts'), path.join(check, 'core.ts')], { stdio: 'inherit' });
await fs.mkdir(path.join(root, 'dist'), { recursive: true });
for (const file of ['main', 'core']) {
  const source = await fs.readFile(path.join(root, `${file}.ts`), 'utf8');
  const output = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022, verbatimModuleSyntax: true } });
  await fs.writeFile(path.join(root, 'dist', `${file}.js`), output.outputText);
}
