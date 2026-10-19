/** Independent real-host subtitle consumer. Execute ONLY after explicit host freeze.
 * Public source/selection/upload facts; not a subtitle pixel/style/device test.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

async function consumer(host) {
  const assert = (await import('node:assert/strict')).default;
  const fs = await import('node:fs/promises'), path = await import('node:path');
  const { createHash } = await import('node:crypto');
  const work = process.env.VESTRA_SUBTITLE_REVIEW;
  const s = host.subtitles, checks = {}, events = [];
  const wait = ms => new Promise(r => setTimeout(r, ms));
  const digest = b => createHash('sha256').update(b).digest('hex');
  const small = text => Buffer.from(`1\n00:00:00,000 --> 00:00:15,000\n${text}\n\n`);
  let uploadId = 0, operationId = 0;
  async function rejected(promise, code) {
    try { await promise; throw Error('expected rejection '+code); }
    catch(e) { assert.equal(e.code, code, String(e)); return {code:e.code,message:e.message}; }
  }
  async function ready(generation) {
    const end = Date.now()+5000;
    while(Date.now()<end) { const state = await host.source.state(); if(state.phase==='ready'&&state.stamp.generation===generation) return state; await wait(20); }
    throw Error('source did not become ready');
  }
  async function upload(bytes, generation, repeat = false) {
    const id = ++uploadId;
    const begun = await s.begin(id,generation,bytes.length);
    assert.deepEqual(await s.begin(id,generation,bytes.length),begun);
    if(repeat) await rejected(s.begin(id,generation,bytes.length+1),'conflict');
    for(let offset=0;offset<bytes.length;offset+=65536) {
      const chunk=bytes.subarray(offset,offset+65536), encoded=chunk.toString('base64');
      const status=await s.append(id,offset,encoded);
      assert.equal(status.receivedBytes,offset+chunk.length);
      if(repeat&&offset===0) {
        assert.deepEqual(await s.append(id,offset,encoded),status);
        const changed=Buffer.from(chunk); changed[0]^=1;
        await rejected(s.append(id,offset,changed.toString('base64')),'conflict');
        assert.deepEqual(await s.uploadState(id),status);
      }
    }
    const status=await s.uploadState(id);
    assert.equal(status.receivedBytes,bytes.length);
    return id;
  }
  async function commit(bytes,resourceId,revision,options={}) {
    const state=await s.state(), id=await upload(bytes,state.source.generation);
    const payload={operationId:++operationId,uploadId:id,sha256:digest(bytes),resourceId,revision,expected:state.selection.stamp,...options};
    const result=await s.commit(payload); await s.discard(id);
    return {payload,result};
  }
  async function preserveFailure(payload,code) {
    const before=await s.state(), error=await rejected(s.commit(payload),code);
    assert.deepEqual(await s.state(),before);
    assert.deepEqual(await s.operation(payload.operationId),{status:'failed',error});
    assert.deepEqual(await rejected(s.commit(payload),code),error);
    assert.deepEqual(await s.state(),before);
    return error;
  }
  async function helpers(count) {
    const end=Date.now()+5000;
    while(Date.now()<end) {
      try { const rows=(await fs.readFile(path.join(work,'helpers.jsonl'),'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse);
        const state=await s.state();
        if(rows.length===count&&state.resources.some(r=>r.owner===rows.at(-1).owner&&r.resourceId==='shared')) return {rows,state};
      } catch(e) { if(e.code!=='ENOENT') throw e; }
      await wait(20);
    }
    throw Error('helper evidence/public resource missing');
  }
  try {
    for(const event of ['source.state','playback.backend','host.error']) await host.events.subscribe(event,data=>events.push({event,data}));
    const opened=await host.source.submit({kind:'open',source:path.join(work,'source.mp4'),startPaused:true});
    await ready(opened.ticket.stamp.generation);
    const original=await commit(small('ORIGINAL <literal> {\\b1}'),'shared',1);
    assert.equal(original.result.confirmation,'applied');
    const owner=original.result.selection.target.owner;
    assert.equal(original.result.selection.target.contentRevision,1);
    assert.deepEqual(await s.operation(original.payload.operationId),{status:'completed',result:original.result});
    assert.deepEqual(await s.commit(original.payload),original.result);
    await rejected(s.commit({...original.payload,resourceId:'different'}),'conflict');
    checks.atomicOriginalAndReplay=original;

    // >1MiB raw SRT (thus larger than one permitted JSONL), <=2MiB, <=4096 chars/cue.
    const large=Buffer.from(Array.from({length:1500},(_,i)=>`${i+1}\n00:00:00,000 --> 00:00:15,000\n${'L'.repeat(900)} ${i}\n\n`).join(''));
    assert.ok(large.length>1048576&&large.length<=2097152);
    const g=original.result.source.generation, largeId=await upload(large,g,true);
    const largePayload={operationId:++operationId,uploadId:largeId,sha256:digest(large),resourceId:'large',revision:1,expected:(await s.state()).selection.stamp};
    const largeState=await s.commit(largePayload), resource=largeState.resources.find(r=>r.owner===owner&&r.resourceId==='large');
    assert.equal(resource.sizeBytes,large.length); assert.equal(resource.digest,digest(large)); assert.equal(resource.cues,1500);
    assert.equal(resource.derived.format,'webvtt'); assert.match(resource.derived.digest,/^[0-9a-f]{64}$/);
    await s.discard(largeId);
    checks.largeChunked={bytes:large.length,chunks:Math.ceil(large.length/65536),resource};

    // A newer explicit off must win over stale selection CAS.
    const oldStamp=largeState.selection.stamp;
    const off=await s.select(null,oldStamp,++operationId);
    const staleOperation=++operationId, staleError=await rejected(s.select(original.result.selection.target,oldStamp,staleOperation),'stale');
    assert.deepEqual(await s.operation(staleOperation),{status:'failed',error:staleError});
    assert.deepEqual(await s.state(),off);
    await s.select(original.result.selection.target,off.selection.stamp,++operationId);
    checks.selectionCAS={oldStamp,off,staleError};

    // Upload/validation failures cannot partially replace an existing selection/resource.
    const good=small('GOOD');
    for(const kind of ['badSrt','badDigest','truncated']) {
      const before=await s.state(), bytes=kind==='badSrt'?Buffer.from('not valid SRT\n'):good;
      const id=++uploadId; await s.begin(id,g,bytes.length+(kind==='truncated'?1:0));
      await s.append(id,0,bytes.toString('base64'));
      const payload={operationId:++operationId,uploadId:id,sha256:kind==='badDigest'?'0'.repeat(64):digest(bytes),resourceId:'shared',revision:2,expected:before.selection.stamp};
      const error=await preserveFailure(payload,kind==='truncated'?'conflict':'invalid');
      await s.discard(id); checks[kind]={error};
    }

    // Translation origin guard is independently stale even with a FRESH selection CAS.
    const origin=original.result.selection.target;
    const updated=await commit(small('NEW ORIGINAL'),'shared',2);
    assert.equal(updated.result.selection.target.contentRevision,2);
    const translation=small('TRANSLATION'), tid=await upload(translation,g);
    const originPayload={operationId:++operationId,uploadId:tid,sha256:digest(translation),resourceId:'translation',revision:1,expected:(await s.state()).selection.stamp,inputOrigin:origin};
    const originError=await preserveFailure(originPayload,'stale'); await s.discard(tid);
    checks.originalRevisionCAS={origin,current:updated.result.selection.target,error:originError};

    // A source switch invalidates an in-flight upload and old CAS, without mock events.
    const pendingId=await upload(small('LATE OLD SOURCE'),g), old=await s.state();
    const next=await host.source.submit({kind:'open',source:path.join(work,'source.mp4'),startPaused:true});
    await ready(next.ticket.stamp.generation);
    const fresh=await s.state(); assert.ok(fresh.source.generation>g); assert.deepEqual(fresh.resources,[]);
    const sourcePayload={operationId:++operationId,uploadId:pendingId,sha256:digest(small('LATE OLD SOURCE')),resourceId:'late',revision:1,expected:old.selection.stamp};
    const sourceError=await rejected(s.commit(sourcePayload),'stale');
    assert.deepEqual(await s.state(),fresh);
    assert.deepEqual(await s.operation(sourcePayload.operationId),{status:'failed',error:sourceError});
    checks.sourceCAS={oldGeneration:g,newGeneration:fresh.source.generation,error:sourceError};

    const baseline=await commit(small('OWNER BASELINE'),'shared',1);
    const helperDir=path.join(work,'helper'); await host.plugins.load(helperDir);
    const one=await helpers(1); assert.notEqual(one.rows[0].owner,owner);
    assert.equal(one.state.resources.filter(r=>r.resourceId==='shared').length,2);
    await host.plugins.disable('review.subtitle-helper');
    const restored=await s.state(); assert.deepEqual(restored.selection.target,baseline.result.selection.target);
    assert.ok(!restored.resources.some(r=>r.owner===one.rows[0].owner));
    await host.plugins.load(helperDir); const two=await helpers(2);
    assert.notEqual(two.rows[1].owner,one.rows[0].owner);
    assert.deepEqual(two.rows[1].prior,{status:'unknown'});
    const explicitlyOff=await s.select(null,two.state.selection.stamp,++operationId);
    await host.plugins.disable('review.subtitle-helper');
    const final=await s.state(); assert.equal(final.selection.target,null);
    assert.deepEqual(final.selection.stamp,explicitlyOff.selection.stamp);
    assert.ok(!final.resources.some(r=>r.owner===two.rows[1].owner));
    checks.ownerIsolation={rows:two.rows,restored,final};
    // Close keeps the source generation but invalidates native subtitle IDs.
    // A later owner cleanup must remain safe without an active native load.
    await host.plugins.load(helperDir); const three=await helpers(3);
    const closingSource=await host.source.state(), closeMarker=events.length;
    await host.source.submit({kind:'close',expected:closingSource.stamp});
    const closeDeadline=Date.now()+4000;
    let emptySource;
    while(Date.now()<closeDeadline) {
      emptySource=await host.source.state(); if(emptySource.phase==='empty') break; await wait(20);
    }
    assert.equal(emptySource.phase,'empty');
    assert.equal(emptySource.stamp.generation,closingSource.stamp.generation);
    const emptySubtitles=await s.state();
    await host.plugins.disable('review.subtitle-helper');
    const afterCloseCleanup=await s.state(), sourceAfterCleanup=await host.source.state();
    assert.equal(afterCloseCleanup.source.phase,'empty');
    assert.equal(afterCloseCleanup.source.generation,closingSource.stamp.generation);
    assert.notEqual(afterCloseCleanup.confirmation,'uncertain');
    assert.equal(afterCloseCleanup.error,null);
    assert.ok(!afterCloseCleanup.resources.some(r=>r.owner===three.rows[2].owner));
    assert.notEqual(afterCloseCleanup.selection.target?.owner,three.rows[2].owner);
    assert.equal(sourceAfterCleanup.phase,'empty');
    assert.deepEqual(sourceAfterCleanup.stamp,emptySource.stamp);
    assert.ok(!events.slice(closeMarker).some(e=>e.event==='host.error'&&/subtitle/i.test(JSON.stringify(e.data))), 'close/owner cleanup must not report subtitle backend failure');
    checks.closedSourceOwnerCleanup={owner:three.rows[2].owner,emptySource,emptySubtitles,afterCloseCleanup,sourceAfterCleanup};
    await fs.writeFile(path.join(work,'result.json'),JSON.stringify({pass:true,checks,events,limits:['Real public upload/selection/source/owner facts, no mock media callbacks.','No glyph pixels, style/font confirmation, native/device audio, backend-failure injection, pending/terminal saturation or all global memory limits proved.','Source switch and origin revision races are controlled serial stale-work cases, not proof of every scheduler interleaving.']},null,2));
    console.error('SUBTITLE_REVIEW_PASS');
  } catch(error) { await fs.writeFile(path.join(work,'failure.json'),JSON.stringify({pass:false,error:String(error),stack:error.stack,checks,events},null,2)); throw error; }
}
async function helper(host) {
  const fs=await import('node:fs/promises'), path=await import('node:path'), {createHash}=await import('node:crypto');
  const s=host.subtitles, prior=await s.operation(1), state=await s.state();
  const bytes=Buffer.from('1\n00:00:00,000 --> 00:00:15,000\nHELPER\n\n');
  await s.begin(1,state.source.generation,bytes.length); await s.append(1,0,bytes.toString('base64'));
  const result=await s.commit({operationId:1,uploadId:1,sha256:createHash('sha256').update(bytes).digest('hex'),resourceId:'shared',revision:1,expected:state.selection.stamp});
  const query=await s.operation(1);
  await fs.appendFile(path.join(process.env.VESTRA_SUBTITLE_REVIEW,'helpers.jsonl'),JSON.stringify({owner:result.selection.target.owner,prior,result,query})+'\n');
}

const work=await mkdtemp(resolve(tmpdir(),'vestra-subtitle-review-'));
const made=spawnSync('ffmpeg',['-v','error','-f','lavfi','-i','testsrc2=size=320x180:rate=20','-t','24','-c:v','libx264','-preset','ultrafast',resolve(work,'source.mp4')],{encoding:'utf8'});
assert.equal(made.status,0,made.stderr);
for(const [name,activate] of [['consumer',consumer],['helper',helper]]) {
  const directory=resolve(work,name); await mkdir(directory);
  await writeFile(resolve(directory,'manifest.json'),JSON.stringify({id:`review.subtitle-${name}`,apiVersion:'vestra.experimental/0.1',entry:'main.mjs',capabilities:['subtitles.read','subtitles.control','playback.read',...(name==='consumer'?['playback.control','events','plugins.manage']:[])]}));
  await writeFile(resolve(directory,'main.mjs'),`export const activate=${activate.toString()};\n`);
}
const args=['--headless','--silent','--seconds','22','--plugin',resolve(work,'consumer')];
const child=spawnSync(resolve(import.meta.dirname,'../host/target/debug/vestra-host'),args,{encoding:'utf8',timeout:32000,env:{...process.env,VESTRA_SUBTITLE_REVIEW:work}});
await writeFile(resolve(work,'host.stdout.log'),child.stdout??''); await writeFile(resolve(work,'host.stderr.log'),child.stderr??'');
await writeFile(resolve(work,'command.json'),JSON.stringify({work,args,exitCode:child.status,signal:child.signal},null,2));
console.log(JSON.stringify({work,exitCode:child.status})); assert.equal(child.status,0,child.stderr); assert.match(child.stderr,/SUBTITLE_REVIEW_PASS/);
assert.equal(JSON.parse(await readFile(resolve(work,'result.json'),'utf8')).pass,true);
const rows=(await readFile(resolve(work,'helpers.jsonl'),'utf8')).trim().split('\n').map(JSON.parse);
assert.equal(rows.length,3); assert.equal(new Set(rows.map(row=>row.owner)).size,3);
for(const row of rows) {assert.deepEqual(row.prior,{status:'unknown'});assert.deepEqual(row.query,{status:'completed',result:row.result});}
await writeFile(resolve(work,'external-review.json'),JSON.stringify({pass:true,rows},null,2));
