// Preparation is inert unless --run is explicitly passed after freeze approval.
import {mkdtemp, mkdir, writeFile, readFile, copyFile, cp, readdir} from 'node:fs/promises';
import {resolve, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {tmpdir} from 'node:os';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
const here = dirname(fileURLToPath(import.meta.url));
if (!process.argv.includes('--run')) throw new Error('Requires --run and a separately granted frozen host execution slot');
const root = await mkdtemp(resolve(tmpdir(), 'vestra-independent-packages-'));
const contents = resolve(root, 'frozen/Contents');
await mkdir(resolve(contents, 'MacOS'), {recursive: true});
await mkdir(resolve(contents, 'Resources'), {recursive: true});
const binary = resolve(contents, 'MacOS/host');
await copyFile(resolve('src/host/target/debug/vestra-host'), binary);
for (const name of ['runtime', 'sdk']) await cp(resolve('src', name), resolve(contents, 'Resources', name), {recursive: true});
console.log(JSON.stringify({root, binary}));
for (const [name, version, bad] of [['one', '1.0.0', false], ['two', '1.1.0', false], ['bad', '1.2.0', true]]) {
  const directory = resolve(root, name);
  await mkdir(directory);
  await writeFile(resolve(directory, 'manifest.json'), JSON.stringify({id: 'independent.counter',
    apiVersion: 'vestra.experimental/0.1', version, entry: 'main.mjs', capabilities: ['commands']}));
  await writeFile(resolve(directory, 'counter.slint'), 'export component Counter inherits Window { in-out property <int> count: 0; Text { text: "Count " + root.count; } }');
  await writeFile(resolve(directory, 'main.mjs'), `export async function activate(host) {
    const fs = await import('node:fs/promises');
    await host.commands.register('increase',1,{title:'Independent counter ${version}',path:['插件','Independent counter','Increase'],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});
    let count=0; let observed=Promise.resolve();
    host.commands.onExecute(event=>{ if(event.id!=='increase')return; count+=${name === 'two' ? 7 : 1}; const value=count;
      observed=observed.then(()=>fs.appendFile(process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT+'/commands.jsonl',JSON.stringify({run:process.env.VESTRA_INDEPENDENT_PACKAGE_RUN,version:${JSON.stringify(version)},pid:process.pid,count:value})+'\\n')); });
    await host.commands.execute('increase');
    await host.commands.execute('increase');
    while(count!==${name === 'two' ? 14 : 2})await new Promise(resolve=>setTimeout(resolve,5));
    await observed;
    await fs.appendFile(process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT+'/activations.jsonl',JSON.stringify({run:process.env.VESTRA_INDEPENDENT_PACKAGE_RUN,version:${JSON.stringify(version)},pid:process.pid})+'\\n');
    ${bad ? "throw new Error('independent activation failure after command registration');" : ''}
  }`);
}
const manager = resolve(root, 'manager');
await mkdir(manager);
await writeFile(resolve(manager, 'manifest.json'), JSON.stringify({id: 'independent.package.manager',
  apiVersion: 'vestra.experimental/0.1', entry: 'main.mjs', capabilities: ['plugins.manage']}));
await copyFile(resolve(here, 'consumer.mjs'), resolve(manager, 'main.mjs'));
const observer = resolve(root, 'observer');
await mkdir(observer);
await writeFile(resolve(observer, 'manifest.json'), JSON.stringify({id:'independent.package.observer',apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['plugins.manage']}));
await copyFile(resolve(here, 'observer.mjs'), resolve(observer, 'main.mjs'));
const sha256 = value => createHash('sha256').update(value).digest('hex');
const before = sha256(await readFile(binary));
async function hashes(directory, prefix='') {
  const entries=[];
  for(const entry of await readdir(directory,{withFileTypes:true})) {
    const name=prefix+entry.name;
    if(entry.isDirectory()) entries.push(...await hashes(resolve(directory,entry.name),name+'/'));
    else entries.push({file:name,sha256:sha256(await readFile(resolve(directory,entry.name)))});
  }
  return entries.sort((a,b)=>a.file.localeCompare(b.file));
}
const frozenHashes=await hashes(contents);
await writeFile(resolve(root,'frozen-sha256.json'),JSON.stringify(frozenHashes,null,2));
await writeFile(resolve(root, 'command.json'), JSON.stringify({binary, sha256: before,
  consumerSha256: sha256(await readFile(resolve(here, 'consumer.mjs'))),
  harnessSha256: sha256(await readFile(fileURLToPath(import.meta.url))),
  limits: ['Headless: no native window/button/focus claims.', 'Slint file compilation only.',
    'No crash recovery/failure injection/fsync fault coverage.', 'Finite 512 retained request receipts is an explicit experimental capacity.']}, null, 2));
for (const phase of ['lifecycle', 'reopen']) {
  const args = ['--headless', '--silent', '--seconds', '90', '--plugin', manager,
    ...(phase==='lifecycle'?['--plugin',observer]:[])];
  const child = spawn(binary, args, {env: {...process.env, VESTRA_PACKAGE_STORE: resolve(root, 'store'),
    VESTRA_INDEPENDENT_PACKAGE_ROOT: root, VESTRA_INDEPENDENT_PACKAGE_PHASE: phase,
    VESTRA_INDEPENDENT_PACKAGE_RUN: phase}, stdio: ['ignore', 'pipe', 'pipe']});
  let stdout = '', stderr = '';
  child.stdout.on('data', data => { stdout += data; });
  child.stderr.on('data', data => { stderr += data; });
  const exited = new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('exit', (code, signal) => resolve({code, signal}));
  });
  let result;
  const deadline = Date.now() + 85000;
  while (Date.now() < deadline) {
    try { result = JSON.parse(await readFile(resolve(root, `${phase}-result.json`), 'utf8')); break; }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (child.exitCode !== null || child.signalCode !== null) break;
    await new Promise(resolve => setTimeout(resolve, 60));
  }
  child.kill('SIGTERM');
  const hardStop = setTimeout(() => child.kill('SIGKILL'), 5000);
  const exit = await exited;
  clearTimeout(hardStop);
  await writeFile(resolve(root, `${phase}-stdout.log`), stdout);
  await writeFile(resolve(root, `${phase}-stderr.log`), stderr);
  await writeFile(resolve(root, `${phase}-process.json`), JSON.stringify({args, exit, result: result ?? null}, null, 2));
  if (!result?.passed) throw new Error(`Independent ${phase} failed; evidence ${root}`);
  if (JSON.stringify(await hashes(contents))!==JSON.stringify(frozenHashes)) throw new Error('Frozen host/runtime/SDK changed during independent run');
}
console.log(JSON.stringify({root, passed: true}));
