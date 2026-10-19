export const activate=async function activate(host){
 const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path');
 const work=process.env.VESTRA_ROTATION_REVIEW,mode=process.env.VESTRA_ROTATION_MODE,dir=path.join(work,mode),phases=[],events=[],rawJobs=[];
 const wait=ms=>new Promise(r=>setTimeout(r,ms));let mediaOp=0,processingRevision=0;
 async function records(){try{return(await fs.readFile(path.join(dir,'frames/frames.jsonl'),'utf8')).split('\n').slice(0,-1).filter(Boolean).map(JSON.parse);}catch(e){if(e.code==='ENOENT')return[];throw e;}}
 async function ready(generation,cycle=0){const end=Date.now()+4500;while(Date.now()<end){const s=await host.source.state();if(s.phase==='ready'&&s.stamp.generation===generation&&s.cycle>=cycle)return s;await wait(20);}throw Error('ready timeout');}
 async function stable(name,fixture){await wait(350);const first=(await records()).length;await wait(500);const last=(await records()).length;assert.ok(last-first>=2,'enough bounded settled samples '+name);const source=await host.source.state();assert.equal(source.phase,'ready');assert.equal(source.paused,true);assert.equal(source.source,path.join(work,fixture+'.mp4'));phases.push({name,fixture,first,last,source,processing:await host.processing.state()});return source;}
 async function raw(fixture){const source=await host.source.state();assert.equal(source.capabilities.snapshotSource,true);const snap=await host.snapshot.capture({operationId:++mediaOp,expected:source.stamp,time:1}),destination=path.join(dir,'artifacts',fixture+'.png'),job=await host.jobs.start({operationId:++mediaOp,snapshotId:snap.snapshotId,kind:'rawPng',destination});await host.snapshot.release(snap.snapshotId,++mediaOp);const observed=[];let state;const end=Date.now()+4500;while(Date.now()<end){state=await host.jobs.state(job.jobId);observed.push(state);if(['completed','failed','cancelled'].includes(state.status))break;await wait(20);}assert.equal(state.status,'completed',JSON.stringify(state));rawJobs.push({fixture,snap,job,observed,destination});}
 async function open(fixture,name=fixture){const receipt=await host.source.submit({kind:'open',source:path.join(work,fixture+'.mp4'),startPaused:true});await ready(receipt.ticket.stamp.generation);return stable(name,fixture);}
 async function eq(brightness){const source=await host.source.state();await host.processing.replace({stage:'video',resourceId:'tone',revision:++processingRevision,expectedGeneration:source.stamp.generation,nodes:[{kind:'eq',brightness,contrast:1,saturation:1,gamma:1}]});return source;}
 try{
  for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
  if(mode==='main'){
   await open('r0');await raw('r0');const ninety=await open('r90');await raw('r90');
   await host.source.submit({kind:'restart',expected:ninety.stamp});const restarted=await ready(ninety.stamp.generation,ninety.cycle+1);await host.source.submit({kind:'control',control:{kind:'pause',paused:true},expected:restarted.stamp});
   const r=await stable('r90-restart','r90');assert.equal(r.stamp.generation,ninety.stamp.generation);assert.ok(r.cycle>ninety.cycle);
   const before=await eq(.03);const e1=await stable('r90-effect-1','r90');assert.deepEqual(e1.stamp,before.stamp);assert.equal(e1.cycle,before.cycle);assert.ok(Math.abs(e1.position-before.position)<.001);
   await eq(.05);const e2=await stable('r90-effect-2','r90');assert.deepEqual(e2.stamp,e1.stamp);assert.equal(e2.cycle,e1.cycle);
   await host.plugins.load(path.join(work,'helper'));const end=Date.now()+3500;let helper;while(Date.now()<end){try{helper=JSON.parse(await fs.readFile(path.join(dir,'helper-ready.json'),'utf8'));break;}catch(e){if(e.code!=='ENOENT')throw e;}await wait(15);}assert.ok(helper);
   const mounted=await stable('r90-owner-effect','r90');await host.plugins.disable('review.rotation-helper');const removed=await stable('r90-owner-revoked','r90');assert.deepEqual(removed.stamp,mounted.stamp);assert.ok(!(await host.processing.state()).video.some(x=>x.resource.manifestId==='review.rotation-helper'));
   await open('r0','plain-after-rotated');await open('rn90');await raw('rn90');
  }else{for(const fixture of ['r180','sar90','sar0']){await open(fixture);await raw(fixture);}}
  await fs.writeFile(path.join(dir,'consumer-result.json'),JSON.stringify({pass:true,phases,events,rawJobs},null,2));console.error('ROTATION_REVIEW_DONE '+mode);
 }catch(error){await fs.writeFile(path.join(dir,'consumer-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,phases,events,rawJobs},null,2));throw error;}
};
