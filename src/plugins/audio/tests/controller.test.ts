import test from 'node:test';
import assert from 'node:assert/strict';
import type { Host } from '../../../sdk/index.ts';
import { AudioController } from '../controller.ts';
import { neutral } from '../model.ts';
type Source = Awaited<ReturnType<Host['source']['state']>>;
type Change = Parameters<Host['processing']['replace']>[0];
const source = (generation=1, track=1, eventSequence=1) => ({stamp:{generation,intentVersion:1},phase:'ready',eventSequence,audioTracks:[{trackId:track,selected:true}]}) as Source;
const tick = () => new Promise(resolve=>setImmediate(resolve));
function rig() {
 const calls: Change[]=[];
 const operations: unknown[]=[];
 const pending: {resolve:()=>void;reject:()=>void}[]=[];
 let outcome: unknown={status:'pending'};
 const host={processing:{replace:(change:Change)=>{calls.push(change);return new Promise((resolve,reject)=>pending.push({resolve:()=>resolve({status:'applied',stage:'audio',revision:change.revision,generation:change.expectedGeneration}),reject:()=>reject(new Error('transport timeout'))}));},operation:async(token:unknown)=>{operations.push(token);return outcome;}}} as unknown as Pick<Host,'processing'>;
 const controller=new AudioController(host,()=>{});controller.source(source());
 return {controller,calls,pending,operations,outcome:(value:unknown)=>{outcome=value;}};
}
test('one in-flight update retains only the newest queued settings',async()=>{
 const r=rig();for(const delayMs of [100,200,300])r.controller.edit({...neutral(),delayMs});
 assert.equal(r.calls.length,1);r.pending[0]!.resolve();await tick();
 assert.equal(r.calls.length,2);assert.equal(r.calls[1]!.offsetMs,300);assert.equal(r.calls[1]!.revision,2);
 r.pending[1]!.resolve();await tick();assert.equal(r.controller.busy,false);
});
test('source generation change queues neutral settings and ignores old success status',async()=>{
 const r=rig();r.controller.edit({...neutral(),delayMs:500});r.controller.source(source(2,2,2));
 assert.equal(r.controller.settings.delayMs,0);r.pending[0]!.resolve();await tick();
 assert.equal(r.calls[1]!.expectedGeneration,2);assert.equal(r.calls[1]!.offsetMs,0);assert.deepEqual(r.calls[1]!.nodes,[]);
});
test('same-generation track switch ignores old in-flight definitive failure and applies neutral',async()=>{
 const r=rig();r.controller.edit({...neutral(),delayMs:500});r.controller.edit({...neutral(),delayMs:900});
 r.controller.source(source(1,2,2));r.outcome({status:'failed',error:{code:'backend',message:'old-track failure'}});
 r.pending[0]!.reject();await tick();
 assert.equal(r.controller.failed,false);assert.equal(r.calls.length,2);
 assert.equal(r.calls[1]!.expectedGeneration,1);assert.equal(r.calls[1]!.offsetMs,0);assert.deepEqual(r.calls[1]!.nodes,[]);
 r.pending[1]!.resolve();await tick();assert.equal(r.controller.status,'已应用');assert.equal(r.controller.settings.delayMs,0);
});
test('same-generation track switch while operation lookup is pending ignores its later failure',async()=>{
 const calls:Change[]=[];let fail:()=>void=()=>{};let settle:(value:unknown)=>void=()=>{};
 const lookup=new Promise(resolve=>{settle=resolve;});
 const host={processing:{replace:(change:Change)=>{calls.push(change);return calls.length===1?new Promise((_,reject)=>{fail=()=>reject(new Error('lost reply'));}):Promise.resolve({status:'applied'});},operation:()=>lookup}} as unknown as Pick<Host,'processing'>;
 const controller=new AudioController(host,()=>{});controller.source(source());controller.edit({...neutral(),delayMs:400});
 fail();await tick();controller.source(source(1,2,2));settle({status:'failed',error:{code:'backend',message:'obsolete'}});await tick();
 assert.equal(controller.failed,false);assert.equal(calls.length,2);assert.equal(calls[1]!.offsetMs,0);
});
test('unknown outcome blocks submissions and checks the original fixed token',async()=>{
 const r=rig();r.controller.edit({...neutral(),delayMs:100});r.pending[0]!.reject();await tick();
 assert.equal(r.controller.failed,true);r.controller.edit({...neutral(),delayMs:900});assert.equal(r.controller.failed,true);
 await r.controller.retry();assert.equal(r.calls.length,1);assert.deepEqual(r.operations[1],r.calls[0]);
 r.outcome({status:'completed',result:{status:'applied'}});void r.controller.retry();await tick();
 assert.equal(r.calls.length,2);assert.equal(r.calls[1]!.offsetMs,900);
});
test('old unknown operation failure after track change unblocks current neutral without reviving old edits',async()=>{
 const r=rig();r.controller.edit({...neutral(),delayMs:700});r.pending[0]!.reject();await tick();r.controller.source(source(1,2,2));
 r.outcome({status:'failed',error:{code:'backend',message:'old'}});void r.controller.retry();await tick();
 assert.equal(r.controller.failed,false);assert.equal(r.calls.length,2);assert.equal(r.calls[1]!.offsetMs,0);
});
test('stale event sequences cannot restore an older source',()=>{
 const r=rig();r.controller.source(source(3,1,20));r.controller.source(source(2,1,19));r.controller.edit({...neutral(),delayMs:100});assert.equal(r.calls[0]!.expectedGeneration,3);
});
