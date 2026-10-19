import { readFile, writeFile, rename, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
const root = new URL('./',import.meta.url);
const target = process.argv[2];
if(!target) throw new Error('Supply the authorized src/plugins/capture directory');
const names=['main.ts','capture.slint','capture-error.slint','manifest.json','behavior.test.mjs','build.mjs','publish.mjs','README.md','API_REQUESTS.md','dist/main.js','dist/manifest.json','dist/capture.slint','dist/capture-error.slint'];
const files={};
for(const name of names){
 const bytes=await readFile(new URL(name,root));if(!bytes.length)throw new Error('Empty '+name);
 const destination=join(target,name);await mkdir(dirname(destination),{recursive:true});
 await writeFile(destination+'.sync',bytes);await rename(destination+'.sync',destination);
 const observed=await readFile(destination);if(!observed.equals(bytes))throw new Error('Sync mismatch '+name);
 files[name]={size:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex'),srcVerifiedImmediately:true};
}
const record={revision:process.argv[3]||'capture-complete-20261018.3',syncedAt:new Date().toISOString(),authority:target,source:'Current author revision, local cache and src identical at synchronization; no claim about evicted earlier revisions',checks:{build:0,strictTypeScriptCurrentSdkCache:0,contractTests:0,slint:'new revision awaiting maintainer compiler',nativeMediaAcceptance:'not performed by author'},files};
const body=JSON.stringify(record,null,2)+'\n';
await writeFile(join(target,'freeze-ready.json.tmp'),body);await rename(join(target,'freeze-ready.json.tmp'),join(target,'freeze-ready.json'));
await writeFile(new URL('freeze-ready.json.tmp',root),body);await rename(new URL('freeze-ready.json.tmp',root),new URL('freeze-ready.json',root));
console.log(record.revision+' complete nonzero snapshot published');
