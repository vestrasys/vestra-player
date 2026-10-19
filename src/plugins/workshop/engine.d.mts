export interface Requirement {kind:'native-counter';id:'community.counter';version:'1.0.0'|'1.1.0';step:1|3}
export function requirement(text:string):Requirement;
export const defaultRequirement:string;
export class WorkshopJob {
 constructor(directory:string,publish?:(event:{phase:string;text:string})=>void);
 directory:string;revision:number;cancelled:boolean;requirement:Requirement;
 agent?:{waitForIdle():Promise<void>;abort():void};
 cancel():void;
 run(text:string):Promise<{directory:string;revision:number;actions:number;turns:number;reports:unknown[]}>;
}
