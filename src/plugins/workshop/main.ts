import type { Host, PackageOperation, PackageValidation, PackageTrial } from '../../sdk/index.ts';
import { WorkshopJob, requirement, defaultRequirement } from './engine.mjs';
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';

export async function activate(host: Host) {
  let opened = false, disposed = false, busy = false;
  let draft = defaultRequirement, job: WorkshopJob | undefined;
  let validation: PackageValidation | undefined, trial: PackageTrial | undefined, operation: PackageOperation | undefined;
  let trialPending = false, trialRecovering = false;
  let trialRequestId: string | undefined;
  let selectedId = 'community.counter', log = '', phase = '选择工作区并输入需求后生成', workspace = '';
  let poll: ReturnType<typeof setTimeout> | undefined;
  const terminal = new Set(['committed','cancelled','failed','failedDegraded','tooLate']);
  const set = async (property: string, value: string | boolean) => { if (opened && !disposed) await host.ui.set('workshop',property,value); };
  const publish = async (text: string) => { log = (log + '\n' + text).slice(-16000); await set('log-text',log); };
  const status = async (text: string) => { phase = text; await set('status-text',text); };
  const controls = async () => {
    await set('busy',busy);
    await set('can-trial',!!validation && !busy && !trial && !trialPending && !operation);
    await set('can-stop',!!trial);
    await set('can-recover',trialPending && !!trialRequestId && !trialRecovering);
    await set('can-install',!!validation && !busy && !trial && !trialPending && !operation);
    await set('workspace-text',workspace || '尚未选择：请选择专用空文件夹');
  };
  const error = async (e: unknown) => { await status(`错误：${String(e)}`); await publish(String(e)); };
  const open = async () => {
    if (opened) { await host.ui.show('workshop',true); return; }
    await host.ui.open('workshop','workshop.slint',{'requirement-text':draft,'log-text':log,'status-text':phase,'workspace-text':workspace,'busy':busy,'can-trial':false,'can-stop':!!trial,'can-install':false,'can-recover':trialPending && !!trialRequestId},['choose-workspace','choose-requirement','generate','cancel-task','start-trial','stop-trial','recover-trial','install-candidate','rollback-version','draft-changed']);
    opened = true; await controls();
  };
  const stopTrial = async () => {
    if (!trial) return;
    trial = await host.packages.stopTrial(trial.trialId);
    while (!trial.cleanupConfirmed && trial.status !== 'stopped' && trial.status !== 'failed') {
      await status(`试载清理中：${trial.status}`);
      await new Promise(resolve=>setTimeout(resolve,250));
      trial = await host.packages.trialState(trial.trialId);
    }
    if (!trial.cleanupConfirmed) throw Error(`试载清理未确认：${JSON.stringify(trial)}`);
    trial = undefined; await status('试载已清理'); await controls();
  };
  const observe = async () => {
    if (!operation || disposed) return;
    try {
      operation = await host.packages.operation(operation.operationId);
      await status(`包操作 ${operation.operationId}：${operation.status}${operation.message ? ' / '+operation.message : ''}`);
      await publish(JSON.stringify(operation));
      if (terminal.has(operation.status)) { operation = undefined; busy = false; await controls(); return; }
    } catch(e) { await status(`包操作结果未知，继续按相同operationId查询：${String(e)}`); }
    poll = setTimeout(()=>void observe(),400);
  };
  const managed = async (rollback: boolean) => {
    if (busy || trial || trialPending || operation) throw Error('请先等待任务完成并停止试载');
    if (!rollback && !validation) throw Error('请先生成并通过宿主校验');
    busy=true;await controls();
    const requestId=randomUUID();
    let submitted=false;
    try {
      const installed=await host.packages.installed(selectedId);
      await publish(`用户明确${rollback?'回退':'安装'}；requestId=${requestId}，expectedRevision=${installed.revision}`);
      submitted=true;
      operation=rollback ? await host.packages.rollback(selectedId,installed.revision,requestId) : await host.packages.install(validation!.token,installed.revision,requestId);
      await observe();
    } catch(e) {
      if (submitted && (e as {code?:string})?.code === 'timeout') {
        // Public packages maintainer guarantees operationId === requestId.
        operation={operationId:requestId,status:'preparing'};
        await status(`请求答复丢失；正在查询同一operationId ${requestId}`);
        await observe(); return;
      }
      if (!operation) { busy=false;await controls(); }
      throw e;
    }
  };
  const generate = async () => {
    if (busy || trial || trialPending || operation) throw Error('请先停止试载并等待当前任务完成');
    if (!workspace) throw Error('请先选择本次工坊专用工作区');
    requirement(draft); busy=true; validation=undefined;await controls();
    job=new WorkshopJob(path.join(workspace,`candidate-${randomUUID()}`), event=>{void publish(`${event.phase}: ${event.text}`).catch(()=>{});});
    try {
      await status('Pi Agent运行中 · 模型本地mock · 工具真实执行');
      const result=await job.run(draft); selectedId=job.requirement.id;
      await status('TypeScript通过，宿主验证生产JS/Slint…');
      const checked=await host.packages.validate(result.directory);
      if (job.cancelled || disposed) throw Error('任务已取消');
      validation=checked;
      await publish(JSON.stringify({candidate:result.directory,revision:result.revision,validation}));
      await status(`候选已通过宿主校验 ${validation.digest}；可试载或明确安装`);
    } catch(e) { validation=undefined; if(job.cancelled) await status('生成已取消，子进程已退出'); else await error(e); }
    finally {busy=false;await controls();}
  };
  const recoverTrial = async () => {
    if (!trialPending || !trialRequestId || trialRecovering) return;
    trialRecovering=true;await controls();
    try {
      trial=await host.packages.trialStateByRequest(trialRequestId);
      trialPending=false;busy=false;
      await publish(`恢复原试载 requestId=${trialRequestId}：${JSON.stringify(trial)}`);
      await status(`已恢复原试载：${trial.status}；未重新申请试载`);
      if (trial.cleanupConfirmed) trial=undefined;
    } catch(e) {
      await status(`试载结果仍未知（${trialRequestId}）：${String(e)}；可点击查询试载，禁止新申请`);
    } finally {trialRecovering=false;await controls();}
  };
  const dispatch = async (name: string, args: unknown[] = []) => {
    if(name==='draft-changed'){draft=String(args[0]??'');return;}
    if(name==='choose-workspace'){
      if(busy||trial||trialPending||operation)throw Error('活动任务期间不能更换工作区');
      const result=await host.dialogs.open({kind:'openDirectory',title:'选择工坊专用工作区（每个候选写入新子目录）'});
      if(!result.cancelled&&result.paths[0]){workspace=await fs.realpath(result.paths[0]);validation=undefined;await controls();}return;
    }
    if(name==='choose-requirement'){
      const result=await host.dialogs.open({kind:'openFile',title:'选择计数器需求JSON'});
      if(!result.cancelled&&result.paths[0]){const stat=await fs.stat(result.paths[0]);if(stat.size>65536)throw Error('需求超过64KiB');const text=await fs.readFile(result.paths[0],'utf8');requirement(text);draft=text;await set('requirement-text',draft);await status('需求已读取，点击生成');}return;
    }
    if(name==='generate')return generate();
    if(name==='cancel-task'){
      if(job&&busy&&!operation&&!trialPending){job.cancel();await job.agent?.waitForIdle();await status('生成取消已请求，等待任务清理');}
      if(trialPending)await recoverTrial();
      if(trial)await stopTrial();
      if(operation){operation=await host.packages.cancel(operation.operationId);await observe();}return;
    }
    if(name==='start-trial'){
      if(!validation||busy||trial||trialPending||operation)throw Error('候选尚未就绪');
      trialPending=true;busy=true;trialRequestId=randomUUID();await controls();
      try { trial=await host.packages.trial(validation.token,trialRequestId); }
      catch(e) {
        if ((e as {code?:string})?.code === 'timeout') {
          await status(`试载首答复丢失，正在查询同一requestId ${trialRequestId}`);
          await recoverTrial();return;
        }
        trialPending=false;busy=false;trialRequestId=undefined;await controls();throw e;
      }
      trialPending=false;busy=false;await controls();
      while(trial.status==='preparing'||trial.status==='activating'||trial.status==='starting'){
        await status(`试载：${trial.status}`);await new Promise(resolve=>setTimeout(resolve,250));trial=await host.packages.trialState(trial.trialId);
      }
      await publish(JSON.stringify(trial));await status(`试载：${trial.status}；请在独立计数器窗口测试增加/归零`);return;
    }
    if(name==='recover-trial')return recoverTrial();
    if(name==='stop-trial')return stopTrial();
    if(name==='install-candidate')return managed(false);
    if(name==='rollback-version')return managed(true);
  };
  const offCallback=host.ui.onCallback(e=>{if(e.view==='workshop')void dispatch(e.callback,e.args).catch(error);});
  const offCommand=host.commands.onExecute(e=>{if(e.id==='workshop.open')void open().catch(error);});
  await host.commands.register('workshop.open',1,{title:'插件工坊…',path:['插件','插件工坊…'],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});
  return async()=>{disposed=true;clearTimeout(poll);job?.cancel();await job?.agent?.waitForIdle();offCallback();offCommand();if(trial)await host.packages.stopTrial(trial.trialId).catch(()=>{});if(opened)await host.ui.close('workshop').catch(()=>{});};
}
