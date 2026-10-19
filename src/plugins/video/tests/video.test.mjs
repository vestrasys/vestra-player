import test from 'node:test';
import assert from 'node:assert/strict';
import {defaults,nodesFor,Latest} from '../model.ts';
import {activate} from '../main.ts';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
const flush=async()=>{for(let i=0;i<15;i++)await tick();};
function fixture(phase='ready'){
 const state={source:{generation:4,phase},video:[],backendUncertain:{}};
 const calls={replaced:[],commands:[],shown:[],sets:[],opened:[]};
 const listeners={};
 const host={
  processing:{state:async()=>state,replace:async change=>{calls.replaced.push(change);return {status:'applied',stage:'video',revision:change.revision,generation:change.expectedGeneration};},operation:async()=>({status:'unknown'})},
  source:{state:async()=>({paused:false})},
  commands:{register:async(id,revision,declaration)=>{calls.commands.push({id,revision,declaration});},onExecute:fn=>{listeners.command=fn;return()=>{};}},
  events:{subscribe:async(event,fn)=>{listeners.source=fn;return async()=>{};}},
  ui:{open:async(...args)=>{calls.opened.push(args);},set:async(...args)=>{calls.sets.push(args);},show:async(...args)=>{calls.shown.push(args);},close:async()=>{},onCallback:fn=>{listeners.ui=fn;return()=>{};}}
 };
 return {host,state,calls,listeners,emit:(callback,...args)=>listeners.ui({view:'picture',callback,args})};
}
test('neutral and blur use a single typed ordered chain',()=>{
 assert.deepEqual(nodesFor(defaults),[{kind:'eq',brightness:0,contrast:1,saturation:1,gamma:1}]);
 assert.deepEqual(nodesFor({...defaults,blurEnabled:true,blurStrength:4})[1],{kind:'gblur',sigma:4,steps:3});
 assert.equal(nodesFor({...defaults,blurEnabled:true,blurStrength:0}).length,1);
});
test('latest queue keeps only one in flight and latest pending through error',async()=>{
 let release;const gate=new Promise(resolve=>{release=resolve;});const applied=[],errors=[];
 const q=new Latest(async value=>{applied.push(value);if(value===1){await gate;throw Error('unsupported');}},e=>errors.push(e.message));
 q.submit(1);q.submit(2);q.submit(3);assert.deepEqual(applied,[1]);release();await flush();
 assert.deepEqual(applied,[1,3]);assert.deepEqual(errors,['unsupported']);q.stop();q.submit(4);await flush();assert.deepEqual(applied,[1,3]);
});
test('canonical menu entrances reuse a tool; hide retains effect; disable clears only own nodes',async()=>{
 const f=fixture();const stop=await activate(f.host);await flush();
 assert.deepEqual(f.calls.commands.map(c=>c.declaration.path),[['视频','画面调节…'],['视频','高斯模糊…']]);
 f.listeners.command({id:'video.picture'});await flush();f.listeners.command({id:'video.gaussian'});await flush();assert.equal(f.calls.opened.length,1);
 f.emit('adjust','brightness',40);await flush();assert.equal(f.calls.replaced.at(-1).nodes[0].brightness,.4);
 f.emit('toggle-blur',true);await flush();assert.equal(f.calls.replaced.at(-1).nodes[1].kind,'gblur');
 const before=f.calls.replaced.length;f.emit('hide-tool');await flush();assert.equal(f.calls.replaced.length,before);
 f.emit('toggle-effect',false);await flush();assert.deepEqual(f.calls.replaced.at(-1).nodes,[]);assert.equal(f.calls.replaced.at(-1).resourceId,'picture');
 assert.ok(f.calls.replaced.every(c=>c.expectedGeneration===4));await stop();
});
test('empty media retains edits and ready transition applies the latest desired values',async()=>{
 const f=fixture('empty');const stop=await activate(f.host);f.emit('adjust','contrast',35);await flush();assert.equal(f.calls.replaced.length,0);
 f.state.source.phase='ready';f.state.source.generation=5;f.listeners.source();await flush();
 assert.equal(f.calls.replaced.at(-1).nodes[0].contrast,2**.35);assert.equal(f.calls.replaced.at(-1).expectedGeneration,5);await stop();
});
test('unknown timeout blocks new tokens until the original result resolves',async()=>{
 const f=fixture();const stop=await activate(f.host);await flush();
 f.host.processing.replace=async change=>{f.calls.replaced.push(change);throw Error('timeout');};
 f.emit('adjust','gamma',20);await flush();const count=f.calls.replaced.length;
 f.emit('adjust','gamma',30);await flush();assert.equal(f.calls.replaced.length,count);
 f.emit('retry');await flush();assert.equal(f.calls.replaced.length,count);
 f.host.processing.operation=async()=>({status:'completed'});
 f.host.processing.replace=async change=>{f.calls.replaced.push(change);};f.emit('retry');await flush();
 assert.equal(f.calls.replaced.length,count+1);assert.equal(f.calls.replaced.at(-1).nodes[0].gamma,2**.3);await stop();
});
