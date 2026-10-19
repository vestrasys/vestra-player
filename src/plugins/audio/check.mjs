import { mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
const root = new URL('./',import.meta.url);
const check = new URL('.typecheck/',root);
await mkdir(check,{recursive:true});
try {
  for (const name of ['main','model','controller']) {
    const source = await readFile(new URL(`${name}.ts`,root),'utf8');
    await writeFile(new URL(`${name}.ts`,check),source.replaceAll('../../sdk/index.ts','/tmp/vestra-current-runtime-cache/sdk/index.ts'));
  }
  const result = spawnSync(process.execPath,['/tmp/vestra-rebuild-toolchain-20261018/node_modules/typescript/lib/_tsc.js','--noEmit','--strict','--skipLibCheck','--target','ES2023','--module','NodeNext','--moduleResolution','NodeNext','--allowImportingTsExtensions','--erasableSyntaxOnly','--typeRoots','/tmp/vestra-rebuild-toolchain-20261018/node_modules/@types',new URL('main.ts',check).pathname],{stdio:'inherit'});
  process.exitCode = result.status ?? 1;
} finally { await rm(check,{recursive:true,force:true}); }
