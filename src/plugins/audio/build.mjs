import { mkdir, readFile, writeFile, cp, rm } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const root = new URL('./', import.meta.url);
const dist = new URL('./dist/', root);
const version = process.argv[2] ?? '1.1.0';
if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version) || version.split('.').some(part => Number(part) > 4294967295)) throw new Error('Version requires three u32 components without leading zeros');
await rm(dist, { recursive: true, force: true }); await mkdir(dist, { recursive: true });
for (const name of ['main','model','controller']) {
  const text = stripTypeScriptTypes(await readFile(new URL(`${name}.ts`,root),'utf8')).replace(/(['"])\.\/(model|controller)\.ts\1/g,'$1./$2.js$1');
  if (/from\s*['"][^'"]*sdk\//.test(text)) throw new Error('Runtime SDK import forbidden');
  await writeFile(new URL(`${name}.js`,dist),text);
}
const manifest = JSON.parse(await readFile(new URL('manifest.json',root),'utf8'));
await writeFile(new URL('manifest.json',dist),JSON.stringify({...manifest,entry:'main.js',version},null,2)+'\n');
await writeFile(new URL('package.json',dist),'{"type":"module"}\n');
await cp(new URL('ui/',root),new URL('ui/',dist),{recursive:true});
console.log(`Built audio ${version}`);
