/** Frozen host only: minimal actual software-RGB subtitle on/off loop.
 * Diagnostics have frameSourceAssociation:none, never an atomic source/PTS fence.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp,mkdir,writeFile,readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
const work=await mkdtemp(resolve(tmpdir(),'vestra-subtitle-pixel-review-'));
const frames=resolve(work,'frames'); await mkdir(frames);
const made=spawnSync('ffmpeg',['-v','error','-f','lavfi','-i','color=c=gray:size=640x360:rate=30','-t','16','-c:v','ffv1',resolve(work,'gray.mkv')],{encoding:'utf8'});
assert.equal(made.status,0,made.stderr);
async function activate(host) {
  const fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto');
  const assert=(await import('node:assert/strict')).default;
  const work=process.env.VESTRA_SUBTITLE_PIXEL_REVIEW,phases=[],events=[];
  const wait=ms=>new Promise(r=>setTimeout(r,ms));
  async function records() {
    try{return (await fs.readFile(path.join(work,'frames','frames.jsonl'),'utf8')).split('\n').slice(0,-1).filter(Boolean).map(JSON.parse);}
    catch(e){if(e.code==='ENOENT')return [];throw e;}
  }
  async function stable(name) {
    await wait(800); const first=(await records()).length; await wait(700); const last=(await records()).length;
    assert.ok(last-first>=2,'at least two settled samples');
    const source=await host.source.state(); assert.equal(source.phase,'ready');assert.equal(source.paused,true);
    phases.push({name,first,last,source,subtitles:await host.subtitles.state()});
  }
  try {
    for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
    const receipt=await host.source.submit({kind:'open',source:path.join(work,'gray.mkv'),startPaused:true});
    const deadline=Date.now()+5000;
    while(Date.now()<deadline){const state=await host.source.state();if(state.phase==='ready'&&state.stamp.generation===receipt.ticket.stamp.generation)break;await wait(20);}
    await stable('baseline');
    const s=host.subtitles,current=await s.state();
    // Literal HTML/ASS-looking bytes are carried through the public SRT->VTT path.
    // Pixel differences below prove visible subtitle on/off, not exact glyph transcription.
    const literal='LITERAL <b>TEXT</b> {\\b1} ASS \\N END';
    const bytes=Buffer.from(`1\n00:00:00,000 --> 00:00:15,000\n${literal}\n\n`);
    await s.begin(1,current.source.generation,bytes.length);await s.append(1,0,bytes.toString('base64'));
    const committed=await s.commit({operationId:1,uploadId:1,sha256:createHash('sha256').update(bytes).digest('hex'),resourceId:'literal',revision:1,expected:current.selection.stamp});
    assert.equal(committed.confirmation,'applied');assert.equal(committed.selection.target.kind,'external');
    await stable('subtitle-on');
    const selected=await s.state();await s.select(null,selected.selection.stamp,2);
    await stable('subtitle-off');
    const first=phases[0].source;
    for(const phase of phases){assert.deepEqual(phase.source.stamp,first.stamp);assert.equal(phase.source.cycle,first.cycle);assert.ok(Math.abs(phase.source.position-first.position)<.001,'paused position preserved');}
    assert.equal(phases[2].subtitles.selection.target,null);
    await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify({phases,events,literal,committed,final:await host.source.state()},null,2));
    console.error('SUBTITLE_PIXEL_CONSUMER_DONE');
  }catch(error){await fs.writeFile(path.join(work,'consumer-failure.json'),JSON.stringify({error:String(error),stack:error.stack,phases,events},null,2));throw error;}
}
const fixture=resolve(work,'consumer');await mkdir(fixture);
await writeFile(resolve(fixture,'manifest.json'),JSON.stringify({id:'review.subtitle-pixel',apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['subtitles.read','subtitles.control','playback.read','playback.control','events']}));
await writeFile(resolve(fixture,'main.mjs'),`export const activate=${activate.toString()};\n`);
const args=['--headless','--silent','--seconds','8','--frame-evidence-dir',frames,'--plugin',fixture];
const child=spawnSync(resolve(import.meta.dirname,'../host/target/debug/vestra-host'),args,{encoding:'utf8',timeout:16000,env:{...process.env,VESTRA_SUBTITLE_PIXEL_REVIEW:work}});
await writeFile(resolve(work,'host.stdout.log'),child.stdout??'');await writeFile(resolve(work,'host.stderr.log'),child.stderr??'');
await writeFile(resolve(work,'command.json'),JSON.stringify({work,args,exitCode:child.status,signal:child.signal},null,2));
console.log(JSON.stringify({work,exitCode:child.status}));assert.equal(child.status,0,child.stderr);assert.match(child.stderr,/SUBTITLE_PIXEL_CONSUMER_DONE/);
const consumer=JSON.parse(await readFile(resolve(work,'consumer-result.json'),'utf8'));
const records=(await readFile(resolve(frames,'frames.jsonl'),'utf8')).trim().split('\n').map(JSON.parse);
async function ppm(record){
 assert.equal(record.frameSourceAssociation,'none');const data=await readFile(resolve(frames,record.file));
 const header=/^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(data.toString('ascii',0,80));assert.ok(header);
 const width=+header[1],height=+header[2],rgb=data.subarray(header[0].length);assert.equal(rgb.length,width*height*3);
 return {record,width,height,rgb};
}
function difference(a,b){
 assert.equal(a.width,b.width);assert.equal(a.height,b.height);
 let sum=0,count=0,x0=a.width,y0=a.height,x1=-1,y1=-1;
 for(let y=0;y<a.height;y++)for(let x=0;x<a.width;x++){
  const i=(y*a.width+x)*3;let max=0;
  for(let c=0;c<3;c++){const d=Math.abs(a.rgb[i+c]-b.rgb[i+c]);sum+=d;max=Math.max(max,d);}
  if(max>12){count++;x0=Math.min(x0,x);y0=Math.min(y0,y);x1=Math.max(x1,x);y1=Math.max(y1,y);}
 }
 return {mae:sum/a.rgb.length,changedPixels:count,bbox:count?{x0,y0,x1,y1}:null};
}
const analysis={};
try{
 const windows=[];
 for(const phase of consumer.phases){const samples=await Promise.all(records.slice(phase.first,phase.last).map(ppm));assert.ok(samples.length>=2);
  const stability=samples.slice(1).map(s=>difference(samples[0],s));for(const d of stability)assert.ok(d.mae<.01,'window RGB must be stable');
  analysis[phase.name]={samples:samples.map(s=>s.record),stability};windows.push(samples);
 }
 const baseline=windows[0][0],on=windows[1],off=windows[2];
 analysis.onDifferences=on.map(s=>difference(baseline,s));analysis.offDifferences=off.map(s=>difference(baseline,s));
 for(const d of analysis.onDifferences){assert.ok(d.changedPixels>200,'actual subtitle pixels required');assert.ok(d.bbox.y0>baseline.height*.4,'bounded subtitle region expected');assert.ok(d.bbox.x0>0&&d.bbox.x1<baseline.width-1,'not global frame change');}
 for(const d of analysis.offDifferences)assert.ok(d.mae<.01&&d.changedPixels===0,'off restores actual baseline RGB');
 const result={pass:true,analysis,source:consumer.phases.map(p=>({name:p.name,source:p.source})),limits:['Actual settled headless RGB subtitle-on difference and exact off restoration on one static paused source.','Literal <b>/ASS/backslash-N bytes submitted through public SRT commit and derived VTT, but bbox/MAE does not prove every glyph is literal or prove font/style support.','Observed source snapshots are not frame PTS/source fences; repeated softwareSequence may represent one cached settled output.','No GUI, physical display, media audio track/device, translation service or backend failure injection.']};
 await writeFile(resolve(work,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({pass:true,on:analysis.onDifferences,off:analysis.offDifferences}));
}catch(error){await writeFile(resolve(work,'pixel-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,analysis,consumer},null,2));throw error;}
