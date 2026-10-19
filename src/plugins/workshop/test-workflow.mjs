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
for(const code of ['unavailable','timeout']){
 const f=await fixture({installed:async()=>{throw Object.assign(Error('read failure'),{code});}});
 await f.send('rollback-version');assert.equal(f.props.get('busy'),false);
 await f.send('choose-workspace');assert.equal(f.dialogCalls,1);assert.doesNotMatch(f.props.get('status-text'),/正在查询同一operationId/);
 await f.dispose();out.push({case:`installed ${code}`,passed:true,busy:false,newWorkspaceAllowed:true});
}
for(const action of ['install-candidate','rollback-version']){
 let requestId,submissions=0,lookups=[];
 const f=await fixture({install:async(_token,_rev,id)=>{requestId=id;submissions++;throw Object.assign(Error('lost accepted response'),{code:'timeout'});},rollback:async(_id,_rev,id)=>{requestId=id;submissions++;throw Object.assign(Error('lost accepted response'),{code:'timeout'});},operation:async id=>{lookups.push(id);return {operationId:id,status:'committed'};}});
 await f.send('choose-workspace');await f.send('generate');await f.send(action);
 assert.equal(submissions,1);assert.deepEqual(lookups,[requestId]);assert.equal(f.props.get('busy'),false);
 await f.dispose();out.push({case:`${action} accepted response timeout`,passed:true,submissions,lookups,requestId});
}
{
 let count=0,rejectTrial;const f=await fixture({trial:()=>{count++;return new Promise((_resolve,reject)=>rejectTrial=reject);}});
 await f.send('choose-workspace');await f.send('generate');await f.send('start-trial');await f.send('start-trial');assert.equal(count,1);
 rejectTrial(Object.assign(Error('trial response lost'),{code:'timeout'}));await delay();await f.send('start-trial');assert.equal(count,1);assert.equal(f.props.get('busy'),true);
 await f.dispose();out.push({case:'double trial and unknown first response',passed:true,trialSubmissions:count,unknownLocked:true,scope:'owner cleanup requires real host acceptance'});
}
await fs.writeFile(path.join(directory,'evidence/workflow-regressions.json'),JSON.stringify({scope:'ordinary Host protocol state regression; fake generation; not managed acceptance',results:out},null,2));console.log(JSON.stringify(out,null,2));
