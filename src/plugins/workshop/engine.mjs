import { Agent } from '@earendil-works/pi-agent-core';
import { createAssistantMessageEventStream } from '@earendil-works/pi-ai';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import ts from './vendor/typescript/lib/typescript.js';
const here = path.dirname(fileURLToPath(import.meta.url));
export function requirement(text) {
  if (Buffer.byteLength(text) > 65536) throw Error('需求超过64KiB');
  const r = JSON.parse(text);
  if (!r || Object.keys(r).sort().join(',') !== 'id,kind,step,version' || r.kind !== 'native-counter' || r.id !== 'community.counter' || !((r.version === '1.0.0' && r.step === 1) || (r.version === '1.1.0' && r.step === 3))) throw Error('仅支持native-counter / community.counter，1.0.0步长1或1.1.0步长3');
  return Object.freeze(r);
}
export const defaultRequirement = JSON.stringify({kind:'native-counter',id:'community.counter',version:'1.0.0',step:1});
const counterUi = `import { Button, VerticalBox, HorizontalBox } from "std-widgets.slint";
export component CounterWindow inherits Window {
 title: "独立计数器"; width: 360px; height: 240px;
 in property <string> count: "0";
 callback increase(); callback reset();
 MenuBar { Menu { title: "计数"; MenuItem { title: "增加"; activated => { root.increase(); } } MenuItem { title: "归零"; activated => { root.reset(); } } } }
 VerticalBox { padding: 24px; Text { text: "计数：" + root.count; font-size: 28px; horizontal-alignment: center; }
 HorizontalBox { Button { text: "增加"; clicked => { root.increase(); } } Button { text: "归零"; clicked => { root.reset(); } } } }
}`;
function counterSource(step, broken) {
  return `import type { Host } from './types/index.ts';\nconst step: number = ${broken ? JSON.stringify(String(step)) : step};\nexport async function activate(host: Host) {\n let count = 0;\n const action = async (name: string) => { count = name === 'increase' ? count + step : 0; await host.ui.set('counter', 'count', String(count)); };\n const off = host.ui.onCallback(e => { if (e.view === 'counter' && (e.callback === 'increase' || e.callback === 'reset')) void action(e.callback); });\n try { await host.ui.open('counter', 'counter.slint', {count: '0'}, ['increase','reset']); } catch (e) { off(); throw e; }\n return async () => { off(); await host.ui.close('counter'); };\n}\n`;
}
export class WorkshopJob {
  constructor(directory, publish = () => {}) { this.directory = directory; this.publish = publish; this.revision = 0; this.cancelled = false; this.events = []; this.reports = []; this.child = null; }
  cancel() { this.cancelled = true; this.agent?.abort(); this.child?.kill('SIGKILL'); }
  check(signal) { if (this.cancelled || signal?.aborted) throw Error('任务已取消'); }
  async file(name) {
    if (!['main.ts','manifest.json','counter.slint'].includes(name)) throw Error('候选路径不在允许列表');
    if (await fs.realpath(this.directory) !== this.realRoot || (await fs.lstat(this.directory)).isSymbolicLink()) throw Error('工作区边界已改变');
    const f = path.join(this.directory, name);
    try { const st = await fs.lstat(f); if (!st.isFile() || st.isSymbolicLink()) throw Error('拒绝符号链接/非文件'); } catch (e) { if (e.code !== 'ENOENT') throw e; }
    return f;
  }
  async validate(revision, signal) {
    this.check(signal); if (revision !== this.revision) throw Error('候选revision过期');
    const args = [path.join(here,'vendor/typescript/lib/_tsc.js'),'--strict','--noEmit','--target','ES2022','--module','NodeNext','--moduleResolution','NodeNext','--allowImportingTsExtensions','--skipLibCheck','--types','node','--typeRoots',path.join(here,'vendor/@types'),path.join(this.directory,'main.ts')];
    const proc = spawn(process.execPath,args,{cwd:this.directory,stdio:['ignore','pipe','pipe']}); this.child = proc;
    let stdout='',stderr=''; const kill = () => proc.kill('SIGKILL'); signal?.addEventListener('abort',kill,{once:true});
    const timer = setTimeout(kill,20000);
    proc.stdout.on('data',b=>{stdout+=b; if(stdout.length>1000000)kill();}); proc.stderr.on('data',b=>{stderr+=b; if(stderr.length>1000000)kill();});
    try {
      const status = await new Promise((resolve,reject)=>{proc.once('error',reject);proc.once('close',(code,signal)=>resolve({code,signal}));});
      const text = await fs.readFile(await this.file('main.ts'),'utf8');
      const report = {revision, ...status, stdout, stderr, command:[process.execPath,...args], digest:createHash('sha256').update(text).digest('hex')};
      this.reports.push(report); this.publish({phase:'真实TypeScript诊断',text:JSON.stringify(report)}); this.check(signal);
      if (status.code === 0) await fs.writeFile(path.join(this.directory,'main.js'), ts.transpileModule(text,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText);
      return report;
    } finally {clearTimeout(timer);signal?.removeEventListener('abort',kill);this.child=null;}
  }
  async run(text) {
    const r = requirement(text); this.requirement=r;
    await fs.mkdir(this.directory,{recursive:true}); this.realRoot = await fs.realpath(this.directory);
    await fs.cp(path.join(here,'vendor/sdk'),path.join(this.directory,'types'),{recursive:true});
    await fs.writeFile(path.join(this.directory,'package.json'),'{"type":"module","private":true}');
    const result = value=>({content:[{type:'text',text:JSON.stringify(value)}],details:{}});
    const schema = properties=>({type:'object',properties,required:Object.keys(properties),additionalProperties:false});
    const tools = [
      {name:'write_candidate',label:'写入候选文件',description:'仅写本次独立候选的manifest/main.ts/Slint',parameters:schema({path:{type:'string'},text:{type:'string'}}),execute:async (_id,args,signal)=>{this.check(signal); if(Buffer.byteLength(args.text)>65536)throw Error('文件超限'); await fs.writeFile(await this.file(args.path),args.text); this.revision++; return result({revision:this.revision});}},
      {name:'read_candidate',label:'读取候选文件',description:'读取本次候选',parameters:schema({path:{type:'string'}}),execute:async (_id,args,signal)=>{this.check(signal);return result({revision:this.revision,text:await fs.readFile(await this.file(args.path),'utf8')});}},
      {name:'validate_candidate',label:'真实TypeScript校验',description:'运行固定版本离线TypeScript编译器',parameters:schema({revision:{type:'integer'}}),execute:async (_id,args,signal)=>{const report=await this.validate(args.revision,signal);return {...result(report),isError:report.code!==0};}}
    ];
    const model = {id:'finite-counter-mock',name:'本地确定性mock（无付费模型）',api:'openai-completions',provider:'local-mock',baseUrl:'http://invalid.localhost',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:32768,maxTokens:8192};
    let turn=0,actions=0;
    const streamFn = (model, context, options) => {
      const stream=createAssistantMessageEventStream();
      const message={role:'assistant',api:model.api,provider:model.provider,model:model.id,usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},content:[],stopReason:'stop',timestamp:Date.now()};
      try {
        this.check(options?.signal); if(++turn>3)throw Error('模型轮次超限');
        const call=(name,args)=>({type:'toolCall',id:`${turn}-${name}-${Math.random()}`,name,arguments:args});
        const reports=context.messages.filter(m=>m.role==='toolResult'&&m.toolName==='validate_candidate');
        if(turn===1)message.content=[call('write_candidate',{path:'manifest.json',text:JSON.stringify({id:r.id,version:r.version,apiVersion:'vestra.experimental/0.1',entry:'main.js',capabilities:['ui.window']},null,2)}),call('write_candidate',{path:'counter.slint',text:counterUi}),call('write_candidate',{path:'main.ts',text:counterSource(r.step,true)}),call('read_candidate',{path:'main.ts'}),call('validate_candidate',{revision:3})];
        else {
          const report=JSON.parse(reports.at(-1)?.content.map(c=>c.text??'').join('')??'null');
          if(turn===2){
            if(report?.code!==2 && report?.code!==1)throw Error('未得到真实失败诊断');
            if(!/main\.ts\(2,7\): error TS2322:/.test(report.stdout))throw Error('真实TS2322文件/位置不匹配，拒绝盲修');
            message.content=[call('write_candidate',{path:'main.ts',text:counterSource(r.step,false)}),call('read_candidate',{path:'main.ts'}),call('validate_candidate',{revision:report.revision+1})];
          } else {if(report?.code!==0)throw Error('修复后诊断未通过');message.content=[{type:'text',text:'独立插件已通过真实TypeScript诊断修复。下一步需要宿主Slint与包验证。'}];}
        }
        message.stopReason=message.content.some(c=>c.type==='toolCall')?'toolUse':'stop';
        stream.push({type:'start',partial:message});
        message.content.forEach((c,contentIndex)=>stream.push(c.type==='toolCall'?{type:'toolcall_end',contentIndex,toolCall:c,partial:message}:{type:'text_end',contentIndex,content:c.text,partial:message}));
        stream.push({type:'done',reason:message.stopReason,message});
      } catch(e) {message.stopReason=this.cancelled?'aborted':'error';message.errorMessage=String(e);stream.push({type:'error',reason:message.stopReason,error:message});}
      return stream;
    };
    this.agent=new Agent({initialState:{systemPrompt:'有限原生计数器工坊。工具真实执行；根据真实诊断修复；不得安装。',model,tools},streamFn,toolExecution:'sequential',getApiKey:()=>undefined,beforeToolCall:async(_,signal)=>{this.check(signal);if(++actions>12)throw Error('工具动作超限');}});
    const off=this.agent.subscribe(event=>{this.events.push(event);if(event.type==='tool_execution_start'||event.type==='tool_execution_end')this.publish({phase:'Pi工具编排',text:JSON.stringify(event)});});
    const deadline=setTimeout(()=>this.cancel(),120000);
    try {
      await this.agent.prompt(text);await this.agent.waitForIdle();this.check();
      const last=this.agent.state.messages.at(-1);
      if(last?.role!=='assistant'||last.stopReason!=='stop'||this.agent.state.errorMessage||this.reports.at(-1)?.code!==0)throw Error(last?.errorMessage??this.agent.state.errorMessage??'Agent未完成有效候选');
      await fs.writeFile(path.join(this.directory,'agent-events.json'),JSON.stringify(this.events,null,2));
      await fs.writeFile(path.join(this.directory,'diagnostics.json'),JSON.stringify(this.reports,null,2));
      return {directory:this.directory,revision:this.revision,actions,turns:turn,reports:this.reports};
    } finally {clearTimeout(deadline);off();this.agent.abort();await this.agent.waitForIdle();await fs.writeFile(path.join(this.directory,'agent-events.json'),JSON.stringify(this.events,null,2));await fs.writeFile(path.join(this.directory,'diagnostics.json'),JSON.stringify(this.reports,null,2));}
  }
}
