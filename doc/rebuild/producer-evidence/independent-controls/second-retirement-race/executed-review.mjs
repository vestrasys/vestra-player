/** Independent public two-runtime controls consumer; --run requires a fresh explicit binary freeze. */
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,readFile,copyFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';

async function provider(host){
  const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path');
  const work=process.env.VESTRA_CONTROLS_REVIEW,log={pass:false,pid:process.pid,states:[],controls:[],events:[]},sleep=ms=>new Promise(r=>setTimeout(r,ms));
  let pipe,prepared,epoch=1,next=0,sequence=0,withheld;
  async function record(){await fs.writeFile(path.join(work,'provider-result.json'),JSON.stringify(log,null,2));}
  function pixels(sample){const rgba=new Uint8Array(32*24*4);for(let i=0;i<rgba.length;i+=4){rgba[i]=(sample/480)%255;rgba[i+1]=80;rgba[i+2]=190;rgba[i+3]=255;}return rgba;}
  async function packet(frames=480){const start=next,pcm=new Float32Array(frames*2);for(let i=0;i<frames;i++){const v=.003*Math.sin(2*Math.PI*440*(start+i)/48000);pcm[i*2]=v;pcm[i*2+1]=v;}await pipe.video({epoch,sampleTime:start,sequence:sequence++},pixels(start));await pipe.audio({epoch,sampleTime:start,sequence:sequence++},pcm);next+=frames;}
  try{
    for(const event of ['source.state','host.error'])await host.events.subscribe(event,data=>log.events.push({at:Date.now(),event,data}));
    prepared=await host.producer.prepare({width:32,height:24});log.prepared=prepared;assert.equal(prepared.sampleRate,48000);assert.equal(prepared.channels,2);assert.equal(prepared.videoFormat,'rgba8');pipe=await host.producer.connect(prepared);
    let state,deadline=Date.now()+5000;while(Date.now()<deadline){state=await host.producer.state(prepared.producerId);if(state.acknowledged)break;await sleep(10);}assert.equal(state.acknowledged,true);await packet(2048);
    while(Date.now()<deadline){state=await host.producer.state(prepared.producerId);if(state.receivedFrames>=2048&&state.hasVideo)break;await sleep(10);}assert.ok(state.receivedFrames>=2048&&state.hasVideo);log.beforeOpen=state;
    log.open=await host.source.submit({kind:'openProvider',producerId:prepared.producerId,startPaused:false},1);assert.equal(log.open.status,'completed');log.owner=log.open.ticket.owner;
    const end=Date.now()+17000,seen=new Set();
    while(Date.now()<end){
      const source=await host.source.state();
      if(withheld&&source.phase==='ready'&&source.kind==='media'&&source.stamp.generation>withheld.stamp.generation){
        log.beforeLateAck=source;
        try{log.unexpectedLateAck=await host.producer.ack(withheld);throw Error('late ACK unexpectedly accepted');}catch(error){if(log.unexpectedLateAck)throw error;assert.ok(['unavailable','stale'].includes(error.code));log.lateAck={at:Date.now(),code:error.code,message:error.message,control:withheld};}
        await sleep(120);log.afterLateAck=await host.source.state();assert.deepEqual(log.afterLateAck.stamp,source.stamp);assert.equal(log.afterLateAck.kind,'media');assert.equal(log.afterLateAck.phase,'ready');assert.equal(log.afterLateAck.source,source.source);
        try{await host.producer.state(prepared.producerId);throw Error('retired producer still queryable');}catch(error){assert.equal(error.code,'unavailable');log.retiredProducerError={code:error.code,message:error.message};}
        await assert.rejects(fs.stat(prepared.socket),{code:'ENOENT'});log.socketRemoved=true;log.pass=true;await record();console.error('INDEPENDENT_CONTROLS_PROVIDER_DONE');return()=>pipe.close();
      }
      if(source.phase==='error'&&withheld){if(!log.timeoutErrorSource)log.timeoutErrorSource={at:Date.now(),state:source};await sleep(10);continue;}
      state=await host.producer.state(prepared.producerId);log.states.push({at:Date.now(),state});assert.ok(!['streamInvalidated','deviceUnavailable','otherError'].includes(state.health),'fatal device state');
      const control=state.pendingControl;
      if(control&&!seen.has(JSON.stringify(control))){
        seen.add(JSON.stringify(control));const item={observedAt:Date.now(),notice:control,source};log.controls.push(item);
        assert.equal(control.producerId,prepared.producerId);const sourceAtNotice=await host.source.state();item.sourceAtNotice=sourceAtNotice;assert.deepEqual(control.stamp,sourceAtNotice.stamp);assert.equal(control.cycle,sourceAtNotice.cycle);
        if(log.controls.length===3){withheld=control;item.withheld=true;await record();continue;}
        // Finite protocol boundary: intentionally allow the controller to observe Pending.
        // This delay is not evidence of a real emulator core frame boundary.
        await sleep(200);
        if(log.controls.length===1){
          try{await host.producer.ack({...control,baseSample:control.baseSample+1});throw Error('wrong ACK accepted');}catch(error){assert.equal(error.code,'stale');item.wrongAck={code:error.code,message:error.message};}
          assert.deepEqual((await host.producer.state(prepared.producerId)).pendingControl,control);
        }
        if(control.epoch!==epoch){epoch=control.epoch;next=control.baseSample;sequence=0;}
        item.ack=await host.producer.ack(control);item.ackAt=Date.now();assert.equal(item.ack.status,'acknowledged');
        item.replay=await host.producer.ack(control);assert.deepEqual(item.replay,item.ack);
        await record();continue;
      }
      if(!withheld&&!state.paused&&state.acknowledged&&next-state.consumedFrames<3840)await packet();
      await sleep(8);
    }
    throw Error('provider finite deadline exceeded');
  }catch(error){log.error=String(error);log.stack=error.stack;await record();pipe?.close();throw error;}
}

