import { WorkshopJob, defaultRequirement } from './engine.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = path.resolve(process.argv[2] || '/tmp/vestra-current-products/workshop/evidence');
await fs.mkdir(root,{recursive:true});
for (const [version,step] of [['1.0.0',1],['1.1.0',3]]) {
 const job=new WorkshopJob(path.join(root,version));
 const result=await job.run(JSON.stringify({kind:'native-counter',id:'community.counter',version,step}));
 assert.equal(result.actions,8);assert.equal(result.turns,3);assert.match(result.reports[0].stdout,/TS2322/);assert.equal(result.reports[1].code,0);
 // Test generated production JS only through its ordinary Host contract, no private counter import.
 const callbacks=[];const properties=[];
 const host={ui:{onCallback:f=>{callbacks.push(f);return ()=>{};},open:async()=>{},set:async(...a)=>properties.push(a),close:async()=>{}}};
 const plugin=await import(path.join(result.directory,'main.js')); // boundary-reviewed: generated candidate under test
 const dispose=await plugin.activate(host);
 for(const callback of ['increase','increase','reset']){callbacks[0]({view:'counter',callback,args:[]});await new Promise(resolve=>setImmediate(resolve));}
 assert.deepEqual(properties.map(p=>p[2]),[String(step),String(step*2),'0']);await dispose();
 const resultReport={version,step,actions:result.actions,turns:result.turns,diagnostics:result.reports.map(r=>({code:r.code,stdout:r.stdout})),ordinaryHostCallbackBehavior:properties.map(p=>p[2]),scope:'real Pi Agent and real TypeScript; generated JS ordinary Host contract test; Slint/managed host not tested'};
 await fs.writeFile(path.join(root,`result-${version}.json`),JSON.stringify(resultReport,null,2));console.log(JSON.stringify(resultReport));
}
