export async function activate(host) {
    const fs = await import('node:fs/promises');
    await host.commands.register('increase',1,{title:'Independent counter 1.0.0',path:['插件','Independent counter','Increase'],shortcut:null},{enabled:true,checked:null,busy:false,reason:null});
    let count=0; let observed=Promise.resolve();
    host.commands.onExecute(event=>{ if(event.id!=='increase')return; count+=1; const value=count;
      observed=observed.then(()=>fs.appendFile(process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT+'/commands.jsonl',JSON.stringify({run:process.env.VESTRA_INDEPENDENT_PACKAGE_RUN,version:"1.0.0",pid:process.pid,count:value})+'\n')); });
    await host.commands.execute('increase');
    await host.commands.execute('increase');
    while(count!==2)await new Promise(resolve=>setTimeout(resolve,5));
    await observed;
    await fs.appendFile(process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT+'/activations.jsonl',JSON.stringify({run:process.env.VESTRA_INDEPENDENT_PACKAGE_RUN,version:"1.0.0",pid:process.pid})+'\n');
    
  }