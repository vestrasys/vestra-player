export const activate=async function helper(host){
 const fs=await import('node:fs/promises'),path=await import('node:path'),s=host.subtitles,state=await s.state();
 const result=await s.style({operationId:1,revision:1,expected:state.selection.stamp,style:{fontId:state.presentation.fonts[0].id,fontSize:72,color:'cyan',outline:2,marginPercent:5,offsetMs:0}});
 await fs.writeFile(path.join(process.env.VESTRA_SUBTITLE_STYLE_REVIEW,'helper-result.json'),JSON.stringify(result,null,2));
};
