export const activate=async function activate(host){
 const assert=(await import('node:assert/strict')).default,fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto');
 const work=process.env.VESTRA_SUBTITLE_STYLE_REVIEW,mode=process.env.VESTRA_SUBTITLE_STYLE_MODE,dir=path.join(work,mode),s=host.subtitles;
 const phases=[],events=[],checks={};let op=0,revision=0;
 const wait=ms=>new Promise(r=>setTimeout(r,ms));
 async function records(){try{return(await fs.readFile(path.join(dir,'frames','frames.jsonl'),'utf8')).split('\n').slice(0,-1).filter(Boolean).map(JSON.parse);}catch(e){if(e.code==='ENOENT')return[];throw e;}}
 async function ready(generation,cycle=0){const end=Date.now()+4500;while(Date.now()<end){const state=await host.source.state();if(state.phase==='ready'&&state.stamp.generation===generation&&state.cycle>=cycle)return state;await wait(20);}throw Error('ready timeout');}
 async function stable(name){await wait(350);const first=(await records()).length;await wait(500);const last=(await records()).length;assert.ok(last-first>=2,'bounded samples '+name);const source=await host.source.state();assert.equal(source.paused,true);assert.equal(source.phase,'ready');phases.push({name,first,last,source,subtitles:await s.state()});}
 async function apply(style){const state=await s.state(),payload={operationId:++op,revision:++revision,expected:state.selection.stamp,style};const result=await s.style(payload);assert.deepEqual(await s.style(payload),result);assert.deepEqual(await s.operation(op),{status:'completed',result});assert.equal(result.presentation.actualFont,null);assert.equal(result.presentation.snapshotEligible,false);return result;}
 async function rejection(promise,code){try{await promise;throw Error('expected '+code);}catch(e){assert.equal(e.code,code,String(e));return{code:e.code,message:e.message};}}
 try{
  for(const event of ['source.state','playback.backend','host.error'])await host.events.subscribe(event,data=>events.push({event,data}));
  const open=await host.source.submit({kind:'open',source:path.join(work,'a.mkv'),startPaused:true});await ready(open.ticket.stamp.generation);
  await stable('gray');
  const initial=await s.state(),bytes=Buffer.from('1\n00:00:00,000 --> 00:00:15,000\nSTYLE SAMPLE\n\n');
  await s.begin(1,initial.source.generation,bytes.length);await s.append(1,0,bytes.toString('base64'));
  await s.commit({operationId:++op,uploadId:1,sha256:createHash('sha256').update(bytes).digest('hex'),resourceId:'sample',revision:1,expected:initial.selection.stamp});
  const font=(await s.state()).presentation.fonts[0];assert.ok(font);assert.equal(font.resolution,'requestedFamily');assert.equal(font.snapshotEligible,false);
  const yellow={fontId:font.id,fontSize:72,color:'yellow',outline:2,marginPercent:5,offsetMs:0};
  if(mode==='main'){
   await stable('default');
   await apply({...yellow,fontSize:16,color:'white',outline:0});await stable('small-white');
   await apply(yellow);await stable('large-yellow');
   const before=await s.state();
   await rejection(s.style({operationId:++op,revision:revision+1,expected:before.selection.stamp,style:{...yellow,fontSize:73}}),'invalid');
   await rejection(s.style({operationId:++op,revision:revision+1,expected:before.selection.stamp,style:{...yellow,fontId:'review-missing-font'}}),'unsupported');
   const staleOp=++op,stalePayload={operationId:staleOp,revision:revision+1,expected:{...before.selection.stamp,revision:before.selection.stamp.revision+100},style:yellow};
   const error=await rejection(s.style(stalePayload),'stale');assert.deepEqual(await s.operation(staleOp),{status:'failed',error});
   assert.deepEqual(await s.state(),before);checks.rejectedPreservePublic=before;await stable('failed-keeps-yellow');
   await apply({...yellow,offsetMs:3000});await stable('delayed-hidden');
   await apply(yellow);await stable('delay-restored');
   await host.plugins.load(path.join(work,'helper'));
   const deadline=Date.now()+4000;let helper;
   while(Date.now()<deadline){try{helper=JSON.parse(await fs.readFile(path.join(work,'helper-result.json'),'utf8'));break;}catch(e){if(e.code!=='ENOENT')throw e;}await wait(20);}
   assert.ok(helper);assert.equal((await s.state()).presentation.effective.manifestId,'review.subtitle-style-z');await stable('z-cyan');
   await apply({...yellow,fontSize:16,color:'white',outline:0});assert.equal((await s.state()).presentation.effective.manifestId,'review.subtitle-style-z');await stable('a-late-still-cyan');
   await host.plugins.disable('review.subtitle-style-z');const after=await s.state();assert.equal(after.presentation.effective.manifestId,'review.subtitle-style-a');await stable('z-disabled-small-white');
   await apply(null);assert.equal((await s.state()).presentation.effective,null);await stable('revoked-default');checks.helper=helper;
  }else{
   await apply(yellow);await stable('a-yellow');
   const before=await host.source.state(),marker=events.length;
   await host.source.submit({kind:'restart',expected:before.stamp});const restarted=await ready(before.stamp.generation,before.cycle+1);
   // Public restart deliberately starts playback; pause once for settled evidence.
   // This is not a subtitle mutation or a claim of paused continuity across restart.
   await host.source.submit({kind:'control',control:{kind:'pause',paused:true},expected:restarted.stamp});
   // No subtitle commit/select/style after restart; test actual Ready replay alone.
   await stable('restart-yellow');checks.restartEvents=events.slice(marker);
   const b=await host.source.submit({kind:'open',source:path.join(work,'b.mkv'),startPaused:true});await ready(b.ticket.stamp.generation);
   // B contains a default embedded SRT; no subtitle mutation after Ready.
   const bstate=await s.state();assert.equal(bstate.selection.target?.kind,'embedded','B Ready must select actual embedded subtitle without new mutation');
   assert.equal(bstate.presentation.effective.style.color,'yellow');await stable('b-ready-yellow');checks.bReady=bstate;
  }
  const first=phases[0].source;
  for(const phase of phases){if(mode==='main'){assert.ok(Math.abs(phase.source.position-first.position)<.001);assert.deepEqual(phase.source.stamp,first.stamp);assert.equal(phase.source.cycle,first.cycle);}}
  await fs.writeFile(path.join(dir,'consumer-result.json'),JSON.stringify({pass:true,phases,events,checks},null,2));console.error('SUBTITLE_STYLE_REVIEW_DONE '+mode);
 }catch(error){await fs.writeFile(path.join(dir,'consumer-failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,phases,events,checks},null,2));throw error;}
};
