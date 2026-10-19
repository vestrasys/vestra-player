export const activate=async function provider(host){
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
};
