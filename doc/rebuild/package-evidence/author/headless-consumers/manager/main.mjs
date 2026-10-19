export async function activate(host){
 const fs=await import('node:fs/promises'), path=await import('node:path'),assert=(await import('node:assert/strict')).default;
 const root=process.env.VESTRA_PACKAGE_PROOF; const trace=[];const pause=()=>new Promise(r=>setTimeout(r,20));
 async function until(fn,predicate){const end=Date.now()+30000;let v;do{v=await fn();trace.push(v);if(predicate(v))return v;await pause();}while(Date.now()<end);throw Error('deadline '+JSON.stringify(v));}
 const terminal=s=>['committed','failed','cancelled','failedDegraded'].includes(s);
 async function operation(v){return until(()=>host.packages.operation(v.operationId),r=>terminal(r.status));}
 try{
 const a=await host.packages.validate(path.join(root,'one')),b=await host.packages.validate(path.join(root,'two')),bad=await host.packages.validate(path.join(root,'bad'));trace.push(a,b,bad);
 const trial=await host.packages.trial(b.token);await until(()=>host.packages.trialState(trial.trialId),v=>v.status==='ready');
 await host.packages.stopTrial(trial.trialId);await until(()=>host.packages.trialState(trial.trialId),v=>v.status==='stopped'&&v.cleanupConfirmed);
 const first=await operation(await host.packages.install(a.token,0,'install-one'));assert.equal(first.status,'committed');assert.equal(first.installed.current.manifest.version,'1.0.0');
 const failed=await operation(await host.packages.install(bad.token,1,'bad-upgrade'));assert.equal(failed.status,'failed');assert.equal(failed.restored,true);assert.equal((await host.packages.installed('community.counter')).revision,1);
 const second=await operation(await host.packages.install(b.token,1,'install-two'));assert.equal(second.status,'committed');assert.equal(second.installed.current.manifest.version,'1.1.0');
 const old=await operation(await host.packages.rollback('community.counter',2,'back-one'));assert.equal(old.status,'committed');assert.equal(old.installed.current.manifest.version,'1.0.0');
 assert.equal((await host.packages.install(a.token,0,'install-one')).operationId,'install-one');
 const late=await host.packages.cancel('back-one');assert.equal(late.status,'tooLate');
 const cancel=await host.packages.install(b.token,3,'cancel-upgrade');await host.packages.cancel(cancel.operationId);const cancelled=await operation(cancel);assert.equal(cancelled.status,'cancelled');assert.equal((await host.packages.installed('community.counter')).revision,3);
 await fs.writeFile(path.join(root,'result.json'),JSON.stringify({passed:true,trace,limits:['Author protocol lifecycle proof, not visible button/menu interaction.','Production JS and real Slint compiler; headless run does not show native windows.','Node candidates are trusted, not sandboxed.']},null,2));
 }catch(e){await fs.writeFile(path.join(root,'result.json'),JSON.stringify({passed:false,error:String(e),stack:e.stack,trace},null,2));}
}