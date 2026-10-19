export async function activate(host) {
  const fs=await import('node:fs/promises');
  const assert=(await import('node:assert/strict')).default;
  const root=process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT;
  const deadline=Date.now()+70000;
  let provisional;
  try {
    while(Date.now()<deadline) {
      try {provisional=JSON.parse(await fs.readFile(root+'/lifecycle-provisional.json','utf8'));break;}
      catch(error){if(error.code!=='ENOENT')throw error;}
      await new Promise(resolve=>setTimeout(resolve,30));
    }
    assert.ok(provisional,'manager lifecycle did not complete');
    const committed=await host.packages.operation('ind-back');
    assert.equal(committed.status,'committed');
    const owner=committed.owner;
    assert.equal(typeof owner,'number');
    await host.plugins.disable('independent.package.manager');
    const plugins=await host.plugins.list();
    assert.equal(plugins.some(p=>p.id==='independent.package.manager'),false);
    const activation=await host.plugins.activation(owner);
    assert.equal(activation.activation.status,'ready');
    assert.equal((await host.packages.installed('independent.counter')).revision,3);
    provisional.trace.push({label:'authoring manager stopped, installed owner remains ready',value:{plugins,owner,activation}});
    await fs.writeFile(root+'/lifecycle-result.json',JSON.stringify(provisional,null,2));
  } catch(error) {
    await fs.writeFile(root+'/lifecycle-result.json',JSON.stringify({passed:false,error:String(error),stack:error.stack,provisional},null,2));
  }
}
