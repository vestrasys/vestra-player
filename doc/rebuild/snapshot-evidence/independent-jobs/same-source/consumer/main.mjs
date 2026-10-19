export const activate=async function sameSourceConsumer(host){
 const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path'),{execFileSync}=await import('node:child_process');
 const work=process.env.VESTRA_MEDIA_JOBS_REVIEW,timeline=[],events=[];let snapshot,starts=[];
 const wait=ms=>new Promise(r=>setTimeout(r,ms));
 function children(){return execFileSync('ps',['-axo','pid=,ppid=,command='],{encoding:'utf8'}).split('\n').map(line=>/^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line)).filter(m=>m&&+m[2]===process.ppid&&/ffmpeg/.test(m[3])&&m[3].includes('/dev/fd/0')).map(m=>({pid:+m[1],ppid:+m[2],command:m[3]}));}
 try{
  for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
  const receipt=await host.source.submit({kind:'open',source:path.join(work,'a.mp4'),startPaused:false});
  const readyEnd=Date.now()+5000;let source;
  while(Date.now()<readyEnd){source=await host.source.state();if(source.phase==='ready'&&source.stamp.generation===receipt.ticket.stamp.generation)break;await wait(20);}
  assert.equal(source.phase,'ready');assert.equal(source.paused,false);assert.equal(source.capabilities.snapshotSource,true);
  snapshot=await host.snapshot.capture({operationId:1,expected:source.stamp,time:12});
  for(let i=0;i<2;i++)starts.push(await host.jobs.start({operationId:i+2,snapshotId:snapshot.snapshotId,kind:'rawPng',destination:path.join(work,'artifacts',`same-a-${i}.png`)}));
  const deadline=Date.now()+7000;
  while(Date.now()<deadline){
   const jobs=await Promise.all(starts.map(x=>host.jobs.state(x.jobId))),state=await host.source.state(),render=await host.playback.state(),ps=children();
   assert.equal(state.source,path.join(work,'a.mp4'));assert.equal(state.stamp.generation,source.stamp.generation);assert.equal(state.paused,false);
   timeline.push({wallMs:Date.now(),jobs,source:state,render,children:ps});
   if(jobs.every(j=>['completed','failed','cancelled'].includes(j.status)))break;await wait(20);
  }
  const final=await Promise.all(starts.map(x=>host.jobs.state(x.jobId)));for(const job of final)assert.equal(job.status,'completed',JSON.stringify(job));
  let interval=null;
  const active=timeline.filter(row=>row.children.length&&row.jobs.some(j=>j.status==='running'&&!j.publicationStarted));
  for(let i=0;i<active.length&&!interval;i++)for(let k=i+1;k<active.length;k++){
   const sameChildPids=active[k].children.filter(x=>active[i].children.some(y=>x.pid===y.pid)).map(x=>x.pid);
   if(sameChildPids.length&&active[k].source.position>active[i].source.position+.03&&active[k].render.videoFrames>active[i].render.videoFrames){interval={sameChildPids,elapsedMs:active[k].wallMs-active[i].wallMs,before:active[i],after:active[k]};break;}
  }
  const cleanEnd=Date.now()+3000;let residualChildren;
  while(Date.now()<cleanEnd){residualChildren=children();if(!residualChildren.length)break;await wait(20);}
  const entries=await fs.readdir(path.join(work,'artifacts'));assert.deepEqual(entries.filter(x=>x.startsWith('.tmp')),[]);assert.deepEqual(residualChildren,[]);
  await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify({pass:interval!==null,snapshot,starts,final,interval,timeline,events,cleanup:{entries,children:residualChildren}},null,2));
  console.error(interval?'MEDIA_JOBS_SAME_SOURCE_DONE':'MEDIA_JOBS_SAME_SOURCE_INSUFFICIENT_WINDOW');
 }catch(error){await fs.writeFile(path.join(work,'consumer-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,snapshot,starts,timeline,events},null,2));throw error;}
};
