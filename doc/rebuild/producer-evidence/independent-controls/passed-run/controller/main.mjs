export const activate=async function controller(host){
  const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path'),sleep=ms=>new Promise(r=>setTimeout(r,ms));
  const work=process.env.VESTRA_CONTROLS_REVIEW,log={pass:false,pid:process.pid,events:[],operations:[]};
  async function until(test,message,timeout=6000){const end=Date.now()+timeout;let last;while(Date.now()<end){last=await test();if(last)return last;await sleep(15);}throw Error(message);}
  async function sourceWhen(test,message,timeout){return until(async()=>{const state=await host.source.state();return test(state)&&state;},message,timeout);}
  async function control(paused,id){
    const before=await host.source.state(),at=Date.now(),receipt=await host.source.submit({kind:'control',control:{kind:'pause',paused},expected:before.stamp},id);assert.equal(receipt.status,'pending');
    const initial=await host.source.operation(id);assert.equal(initial.status,'pending');assert.deepEqual(initial.ticket,receipt.ticket);
    const observations=[];const terminal=await until(async()=>{const op=await host.source.operation(id);observations.push({at:Date.now(),op});return op.status!=='pending'&&op;},'operation terminal '+id,7000);
    const source=await host.source.state(),item={id,paused,at,before,receipt,initial,terminal,observations,source,finishedAt:Date.now()};log.operations.push(item);return item;
  }
  try{
    for(const event of ['source.state','host.error'])await host.events.subscribe(event,data=>log.events.push({at:Date.now(),event,data}));
    log.initial=await sourceWhen(s=>s.kind==='producer'&&s.phase==='ready'&&!s.paused,'provider Ready missing');
    log.progressBefore=await sourceWhen(s=>s.kind==='producer'&&s.position>.04,'sample clock never progressed');
    const pause=await control(true,101);assert.equal(pause.terminal.status,'completed');assert.equal(pause.source.paused,true);log.owner=pause.receipt.ticket.owner;
    const pausedPosition=pause.source.position;await sleep(180);log.pausedLater=await host.source.state();assert.equal(log.pausedLater.position,pausedPosition);assert.equal(log.pausedLater.paused,true);
    const resume=await control(false,102);assert.equal(resume.terminal.status,'completed');assert.equal(resume.source.paused,false);assert.equal(resume.receipt.ticket.owner,log.owner);
    log.resumed=await sourceWhen(s=>s.kind==='producer'&&!s.paused&&s.position>pausedPosition+.04,'resume sample clock missing');
    const timeout=await control(true,103);assert.equal(timeout.terminal.status,'failed');assert.match(JSON.stringify(timeout.terminal.error),/timed out/i);assert.ok(timeout.finishedAt-timeout.at>=4900,'must actually wait near5s');
    log.errorSource=await sourceWhen(s=>s.kind==='producer'&&s.phase==='error','timeout must expose producer Error');assert.equal(log.errorSource.paused,true);assert.match(JSON.stringify(log.errorSource.terminalReason),/timed out/i);
    const errorPosition=log.errorSource.position;await sleep(120);log.errorLater=await host.source.state();assert.equal(log.errorLater.position,errorPosition);assert.equal(log.errorLater.phase,'error');
    log.replacement=await host.source.submit({kind:'open',source:path.join(work,'replacement.wav'),startPaused:true,expected:log.errorLater.stamp},104);assert.equal(log.replacement.status,'completed');
    log.newSource=await sourceWhen(s=>s.kind==='media'&&s.phase==='ready'&&s.stamp.generation>log.errorSource.stamp.generation,'replacement media not Ready');
    await sleep(450);log.afterLateWindow=await host.source.state();assert.deepEqual(log.afterLateWindow.stamp,log.newSource.stamp);assert.equal(log.afterLateWindow.phase,'ready');assert.equal(log.afterLateWindow.source,log.newSource.source);assert.equal(log.afterLateWindow.paused,true);
    log.timeoutReplay=await host.source.operation(103);assert.deepEqual(log.timeoutReplay,timeout.terminal);assert.equal(log.events.filter(e=>e.event==='host.error'&&e.data.code==='producer-failed').length,1);
    log.pass=true;await fs.writeFile(path.join(work,'controller-result.json'),JSON.stringify(log,null,2));console.error('INDEPENDENT_CONTROLS_CONTROLLER_DONE');
  }catch(error){log.error=String(error);log.stack=error.stack;await fs.writeFile(path.join(work,'controller-result.json'),JSON.stringify(log,null,2));throw error;}
};
