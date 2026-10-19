/** Prepared public MP4 video-tail test. No host/tool execution without --run and an explicit final freeze. */
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp,mkdir,copyFile,readFile,writeFile,readdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';
const fixtureSha='848339a163c67cca4d5900591365122b219ab6b8bb07663272926f5b41a54545';
const digest=b=>createHash('sha256').update(b).digest('hex');

async function consumer(host){
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
}

if(process.argv.includes('--run')){
  const root=resolve(import.meta.dirname,'../../../..'),fixture=resolve(import.meta.dirname,'../audio-tail-preparation/source-video2-audio4.mkv'),bytes=await readFile(fixture);assert.equal(digest(bytes),fixtureSha);
  const work=await mkdtemp(resolve(tmpdir(),'vestra-independent-audio-tail-'));await mkdir(resolve(work,'consumer'));await mkdir(resolve(work,'outputs'));await mkdir(resolve(work,'analysis'));await copyFile(fixture,resolve(work,'source-video2-audio4.mkv'));await copyFile(import.meta.filename,resolve(work,'executed-review.mjs'));
  await writeFile(resolve(work,'consumer/manifest.json'),JSON.stringify({id:'independent.audio-tail',apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['events','playback.read','playback.control','media.read','media.control']}));await writeFile(resolve(work,'consumer/main.mjs'),'export const activate='+consumer.toString()+';\n');
  const binary=resolve(root,'src/host/target/debug/vestra-host'),args=['--headless','--silent','--seconds','12','--plugin',resolve(work,'consumer')],host=spawnSync(binary,args,{encoding:'utf8',timeout:22000,env:{...process.env,VESTRA_AUDIO_TAIL_REVIEW:work}});await writeFile(resolve(work,'host.stdout.log'),host.stdout??'');await writeFile(resolve(work,'host.stderr.log'),host.stderr??'');await writeFile(resolve(work,'command.json'),JSON.stringify({binary,args,fixture,fixtureSha,binarySha256:digest(await readFile(binary)),work,exitCode:host.status,signal:host.signal,error:host.error?.message},null,2));console.log(JSON.stringify({work,hostExitCode:host.status}));
  const tools=[],analysis={};
  async function run(binary,args,name,encoding='utf8',maxBuffer=8*1024*1024){const out=spawnSync(binary,args,{encoding,maxBuffer});tools.push({binary,args,name,exitCode:out.status,signal:out.signal,stderr:out.stderr?.toString()});await writeFile(resolve(work,'tool-commands.json'),JSON.stringify(tools,null,2));await writeFile(resolve(work,'analysis',name+'.stderr.log'),out.stderr??'');assert.equal(out.status,0,out.stderr?.toString());return out.stdout;}
  try{
    assert.equal(host.status,0);assert.match(host.stderr,/INDEPENDENT_AUDIO_TAIL_DONE/);const result=JSON.parse(await readFile(resolve(work,'consumer-result.json')));assert.equal(result.pass,true);
    const ffmpeg='/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg',ffprobe='/opt/homebrew/opt/ffmpeg-full/bin/ffprobe';
    for(const record of result.jobs.filter(x=>!x.invalidTail)){
      const file=record.request.destination,name=record.name,probe=JSON.parse(await run(ffprobe,['-v','error','-show_streams','-show_format','-count_frames','-of','json',file],name+'-probe'));await writeFile(resolve(work,'analysis',name+'-probe.json'),JSON.stringify(probe,null,2));const video=probe.streams.find(s=>s.codec_type==='video'),audio=probe.streams.find(s=>s.codec_type==='audio');assert.equal(probe.streams.length,2);assert.equal(video.codec_name,'h264');assert.equal(video.width,320);assert.equal(video.height,180);assert.equal(+video.nb_read_frames,30);assert.equal(audio.codec_name,'aac');assert.equal(+audio.sample_rate,48000);assert.equal(audio.channels,1);
      const frames=JSON.parse(await run(ffprobe,['-v','error','-select_streams','v:0','-show_frames','-of','json',file],name+'-frames'));await writeFile(resolve(work,'analysis',name+'-frames.json'),JSON.stringify(frames,null,2));assert.equal(frames.frames.length,30);const pts=frames.frames.map(f=>+f.best_effort_timestamp_time);assert.equal(pts[0],0);assert.ok(Math.abs(pts.at(-1)-29/30)<.00001);for(let i=1;i<pts.length;i++)assert.ok(pts[i]>pts[i-1]);
      await run(ffmpeg,['-nostdin','-v','error','-i',file,'-map','0:v:0','-map','0:a:0','-f','null','-'],name+'-full-decode');
      const pcm=await run(ffmpeg,['-nostdin','-v','error','-i',file,'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-'],name+'-audio',null);assert.equal(pcm.length%4,0);const samples=pcm.length/4;assert.ok(samples>=48000&&samples<=48000+1024,'one second finite AAC output with at most one AAC frame trailing pad');for(let offset=0;offset<pcm.length;offset+=4)assert.ok(Number.isFinite(pcm.readFloatLE(offset)));await writeFile(resolve(work,'analysis',name+'.f32le'),pcm);
      analysis[name]={probe,videoFrames:30,firstVideoPts:pts[0],lastVideoPts:pts.at(-1),decodedMonoSamples:samples,decodedAudioSeconds:samples/48000,sha256:digest(await readFile(file)),fullDecodeExitCode:0};
    }
    const files=await readdir(resolve(work,'outputs'));assert.ok(!files.some(n=>n.startsWith('.tmp')||n.includes('video-short')));const summary={pass:true,hostRerun:false,fixtureSha,failedJobs:result.jobs.filter(x=>x.invalidTail),analysis,limits:['Finite source with actual two-second video and four-second mono audio; no assertion based on container duration alone.','Negative worker facts show actual audio/container coverage while video is short; final failure and no-publication observed.','Valid files decoded completely; this is not acoustic output, arbitrary timestamp/codec coverage or full M3 acceptance.']};await writeFile(resolve(work,'result.json'),JSON.stringify(summary,null,2));console.log(JSON.stringify({pass:true,work,analysis}));
  }catch(error){await writeFile(resolve(work,'review-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,analysis},null,2));throw error;}
}else console.log('Prepared only. Explicit maintainer source/SDK/binary freeze required before --run.');
