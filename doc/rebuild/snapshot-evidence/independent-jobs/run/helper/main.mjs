export const activate=async function helper(host){
 const fs=await import('node:fs/promises'),path=await import('node:path'),work=process.env.VESTRA_MEDIA_JOBS_REVIEW,state=await host.source.state();
 const prior=await host.jobs.operation(1),snapshot=await host.snapshot.capture({operationId:1,expected:state.stamp,time:20}),jobs=[];
 for(let i=0;i<2;i++)jobs.push(await host.jobs.start({operationId:i+2,snapshotId:snapshot.snapshotId,kind:'rawPng',destination:path.join(work,'owner-cancel',`owner-${i}.png`)}));
 const states=await Promise.all(jobs.map(j=>host.jobs.state(j.jobId)));await fs.writeFile(path.join(work,'helper-started.json'),JSON.stringify({prior,snapshot,jobs,states},null,2));
};
