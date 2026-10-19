export const activate=async function consumer(host){
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
};
