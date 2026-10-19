import type { Host, Json } from '../../sdk/index.ts';
import type { ProcessingChange } from '../../sdk/processing.ts';
import { defaults,presets,nodesFor,Latest,clamp } from './model.ts';
import type { Picture } from './model.ts';
const resourceId='picture',view='picture';
export async function activate(host:Host){
 let picture={...defaults},enabled=true,opened=false,disposed=false,revision=0,lastSource='',blocked=false;
 let unresolved:ProcessingChange|undefined;
 let status='等待媒体';
 const text=(e:unknown)=>e instanceof Error?e.message:String(e);
 async function set(key:string,value:Json){if(opened&&!disposed)await host.ui.set(view,key,value);}
 function message(value:string){status=value;void set('status',value).catch(()=>{});}
 const rendering=new Latest<Picture>(async snapshot=>{for(const [k,v] of Object.entries(snapshot))await set(k.replace(/[A-Z]/g,c=>`-${c.toLowerCase()}`),v);await set('effect-enabled',enabled);await set('status',status);},e=>message(`窗口更新失败：${text(e)}`));
 function render(){rendering.submit({...picture});}
 async function accepted(change:ProcessingChange){
  unresolved=change;
  try{await host.processing.replace(change);unresolved=undefined;}
  catch(error){blocked=true;const op=await host.processing.operation({stage:'video',resourceId,revision:change.revision});
   if(op.status==='completed'){unresolved=undefined;blocked=false;return;}
   if(op.status==='failed'){unresolved=undefined;blocked=false;throw new Error(`${op.error.code}: ${op.error.message}`);}
   throw new Error(`处理结果 ${op.status}；请检查状态。${text(error)}`);
  }
 }
 const processing=new Latest<{picture:Picture;enabled:boolean}>(async desired=>{
  if(blocked)throw new Error('上次处理结果未确定，请检查状态');
  const state=await host.processing.state();
  if(state.backendUncertain.video)throw new Error(state.backendUncertain.video);
  if(state.source.phase!=='ready'){message(state.source.phase==='empty'?'没有媒体；已保留设置':`媒体状态 ${state.source.phase||'不可用'}；已保留设置`);return;}
  const own=state.video.find(x=>x.resource.manifestId==='vestra.video'&&x.resource.resourceId===resourceId);
  revision=Math.max(revision,own?.revision??0)+1;message('正在应用画面设置…');
  await accepted({stage:'video',resourceId,revision,expectedGeneration:state.source.generation,nodes:desired.enabled?nodesFor(desired.picture):[]});
  const source=await host.source.state();message(source.paused?'已接受；暂停画面可能在继续播放后更新':'画面设置已接受');
 },e=>message(`画面处理失败：${text(e)}`));
 function apply(){processing.submit({picture:{...picture},enabled});}
 async function refresh(){const state=await host.processing.state();const identity=`${state.source.generation}:${state.source.phase}`;if(identity!==lastSource){lastSource=identity;if(!blocked)apply();}}
 async function retry(){try{
  const state=await host.processing.state();if(state.backendUncertain.video)throw new Error(state.backendUncertain.video);
  const own=state.video.find(x=>x.resource.manifestId==='vestra.video'&&x.resource.resourceId===resourceId);
  if(unresolved){const op=await host.processing.operation({stage:'video',resourceId,revision:unresolved.revision});
   if(op.status==='pending'||op.status==='unknown'){message(`上次处理结果 ${op.status}；请稍后再次检查`);return;}
   if(op.status==='unknownExpired'&&own?.revision!==unresolved.revision){message('上次处理结果已过期；请重新加载插件以安全撤销自身资源');return;}
   unresolved=undefined;
  }
  revision=Math.max(revision,own?.revision??0);blocked=false;apply();
 }catch(e){message(`状态查询失败：${text(e)}`);}}
 let opening:Promise<void>|undefined;
 async function open(){if(opening)return opening;opening=(async()=>{if(!opened){await host.ui.open(view,'picture.slint',{},['adjust','preset','reset','toggle-effect','toggle-blur','retry','hide-tool']);opened=true;render();}await host.ui.show(view,true);})();try{await opening;}finally{opening=undefined;}}
 const offCommands=host.commands.onExecute(e=>{if(e.id==='video.picture'||e.id==='video.gaussian')void open().catch(e=>message(text(e)));});
 const offUi=host.ui.onCallback(e=>{if(e.view!==view||disposed)return;
  if(e.callback==='hide-tool'){void host.ui.show(view,false).catch(e=>message(text(e)));return;}
  if(e.callback==='adjust'){const [field,value]=e.args;if(typeof value!=='number'||!Number.isFinite(value))return;if(field==='brightness'||field==='contrast'||field==='saturation'||field==='gamma')picture[field]=clamp(value,-100,100);else if(field==='blurStrength')picture.blurStrength=clamp(value,0,20);else return;}
  else if(e.callback==='preset'){const p=presets[String(e.args[0])];if(!p)return;picture={...p};}
  else if(e.callback==='reset')picture={...defaults};
  else if(e.callback==='toggle-effect')enabled=e.args[0]===true;
  else if(e.callback==='toggle-blur')picture.blurEnabled=e.args[0]===true;
  else if(e.callback==='retry'){void retry();return;}else return;render();apply();
 });
 let offSource:(()=>Promise<void>)|undefined;
 try{for(const [id,title] of [['video.picture','画面调节…'],['video.gaussian','高斯模糊…']])await host.commands.register(id!,1,{title:title!,path:['视频',title!],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});offSource=await host.events.subscribe('source.state',()=>{void refresh().catch(e=>message(text(e)));});await refresh();}
 catch(e){offCommands();offUi();processing.stop();rendering.stop();await offSource?.();throw e;}
 return async()=>{disposed=true;offCommands();offUi();processing.stop();rendering.stop();await offSource?.();if(opened)await host.ui.close(view);};
}
