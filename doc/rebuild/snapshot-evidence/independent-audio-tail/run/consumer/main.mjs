export const activate=async function consumer(host){
  const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path');
  const work=process.env.VESTRA_AUDIO_TAIL_REVIEW,result={pass:false,jobs:[],events:[]},sleep=ms=>new Promise(r=>setTimeout(r,ms));let op=0;
  async function terminal(id){const states=[],end=Date.now()+10000;while(Date.now()<end){const state=await host.jobs.state(id);states.push({at:Date.now(),state});if(['completed','failed','cancelled'].includes(state.status))return{state,states};await sleep(15);}throw Error('finite job deadline exceeded '+id);}
  try{
    for(const event of ['source.state','host.error'])await host.events.subscribe(event,data=>result.events.push({event,data}));
    result.open=await host.source.submit({kind:'open',source:path.join(work,'source-video2-audio4.mkv'),startPaused:true},1);assert.equal(result.open.status,'completed');
    const deadline=Date.now()+5000;let source;while(Date.now()<deadline){source=await host.source.state();if(source.phase==='ready'&&source.stamp.generation===result.open.ticket.stamp.generation&&source.audioTracks?.length)break;await sleep(20);}
    assert.equal(source.phase,'ready');assert.equal(source.paused,true);assert.equal(source.duration,4,'fixture metadata is deliberately longer than video');assert.equal(source.audioTracks.length,1);assert.equal(source.audioTracks[0].streamIndex,1);result.source=source;
    for(const mode of ['raw','processed'])for(const invalidTail of [true,false]){
      const name=mode+(invalidTail?'-video-short':'-valid'),start=invalidTail?1.5:.5,end=invalidTail?3:1.5,processing=await host.processing.state();
      const capture={operationId:++op,expected:source.stamp,time:start,audio:{kind:'selected'},...(mode==='processed'?{mode:'processed',includeSubtitles:false,expectedProcessingRevision:processing.configurationRevision}:{})};
      const snapshot=await host.snapshot.capture(capture);assert.equal(snapshot.kind,mode);assert.equal(snapshot.audio.streamIndex,1);assert.equal(snapshot.videoStreamIndex,0);assert.equal(snapshot.timeline.containerStartTime,0);if(mode==='processed'){assert.equal(snapshot.subtitle,null);assert.deepEqual(snapshot.processing.video,[]);assert.deepEqual(snapshot.processing.audio,[]);assert.equal(snapshot.audioOffsetMs,0);}
      const destination=path.join(work,'outputs',name+'.mp4'),request={operationId:++op,snapshotId:snapshot.snapshotId,kind:mode==='raw'?'rawMp4':'processedMp4',end,destination},receipt=await host.jobs.start(request),done=await terminal(receipt.jobId);
      const record={name,invalidTail,capture,snapshot,request,receipt,terminal:done.state,states:done.states};result.jobs.push(record);
      assert.equal(done.state.facts.hasAudio,true);assert.deepEqual(done.state.facts.range.start,start);assert.equal(done.state.facts.range.end,end);assert.equal(done.state.facts.sourceTimeline.validation,'normalized-origin-matched');
      if(invalidTail){
        assert.equal(done.state.status,'failed');assert.equal(done.state.outcome.error.code,'export-failed');assert.match(done.state.outcome.error.message,/source-unavailable.*video.*shorter/i);assert.equal(done.state.publicationStarted,false);await assert.rejects(fs.stat(destination),{code:'ENOENT'});
        const output=done.state.facts.output;assert.ok(output,'current worker retains actual encoded stream facts before refusing publication');const v=output.streams.find(x=>x.codec_type==='video'),a=output.streams.find(x=>x.codec_type==='audio');assert.ok(v&&a);assert.ok(Math.abs(Number(v.duration)-.5)<=1/30+.0001);assert.ok(Number(a.duration)>=1.49);assert.ok(Number(output.format.duration)>=1.49);record.actualVideoShortDespiteAudioAndContainer=true;
      }else{assert.equal(done.state.status,'completed');assert.equal(done.state.publicationStarted,true);assert.ok((await fs.stat(destination)).size>0);}
      assert.deepEqual(await host.snapshot.state(snapshot.snapshotId),snapshot);await host.snapshot.release(snapshot.snapshotId,++op);
      const names=await fs.readdir(path.join(work,'outputs'));assert.ok(!names.some(n=>n.startsWith('.tmp')),'failed/completed workers must clean temporary outputs');
    }
    result.finalSource=await host.source.state();assert.deepEqual(result.finalSource.stamp,source.stamp);assert.equal(result.finalSource.paused,true);assert.equal(result.finalSource.position,source.position);result.outputs=await fs.readdir(path.join(work,'outputs'));assert.deepEqual(result.outputs.sort(),['processed-valid.mp4','raw-valid.mp4']);result.pass=true;await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify(result,null,2));console.error('INDEPENDENT_AUDIO_TAIL_DONE');
  }catch(error){result.error=String(error);result.stack=error.stack;await fs.writeFile(path.join(work,'consumer-result.json'),JSON.stringify(result,null,2));throw error;}
};