async function controller(host){
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
}

if(process.argv.includes('--run')){
  const work=await mkdtemp(resolve(tmpdir(),'vestra-independent-controls-'));
  const root=resolve(import.meta.dirname,'../../../..');
  for(const[name,activate]of [['provider',provider],['controller',controller]]){
    await mkdir(resolve(work,name));await writeFile(resolve(work,name,'manifest.json'),JSON.stringify({id:'independent.controls.'+name,apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['events','playback.read','playback.control',...(name==='provider'?['producer.av']:[])]}));
    await writeFile(resolve(work,name,'main.mjs'),'export const activate='+activate.toString()+';\n');
  }
  // A private harness-created ordinary replacement source; no plugin result channel.
  const wave=Buffer.alloc(44+48000*2*2*2);wave.write('RIFF');wave.writeUInt32LE(wave.length-8,4);wave.write('WAVEfmt ',8);wave.writeUInt32LE(16,16);wave.writeUInt16LE(1,20);wave.writeUInt16LE(2,22);wave.writeUInt32LE(48000,24);wave.writeUInt32LE(192000,28);wave.writeUInt16LE(4,32);wave.writeUInt16LE(16,34);wave.write('data',36);wave.writeUInt32LE(wave.length-44,40);await writeFile(resolve(work,'replacement.wav'),wave);
  const binary=resolve(root,'src/host/target/debug/vestra-host'),args=['--headless','--silent','--seconds','24','--plugin',resolve(work,'provider'),'--plugin',resolve(work,'controller')];
  const result=spawnSync(binary,args,{encoding:'utf8',timeout:35000,env:{...process.env,VESTRA_CONTROLS_REVIEW:work}});
  await writeFile(resolve(work,'host.stdout.log'),result.stdout??'');await writeFile(resolve(work,'host.stderr.log'),result.stderr??'');await writeFile(resolve(work,'command.json'),JSON.stringify({binary,args,work,exitCode:result.status,signal:result.signal,error:result.error?.message},null,2));await copyFile(import.meta.filename,resolve(work,'executed-review.mjs'));console.log(JSON.stringify({work,exitCode:result.status}));
  try{
    assert.equal(result.status,0);assert.match(result.stderr,/INDEPENDENT_CONTROLS_PROVIDER_DONE/);assert.match(result.stderr,/INDEPENDENT_CONTROLS_CONTROLLER_DONE/);
    const p=JSON.parse(await readFile(resolve(work,'provider-result.json'))),c=JSON.parse(await readFile(resolve(work,'controller-result.json')));assert.equal(p.pass,true);assert.equal(c.pass,true);assert.notEqual(p.pid,c.pid);assert.notEqual(p.owner,c.owner);assert.equal(p.controls.length,3);assert.equal(c.operations.length,3);
    for(let i=0;i<3;i++){assert.deepEqual(p.controls[i].notice.stamp,c.operations[i].receipt.ticket.stamp);assert.equal(p.controls[i].notice.cycle,c.operations[i].receipt.ticket.cycle);assert.equal(c.operations[i].receipt.ticket.owner,c.owner);}
    const summary={pass:true,providerPid:p.pid,controllerPid:c.pid,providerOwner:p.owner,controllerOwner:c.owner,operationStates:c.operations.map(x=>({id:x.id,initial:x.initial.status,terminal:x.terminal.status,durationMs:x.finishedAt-x.at})),lateAck:p.lateAck,retired:p.retiredProducerError,limits:['Two genuine isolated plugin runtimes and public source/producer handshake; harness reads results only after host termination.','Finite generated PCM/RGBA protocol boundaries, not physical acoustic output, keyboard or NES core frame-boundary proof.','Provider deliberately withholds third ACK; no mocking of host timeout/Book/CPAL acceptance.']};await writeFile(resolve(work,'result.json'),JSON.stringify(summary,null,2));console.log(JSON.stringify(summary));
  }catch(error){await writeFile(resolve(work,'review-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack},null,2));throw error;}
}else console.log('Prepared only. Run --run only after maintainer explicitly freezes source, SDK and binary.');
