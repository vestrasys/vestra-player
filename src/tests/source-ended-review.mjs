/** Independent real-host natural EOF -> seek -> resume -> second EOF check. */
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';
const work=await mkdtemp(resolve(tmpdir(),'vestra-source-ended-review-'));
const media=resolve(work,'short.mp4');
const made=spawnSync('ffmpeg',['-v','error','-f','lavfi','-i','testsrc2=size=160x90:rate=20','-t','1.2','-c:v','libx264','-preset','ultrafast',media],{encoding:'utf8'});
assert.equal(made.status,0,made.stderr);
async function activate(host){
 const fs=await import('node:fs/promises'); const path=await import('node:path');
 const work=process.env.VESTRA_ENDED_REVIEW; const records=[];const events=[];
 const wait=ms=>new Promise(r=>setTimeout(r,ms));let id=0;
 const submit=intent=>host.source.submit(intent,++id);
 async function sample(label){const source=await host.source.state();const playback=await host.playback.state();const value={label,wall:Date.now(),source,playback};records.push(value);return value;}
 async function until(predicate,label){const deadline=Date.now()+4000;while(Date.now()<deadline){const s=await sample(label);if(predicate(s.source))return s;await wait(25);}throw Error('timeout: '+label);}
 await host.events.subscribe('source.state',data=>events.push({kind:'source.state',wall:Date.now(),data}));
 await host.events.subscribe('playback.backend',data=>events.push({kind:'playback.backend',wall:Date.now(),data}));
 let pass=false,error;
 try{
  await submit({kind:'open',source:path.join(work,'short.mp4'),startPaused:false});
  const first=await until(s=>s.phase==='ended'&&s.terminalReason==='natural','first-natural-ended');
  if(!first.source.capabilities.seek)throw Error('ended source does not expose seek capability');
  await submit({kind:'control',control:{kind:'seek',seconds:0.2},expected:first.source.stamp});
  const sought=await sample('after-seek');
  await submit({kind:'control',control:{kind:'pause',paused:false},expected:sought.source.stamp});
  const resumeStart=Date.now();
  while(Date.now()-resumeStart<1800){await sample('after-resume');await wait(25);}
  const initial=first.source;
  const resumed=records.filter(r=>r.label==='after-resume');
  const ready=resumed.find(r=>r.source.phase==='ready'&&r.source.position>0.2);
  const ended=resumed.find(r=>r.source.phase==='ended'&&r.source.terminalReason==='natural'&&r.source.cycle>initial.cycle);
  const frameAdvance=resumed.at(-1).playback.videoFrames-first.playback.videoFrames;
  if(!ready||!ended)throw Error(JSON.stringify({message:'seek/resume must leave ended then naturally end a new playback cycle',readyObserved:!!ready,secondNaturalCycleObserved:!!ended,frameAdvance}));
  pass=true;
 }catch(e){error={message:String(e),stack:e.stack};}
 await fs.writeFile(path.join(work,pass?'result.json':'failure.json'),JSON.stringify({pass,error,records,events,limits:['Headless actual libmpv software frames; no native draw or device audio.']},null,2));
 console.error(pass?'SOURCE_ENDED_REVIEW_PASS':'SOURCE_ENDED_REVIEW_FAILURE',JSON.stringify(error));
}
const fixture=resolve(work,'consumer');await mkdir(fixture);
await writeFile(resolve(fixture,'manifest.json'),JSON.stringify({id:'review.source-ended',apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['playback.read','playback.control','events']}));
await writeFile(resolve(fixture,'main.mjs'),`export const activate=${activate.toString()};\n`);
const args=['--headless','--silent','--seconds','8','--plugin',fixture];
const result=spawnSync(resolve(import.meta.dirname,'../host/target/debug/vestra-host'),args,{encoding:'utf8',timeout:15000,env:{...process.env,VESTRA_ENDED_REVIEW:work}});
await writeFile(resolve(work,'host.stderr.log'),result.stderr??'');await writeFile(resolve(work,'host.stdout.log'),result.stdout??'');
await writeFile(resolve(work,'command.json'),JSON.stringify({args,exitCode:result.status,signal:result.signal,work},null,2));
console.log(JSON.stringify({work,exitCode:result.status}));assert.equal(result.status,0,result.stderr);
let report;try{report=JSON.parse(await readFile(resolve(work,'result.json'),'utf8'));}catch(e){if(e.code!=='ENOENT')throw e;report=JSON.parse(await readFile(resolve(work,'failure.json'),'utf8'));}
console.log(JSON.stringify({pass:report.pass,error:report.error}));if(!report.pass)process.exitCode=1;
