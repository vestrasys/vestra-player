export const activate=async function consumer(host){
 const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto'),{execFileSync}=await import('node:child_process');
 const work=process.env.VESTRA_MEDIA_JOBS_REVIEW,s=host.snapshot,j=host.jobs,checks={},events=[],timeline=[];
 const wait=ms=>new Promise(r=>setTimeout(r,ms)),hash=b=>createHash('sha256').update(b).digest('hex');let op=0;
 function children(){return execFileSync('ps',['-axo','pid=,ppid=,command='],{encoding:'utf8'}).split('\n').map(line=>/^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line)).filter(m=>m&&+m[2]===process.ppid&&/ffmpeg/.test(m[3])&&m[3].includes('/dev/fd/0')).map(m=>({pid:+m[1],ppid:+m[2],command:m[3]}));}
 async function ready(generation){const end=Date.now()+5000;while(Date.now()<end){const x=await host.source.state();if(x.phase==='ready'&&x.stamp.generation===generation)return x;await wait(20);}throw Error('source ready timeout');}
 async function reject(promise,code){try{await promise;throw Error('expected '+code);}catch(e){assert.equal(e.code,code,String(e));return{code:e.code,message:e.message};}}
 async function terminal(id){const end=Date.now()+7000;while(Date.now()<end){const x=await j.state(id);if(['completed','failed','cancelled'].includes(x.status))return x;await wait(15);}throw Error('job terminal timeout '+id);}
 async function cleanup(dir){const end=Date.now()+4000;let entries,ps;while(Date.now()<end){entries=await fs.readdir(dir);ps=children();if(!entries.some(name=>!/^owner-[01]\.png$/.test(name))&&!ps.length)return{entries,children:ps,noPublicationObserved:entries.length===0};await wait(20);}throw Error('owner worker/temp did not clean '+JSON.stringify({entries,ps}));}
 try{
  for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
  const userPath=path.join(work,'user.mp4');await fs.copyFile(path.join(work,'a.mp4'),userPath);
  const opened=await host.source.submit({kind:'open',source:userPath,startPaused:true}),a=await ready(opened.ticket.stamp.generation);
  assert.equal(a.capabilities.snapshotSource,true);assert.deepEqual(await j.operation(1),{status:'unknown'});
  const capturePayload={operationId:++op,expected:a.stamp,time:12},snapshot=await s.capture(capturePayload);
  assert.equal(snapshot.kind,'raw');assert.equal(snapshot.source,userPath);assert.equal(snapshot.time,12);assert.deepEqual(await s.state(snapshot.snapshotId),snapshot);
  assert.deepEqual(await s.capture(capturePayload),snapshot);assert.deepEqual(await j.operation(op),{status:'completed',result:snapshot});await reject(s.capture({...capturePayload,time:13}),'conflict');
  const before=await fs.stat(userPath);await fs.rename(userPath,path.join(work,'old-a.mp4'));await fs.copyFile(path.join(work,'b.mp4'),userPath);await fs.unlink(path.join(work,'old-a.mp4'));const after=await fs.stat(userPath);assert.notEqual(before.ino,after.ino);
  const bReceipt=await host.source.submit({kind:'open',source:userPath,startPaused:false}),b=await ready(bReceipt.ticket.stamp.generation);assert.ok(b.stamp.generation>a.stamp.generation);
  const staleId=++op,stalePayload={operationId:staleId,expected:a.stamp,time:12},staleError=await reject(s.capture(stalePayload),'stale');assert.deepEqual(await j.operation(staleId),{status:'failed',error:staleError});assert.deepEqual(await reject(s.capture(stalePayload),'stale'),staleError);
  checks.snapshotIdentity={snapshot,a,b,identityBefore:{dev:before.dev,ino:before.ino},identityAfter:{dev:after.dev,ino:after.ino}};
  const starts=[];
  for(const filename of ['a-output.png','a-second.png']){
   const payload={operationId:++op,snapshotId:snapshot.snapshotId,kind:'rawPng',destination:path.join(work,'artifacts',filename)},started=await j.start(payload);starts.push(started);
   assert.deepEqual(await j.start(payload),started);assert.deepEqual(await j.operation(op),{status:'completed',result:started});await reject(j.start({...payload,destination:payload.destination+'.different'}),'conflict');
  }
  await s.release(snapshot.snapshotId,++op);await reject(s.state(snapshot.snapshotId),'unknown-expired');
  const deadline=Date.now()+7000;
  while(Date.now()<deadline){
   const jobs=await Promise.all(starts.map(x=>j.state(x.jobId))),source=await host.source.state(),render=await host.playback.state(),ps=children();timeline.push({wallMs:Date.now(),jobs,source,render,children:ps});
   if(jobs.every(x=>['completed','failed','cancelled'].includes(x.status)))break;await wait(20);
  }
  const done=await Promise.all(starts.map(x=>terminal(x.jobId)));for(const x of done)assert.equal(x.status,'completed',JSON.stringify(x));
  const active=timeline.filter(x=>x.children.length&&x.jobs.some(j=>j.status==='running'&&!j.publicationStarted));
  let interval=null;
  for(let first=0;first<active.length&&!interval;first++)for(let last=first+1;last<active.length;last++)if(active[last].source.position>active[first].source.position+.03&&active[last].render.videoFrames>active[first].render.videoFrames){interval={before:active[first],after:active[last]};break;}
  checks.releasedSnapshotJobs={starts,done,observedWorkerProgress:interval,workerProgressCovered:interval!==null};
  const current=await host.source.state(),bSnapshot=await s.capture({operationId:++op,expected:current.stamp,time:20});
  const existing=path.join(work,'artifacts','existing.png'),original=await fs.readFile(existing),collision=await j.start({operationId:++op,snapshotId:bSnapshot.snapshotId,kind:'rawPng',destination:existing}),collisionDone=await terminal(collision.jobId);
  assert.equal(collisionDone.status,'failed');assert.deepEqual(await fs.readFile(existing),original);checks.noClobber={collisionDone,sha256:hash(original)};
  const cancelledPath=path.join(work,'cancelled','cancelled.png'),cancelJob=await j.start({operationId:++op,snapshotId:bSnapshot.snapshotId,kind:'rawPng',destination:cancelledPath}),cancel=await j.cancel(cancelJob.jobId,++op),cancelDone=await terminal(cancelJob.jobId);
  if(cancel.cancel==='requested'){assert.equal(cancelDone.status,'cancelled');await assert.rejects(fs.stat(cancelledPath),{code:'ENOENT'});assert.deepEqual(await fs.readdir(path.dirname(cancelledPath)),[]);}
  else assert.ok(['tooLate','terminal'].includes(cancel.cancel));checks.cancel={cancel,cancelDone,acceptedCancellationCovered:cancel.cancel==='requested'};
  await host.plugins.load(path.join(work,'helper'));
  let helper;const helperDeadline=Date.now()+5000;while(Date.now()<helperDeadline){try{helper=JSON.parse(await fs.readFile(path.join(work,'helper-started.json'),'utf8'));break;}catch(e){if(e.code!=='ENOENT')throw e;}await wait(5);}assert.ok(helper);
  const beforeDisableChildren=children();await host.plugins.disable('review.media-jobs-helper');
  const ownerCleanup=await cleanup(path.join(work,'owner-cancel'));assert.ok(!(await j.list()).some(x=>helper.jobs.some(h=>h.jobId===x.jobId)));checks.ownerDisable={helper,beforeDisableChildren,cleanup:ownerCleanup,limits:'Final owner PNGs, if any, can have won Publishing before disable; noPublicationObserved records the actual case. Owner jobs become inaccessible on disable; no post-disable public terminal is claimed. Pending pre-publication jobs cancel; already publishing can complete.'};
  const sourceAfter=await host.source.state(),renderAfter=await host.playback.state();assert.equal(sourceAfter.phase,'ready');assert.equal(sourceAfter.stamp.generation,b.stamp.generation);assert.ok(sourceAfter.position>b.position);assert.ok(renderAfter.videoFrames>0);
  await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify({pass:true,checks,timeline,events,sourceAfter,renderAfter},null,2));console.error('MEDIA_JOBS_REVIEW_DONE');
 }catch(error){await fs.writeFile(path.join(work,'consumer-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,checks,timeline,events},null,2));throw error;}
};
