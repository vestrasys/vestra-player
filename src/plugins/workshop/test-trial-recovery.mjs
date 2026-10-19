// Control-plane regression only. Fake generation and Host transport do NOT certify
// real Pi, Slint, trial, installation or rollback. Real Pi evidence is separate.
import ts from './vendor/typescript/lib/typescript.js';
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
const directory=path.dirname(new URL(import.meta.url).pathname);
const fakeEngine=`export const defaultRequirement='{}'; export function requirement() {return {};}; export class WorkshopJob { constructor(directory){this.directory=directory;this.requirement={id:'community.counter'};this.cancelled=false;} async run(){return {directory:this.directory,revision:4};} cancel(){this.cancelled=true;} }`;
const source=(await fs.readFile(path.join(directory,'main.ts'),'utf8')).replace("'./engine.mjs'",JSON.stringify('data:text/javascript,'+encodeURIComponent(fakeEngine)));
const compiled=path.join(directory,'evidence/workflow-test-main.mjs');await fs.mkdir(path.dirname(compiled),{recursive:true});await fs.writeFile(compiled,ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText);
const {activate}=await import(pathToFileURL(compiled).href); // boundary-reviewed: own main.ts compiled to a temp file
const delay=()=>new Promise(resolve=>setTimeout(resolve,30));
const out=[];
async function fixture(overrides={}){
 let callback,command,dialogCalls=0;const props=new Map();const calls=[];
 const packages={validate:async()=>({token:'validated',digest:'digest'}),installed:async()=>({revision:7}),install:async()=>({operationId:'unused',status:'committed'}),rollback:async()=>({operationId:'unused',status:'committed'}),operation:async id=>({operationId:id,status:'committed'}),trial:async()=>({trialId:1,status:'running'}),stopTrial:async id=>({trialId:id,status:'stopped',cleanupConfirmed:true}),trialState:async id=>({trialId:id,status:'running'}),cancel:async id=>({operationId:id,status:'cancelled'}),...overrides};
 const host={ui:{onCallback:f=>(callback=f,()=>{}),open:async()=>{},set:async(_id,k,v)=>props.set(k,v),show:async()=>{},close:async()=>{}},commands:{onExecute:f=>(command=f,()=>{}),register:async()=>{}},dialogs:{open:async()=>{dialogCalls++;return {cancelled:false,paths:[directory]};}},packages};
 const dispose=await activate(host);command({id:'workshop.open'});await delay();
 const send=async name=>{callback({view:'workshop',callback:name,args:[]});await delay();};
 return {props,packages,dispose,send,get dialogCalls(){return dialogCalls;}};
}
for (const mode of ['recover-ready','query-timeout-then-recover']) {
 let count=0,requestId,queries=[],stopIds=[],rejectTrial,first=true;
 const f=await fixture({
  trial:(_token,id)=>{count++;requestId=id;return new Promise((_resolve,reject)=>rejectTrial=reject);},
  trialStateByRequest:async id=>{queries.push(id);if(mode==='query-timeout-then-recover'&&first){first=false;throw Object.assign(Error('query timed out'),{code:'timeout'});}return {trialId:42,status:'running'};},
  stopTrial:async id=>{stopIds.push(id);return {trialId:id,status:'stopped',cleanupConfirmed:true};}
 });
 await f.send('choose-workspace');await f.send('generate');await f.send('start-trial');await f.send('start-trial');
 assert.equal(count,1);assert.ok(requestId);
 rejectTrial(Object.assign(Error('first trial reply lost'),{code:'timeout'}));await delay();
 if(mode==='query-timeout-then-recover'){
   assert.equal(f.props.get('busy'),true);assert.equal(f.props.get('can-recover'),true);
   await f.send('start-trial');assert.equal(count,1);await f.send('recover-trial');
 }
 assert.equal(f.props.get('busy'),false);assert.equal(f.props.get('can-stop'),true);assert.equal(f.props.get('can-recover'),false);
 await f.send('stop-trial');assert.deepEqual(stopIds,[42]);assert.equal(f.props.get('can-trial'),true);
 assert.ok(queries.every(id=>id===requestId));assert.equal(count,1);await f.dispose();
 out.push({case:mode,passed:true,trialSubmissions:count,requestId,queries,stoppedOriginalTrial:stopIds});
}
await fs.writeFile(path.join(directory,'evidence/trial-recovery-regressions.json'),JSON.stringify({scope:'ordinary Host protocol regression with fake generation; not managed trial acceptance',results:out},null,2));console.log(JSON.stringify(out,null,2));
