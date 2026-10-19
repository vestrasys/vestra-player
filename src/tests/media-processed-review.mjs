/** Independent processed-PNG candidate, execute only after final binary freeze.
 * Actual pixels and frozen bytes/revisions; no MP4/audio or full-M3 claim.
 */
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';
const oldText='旧字幕 <b>字面</b> {\\b1} \\N OLD';
const newText='新字幕 CURRENT 中文';
const srt=text=>`1\n00:00:01,000 --> 00:00:02,000\n${text}\n\n`;
const vtt=text=>`WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n${text.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;')}\n\n`;
const sha=b=>createHash('sha256').update(b).digest('hex');
async function consumer(host){
 const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto');
 const work=process.env.VESTRA_PROCESSED_REVIEW,outputs=[],checks={},events=[],wait=ms=>new Promise(r=>setTimeout(r,ms)),hash=b=>createHash('sha256').update(b).digest('hex');
 let mediaOp=0,subtitleOp=0,upload=0,styleRevision=0,processingRevision=0;
 const eq=b=>({kind:'eq',brightness:b,contrast:1,saturation:1,gamma:1});
 async function conditions(){return{source:await host.source.state(),processing:await host.processing.state(),subtitle:await host.subtitles.state()};}
 function payload(c,time){return{operationId:++mediaOp,expected:c.source.stamp,time,mode:'processed',expectedProcessingRevision:c.processing.configurationRevision,expectedSubtitle:{selection:c.subtitle.selection.stamp,presentationRevision:c.subtitle.presentation.revision}};}
 async function capture(time,overrides={}){const c=await conditions();return host.snapshot.capture({...payload(c,time),...overrides});}
 async function output(snapshot,name,kind='processedPng'){const destination=path.join(work,'artifacts',name+'.png'),job=await host.jobs.start({operationId:++mediaOp,snapshotId:snapshot.snapshotId,kind,destination}),observed=[];const end=Date.now()+8000;let terminal;while(Date.now()<end){terminal=await host.jobs.state(job.jobId);observed.push(terminal);if(['completed','failed','cancelled'].includes(terminal.status))break;await wait(20);}assert.equal(terminal.status,'completed',JSON.stringify(terminal));outputs.push({name,snapshot,job,observed,destination});return terminal;}
 async function single(time,name){const snap=await capture(time);await output(snap,name);await host.snapshot.release(snap.snapshotId,++mediaOp);return snap;}
 async function open(name){const r=await host.source.submit({kind:'open',source:path.join(work,name+'.mp4'),startPaused:true});const end=Date.now()+5000;let state;while(Date.now()<end){state=await host.source.state();if(state.phase==='ready'&&state.stamp.generation===r.ticket.stamp.generation)break;await wait(20);}assert.equal(state.phase,'ready');await wait(400);return host.source.state();}
 async function applyStyle(offsetMs=0){const state=await host.subtitles.state(),style={fontId:state.presentation.fonts[0].id,fontSize:72,color:'cyan',outline:1,marginPercent:5,offsetMs};assert.equal(style.fontId,'noto-cjk-sc-regular-2.004');return host.subtitles.style({operationId:++subtitleOp,revision:++styleRevision,expected:state.selection.stamp,style});}
 async function commit(text,resourceId='current',revision=1){const state=await host.subtitles.state(),bytes=Buffer.from(`1\n00:00:01,000 --> 00:00:02,000\n${text}\n\n`),id=++upload;await host.subtitles.begin(id,state.source.generation,bytes.length);await host.subtitles.append(id,0,bytes.toString('base64'));const result=await host.subtitles.commit({operationId:++subtitleOp,uploadId:id,sha256:hash(bytes),resourceId,revision,expected:state.selection.stamp});return result;}
 async function reject(promise,code){try{await promise;throw Error('expected '+code);}catch(e){assert.equal(e.code,code,String(e));return{code:e.code,message:e.message};}}
 try{
  for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
  await open('a');await host.plugins.load(path.join(work,'helper'));
  const helperEnd=Date.now()+5000;let helper;while(Date.now()<helperEnd){try{helper=JSON.parse(await fs.readFile(path.join(work,'helper-ready.json'),'utf8'));break;}catch(e){if(e.code!=='ENOENT')throw e;}await wait(15);}assert.ok(helper);
  const oldConditions=await conditions(),frozen=await capture(1.5);assert.equal(frozen.kind,'processed');assert.equal(frozen.processing.revision,oldConditions.processing.configurationRevision);assert.deepEqual(frozen.subtitle.selectionStamp,oldConditions.subtitle.selection.stamp);assert.equal(frozen.subtitle.presentationRevision,oldConditions.subtitle.presentation.revision);assert.equal(frozen.subtitle.canonical.digest,hash(await fs.readFile(path.join(work,'old.srt'))));assert.equal(frozen.subtitle.derived.digest,hash(await fs.readFile(path.join(work,'old.vtt'))));assert.equal(frozen.subtitle.font.digest,process.env.VESTRA_PROCESSED_FONT_DIGEST);assert.equal(frozen.subtitle.font.faceIndex,0);assert.equal(frozen.subtitle.coverageValidated,true);assert.equal(frozen.subtitle.style.offsetMs,250);assert.deepEqual(frozen.presentation,{width:1280,height:360});
  assert.ok(frozen.processing.video.some(r=>r.manifestId==='review.processed-helper'&&r.nodes.some(n=>n.kind==='gblur'&&n.sigma===1.3)));
  await output(frozen,'old-before');
  // Main owns the snapshot; contribution owner stop must not drop its leased assets.
  await host.plugins.disable('review.processed-helper');const noHelper=await conditions();assert.ok(!noHelper.processing.video.some(r=>r.resource.manifestId==='review.processed-helper'));assert.ok(!noHelper.subtitle.resources.some(r=>r.owner===helper.subtitle.selection.target.owner));
  await host.processing.replace({stage:'video',resourceId:'current',revision:++processingRevision,expectedGeneration:noHelper.source.stamp.generation,nodes:[eq(-.1)]});
  await commit(process.env.VESTRA_PROCESSED_NEW_TEXT);await applyStyle(0);await single(1.5,'new-a');
  // Each independent stale condition must reject, while the retained old snapshot stays exact.
  const now=await conditions();
  for(const [name,overrides]of [['processing',{expectedProcessingRevision:oldConditions.processing.configurationRevision}],['selection',{expectedSubtitle:{selection:oldConditions.subtitle.selection.stamp,presentationRevision:now.subtitle.presentation.revision}}],['presentation',{expectedSubtitle:{selection:now.subtitle.selection.stamp,presentationRevision:oldConditions.subtitle.presentation.revision}}]]){
   const request={...payload(now,1.5),...overrides},error=await reject(host.snapshot.capture(request),'stale');assert.deepEqual(await host.jobs.operation(request.operationId),{status:'failed',error});assert.deepEqual(await host.snapshot.state(frozen.snapshotId),frozen);checks['stale-'+name]={request,error};
  }
  await open('b');await commit(process.env.VESTRA_PROCESSED_NEW_TEXT);const styled=await applyStyle(0);const target=styled.selection.target;
  await output(frozen,'old-after-source-owner-changes');assert.deepEqual(await host.snapshot.state(frozen.snapshotId),frozen);await host.snapshot.release(frozen.snapshotId,++mediaOp);checks.frozenOld={frozen,helper,oldConditions};
  const bsource=await host.source.state(),raw=await host.snapshot.capture({operationId:++mediaOp,expected:bsource.stamp,time:1.5});assert.equal(raw.kind,'raw');await output(raw,'raw-b-unbaked','rawPng');await host.snapshot.release(raw.snapshotId,++mediaOp);
  await single(1.5,'b-on');await single(.5,'b-before-cue');await single(2.1,'b-after-cue');
  let sub=await host.subtitles.state();await host.subtitles.select(null,sub.selection.stamp,++subtitleOp);const off=await single(1.5,'b-explicit-off');assert.equal(off.subtitle,null);
  sub=await host.subtitles.state();await host.subtitles.select(target,sub.selection.stamp,++subtitleOp);await applyStyle(500);await single(1.4,'b-delay-before');await single(1.6,'b-delay-on');
  await applyStyle(-500);await single(.6,'b-advance-on');await single(1.6,'b-advance-after');
  // Unknown cmap character is explicit unsupported, not silently replaced/fallback.
  const missing=await commit(String.fromCodePoint(0x10ffff),'missing',1),mc=await conditions(),missingPayload=payload(mc,1.5),missingError=await reject(host.snapshot.capture(missingPayload),'unsupported');assert.deepEqual(await host.jobs.operation(missingPayload.operationId),{status:'failed',error:missingError});checks.missingGlyph={target:missing.selection.target,error:missingError};
  sub=await host.subtitles.state();await host.subtitles.select(null,sub.selection.stamp,++subtitleOp);
  await host.processing.replace({stage:'video',resourceId:'current',revision:++processingRevision,expectedGeneration:(await host.source.state()).stamp.generation,nodes:[{kind:'gblur',sigma:2,steps:3}]});
  const blur=await single(1.5,'b-storage-blur-before-sar');assert.equal(blur.subtitle,null);checks.blurSnapshot=blur;
  const final=await host.source.state();assert.equal(final.paused,true);assert.equal(final.position,0);
  await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify({pass:true,outputs,checks,events,final},null,2));console.error('MEDIA_PROCESSED_REVIEW_DONE');
 }catch(error){await fs.writeFile(path.join(work,'consumer-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,outputs,checks,events},null,2));throw error;}
}
async function helper(host){
 const fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto'),work=process.env.VESTRA_PROCESSED_REVIEW,state=await host.source.state();
 const processing=await host.processing.replace({stage:'video',resourceId:'old',revision:1,expectedGeneration:state.stamp.generation,nodes:[{kind:'eq',brightness:.12,contrast:1,saturation:1,gamma:1},{kind:'gblur',sigma:1.3,steps:3}]});
 const bytes=await fs.readFile(path.join(work,'old.srt')),s=host.subtitles,initial=await s.state();await s.begin(1,state.stamp.generation,bytes.length);await s.append(1,0,bytes.toString('base64'));const subtitle=await s.commit({operationId:1,uploadId:1,sha256:createHash('sha256').update(bytes).digest('hex'),resourceId:'old',revision:1,expected:initial.selection.stamp});
 const style=await s.style({operationId:2,revision:1,expected:subtitle.selection.stamp,style:{fontId:subtitle.presentation.fonts[0].id,fontSize:72,color:'yellow',outline:1,marginPercent:5,offsetMs:250}});await fs.writeFile(path.join(work,'helper-ready.json'),JSON.stringify({processing,subtitle,style},null,2));
}
function command(binary,args,options={}){if(binary==='ffmpeg')binary='/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg';const x=spawnSync(binary,args,{encoding:'utf8',...options});assert.equal(x.status,0,x.stderr?.toString());return x;}
function decode(bytes){const h=/^P6\s+(\d+)\s+(\d+)\s+255\s/.exec(bytes.toString('ascii',0,100));assert.ok(h);const width=+h[1],height=+h[2],rgb=bytes.subarray(h[0].length);assert.equal(rgb.length,width*height*3);return{width,height,rgb};}
function delta(a,b){assert.equal(a.width,b.width);assert.equal(a.height,b.height);let sum=0,exactDiffChannels=0,maxAbsDiff=0;for(let i=0;i<a.rgb.length;i++){const d=Math.abs(a.rgb[i]-b.rgb[i]);sum+=d;if(d)exactDiffChannels++;maxAbsDiff=Math.max(maxAbsDiff,d);}return{mae:sum/a.rgb.length,exactDiffChannels,maxAbsDiff};}
function colorCount(image,color){let n=0;for(let y=Math.floor(image.height*.5);y<image.height;y++)for(let x=0;x<image.width;x++){const i=(y*image.width+x)*3,[r,g,b]=image.rgb.subarray(i,i+3);if(color==='yellow'?r>180&&g>180&&b<100:r<100&&g>180&&b>180)n++;}return n;}
const analyzeIndex=process.argv.indexOf('--analyze-work');
if(process.argv.includes('--run')||analyzeIndex>=0){
 const work=analyzeIndex>=0?resolve(process.argv[analyzeIndex+1]):await mkdtemp(resolve(tmpdir(),'vestra-media-processed-review-'));
 const font=await readFile(resolve(import.meta.dirname,'../assets/fonts/noto-cjk/NotoSansCJKsc-Regular.otf')),fontDigest=sha(font);
 if(analyzeIndex<0){for(const d of ['artifacts','fonts','references'])await mkdir(resolve(work,d));
 await writeFile(resolve(work,'fonts/NotoSansCJKsc-Regular.otf'),font);await writeFile(resolve(work,'old.srt'),srt(oldText));await writeFile(resolve(work,'old.vtt'),vtt(oldText));
 for(const[name,marker]of [['a','red'],['b','blue']])command('ffmpeg',['-v','error','-f','lavfi','-i',`color=c=gray:size=640x360:rate=30,drawgrid=w=8:h=8:t=1:c=white,drawbox=x=0:y=0:w=120:h=120:color=${marker}:t=fill,setsar=2`,'-t','8','-c:v','libx264','-preset','ultrafast',resolve(work,name+'.mp4')]);
 for(const[name,fn]of [['consumer',consumer],['helper',helper]]){const d=resolve(work,name);await mkdir(d);await writeFile(resolve(d,'manifest.json'),JSON.stringify({id:`review.processed-${name}`,apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['playback.read','subtitles.read','subtitles.control','processing.video',...(name==='consumer'?['playback.control','events','plugins.manage','media.read','media.control']:[])]}));await writeFile(resolve(d,'main.mjs'),`export const activate=${fn.toString()};\n`);}
 const args=['--headless','--silent','--seconds','18','--plugin',resolve(work,'consumer')],child=spawnSync(resolve(import.meta.dirname,'../host/target/debug/vestra-host'),args,{encoding:'utf8',timeout:28000,env:{...process.env,VESTRA_PROCESSED_REVIEW:work,VESTRA_PROCESSED_FONT_DIGEST:fontDigest,VESTRA_PROCESSED_NEW_TEXT:newText}});
 await writeFile(resolve(work,'host.stdout.log'),child.stdout??'');await writeFile(resolve(work,'host.stderr.log'),child.stderr??'');await writeFile(resolve(work,'command.json'),JSON.stringify({work,args,exitCode:child.status,signal:child.signal},null,2));console.log(JSON.stringify({work,exitCode:child.status}));assert.equal(child.status,0,child.stderr);assert.match(child.stderr,/MEDIA_PROCESSED_REVIEW_DONE/);
 }else console.log(JSON.stringify({work,analysisOnly:true,ffmpeg:'/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg'}));
 const c=JSON.parse(await readFile(resolve(work,'consumer-result.json'),'utf8')),images={},analysis={};
 try{
  for(const out of c.outputs){const png=await readFile(out.destination);assert.deepEqual(png.subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));const rgb=command('ffmpeg',['-v','error','-i',out.destination,'-frames:v','1','-f','image2pipe','-vcodec','ppm','-'],{encoding:null,maxBuffer:8*1024*1024});images[out.name]=decode(rgb.stdout);analysis[out.name]={width:images[out.name].width,height:images[out.name].height,sha256:sha(png),yellowPixels:colorCount(images[out.name],'yellow'),cyanPixels:colorCount(images[out.name],'cyan')};}
  const square="scale=w='max(1,round(iw*sar))':h=ih,setsar=1",force='FontName=Noto Sans CJK SC,FontSize=24,PrimaryColour=&H0000FFFF,SecondaryColour=&H0000FFFF,OutlineColour=&H00000000,BackColour=&H00000000,BorderStyle=1,Outline=0.3333333333333333,Shadow=0,Alignment=2,MarginL=9,MarginR=9,MarginV=18,Bold=0,Italic=0,Spacing=0,ScaleX=1,ScaleY=1,PlayResX=1280,PlayResY=360,ScaledBorderAndShadow=yes';
  const recipes={old:{source:'a',filter:`eq=brightness=0.12:contrast=1:saturation=1:gamma=1,gblur=sigma=1.3:sigmaV=1.3:steps=3,${square},setpts=PTS-0.25/TB,subtitles=filename='${resolve(work,'old.vtt')}':fontsdir='${resolve(work,'fonts')}':force_style='${force}',setpts=PTS+0.25/TB,trim=start=1.5,setpts=PTS-STARTPTS`},raw:{source:'b',filter:`${square},trim=start=1.5,setpts=PTS-STARTPTS`},storageBlur:{source:'b',filter:`gblur=sigma=2:sigmaV=2:steps=3,${square},trim=start=1.5,setpts=PTS-STARTPTS`},wrongSarFirst:{source:'b',filter:`${square},gblur=sigma=2:sigmaV=2:steps=3,trim=start=1.5,setpts=PTS-STARTPTS`}};
  const refs={};for(const[name,recipe]of Object.entries(recipes)){const dest=resolve(work,'references',name+'.png');command('ffmpeg',['-v','error','-threads','1','-filter_threads','1','-i',resolve(work,recipe.source+'.mp4'),'-vf',recipe.filter,'-frames:v','1','-pix_fmt','rgb24','-c:v','png',dest]);refs[name]=decode(command('ffmpeg',['-v','error','-i',dest,'-frames:v','1','-f','image2pipe','-vcodec','ppm','-'],{encoding:null,maxBuffer:8*1024*1024}).stdout);}
  const comparisons={frozenUnchanged:delta(images['old-before'],images['old-after-source-owner-changes']),oldRecipe:delta(images['old-before'],refs.old),newRecipeDiffers:delta(images['old-before'],images['new-a']),rawUnbaked:delta(images['raw-b-unbaked'],refs.raw),storageBlur:delta(images['b-storage-blur-before-sar'],refs.storageBlur),wrongSarFirst:delta(images['b-storage-blur-before-sar'],refs.wrongSarFirst)};
  assert.equal(comparisons.frozenUnchanged.exactDiffChannels,0);assert.ok(comparisons.oldRecipe.mae<=1,'old processed image matches independently written recipe');assert.ok(comparisons.newRecipeDiffers.mae>2);assert.ok(analysis['old-before'].yellowPixels>100);assert.ok(analysis['new-a'].cyanPixels>100);assert.ok(comparisons.rawUnbaked.mae<=1);assert.ok(comparisons.storageBlur.mae<=1);assert.ok(comparisons.wrongSarFirst.mae>comparisons.storageBlur.mae+.1,'SAR2 fixture distinguishes storage blur -> SAR from wrong order');
  const offNames=['b-before-cue','b-after-cue','b-explicit-off','b-delay-before','b-advance-after'],onNames=['b-delay-on','b-advance-on'];comparisons.off={};comparisons.on={};
  for(const name of offNames){comparisons.off[name]=delta(images['b-explicit-off'],images[name]);assert.equal(comparisons.off[name].exactDiffChannels,0,name+' must have no subtitle pixels');}
  for(const name of onNames){comparisons.on[name]=delta(images['b-on'],images[name]);assert.equal(comparisons.on[name].exactDiffChannels,0,name+' must show same shifted cue');}
  assert.ok(analysis['b-on'].cyanPixels>100);assert.equal(analysis['b-explicit-off'].cyanPixels,0);assert.ok(delta(images['b-on'],images['b-explicit-off']).mae>.05);
  const result={pass:true,analysis,comparisons,fontDigest,checks:c.checks,limits:['Only processed PNG with controlled external SRT/fixed Noto; native subtitle/MP4/audio/preroll/device/offline sound not validated.','Font digest/cmap/recipe rendered pixels checked for this finite text; no claim about arbitrary scripts/shaping or malicious in-place source writes.','Raw remains unbaked; storage-pixel blur before SAR proved against deliberately wrong-order reference.','No native PTS/source frame fence or all scheduler interleavings/failed-backend rollback proof.']};await writeFile(resolve(work,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({pass:true,work,comparisons}));
 }catch(error){await writeFile(resolve(work,'pixel-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,analysis,consumer:c},null,2));throw error;}
}
