export const activate=async function helper(host){
 const fs=await import('node:fs/promises'),path=await import('node:path'),{createHash}=await import('node:crypto'),work=process.env.VESTRA_PROCESSED_REVIEW,state=await host.source.state();
 const processing=await host.processing.replace({stage:'video',resourceId:'old',revision:1,expectedGeneration:state.stamp.generation,nodes:[{kind:'eq',brightness:.12,contrast:1,saturation:1,gamma:1},{kind:'gblur',sigma:1.3,steps:3}]});
 const bytes=await fs.readFile(path.join(work,'old.srt')),s=host.subtitles,initial=await s.state();await s.begin(1,state.stamp.generation,bytes.length);await s.append(1,0,bytes.toString('base64'));const subtitle=await s.commit({operationId:1,uploadId:1,sha256:createHash('sha256').update(bytes).digest('hex'),resourceId:'old',revision:1,expected:initial.selection.stamp});
 const style=await s.style({operationId:2,revision:1,expected:subtitle.selection.stamp,style:{fontId:subtitle.presentation.fonts[0].id,fontSize:72,color:'yellow',outline:1,marginPercent:5,offsetMs:250}});await fs.writeFile(path.join(work,'helper-ready.json'),JSON.stringify({processing,subtitle,style},null,2));
};
