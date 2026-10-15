/* Cline adapter — VS Code globalStorage task directories. */
import{readdir,readFile,stat}from'fs/promises';import{basename,join}from'path';import{homedir}from'os';import type{Adapter,AdapterInfo,Conversation,Message,ParseOptions,ToolCall}from'../types/index.ts';import{isUnderAnyCanonicalRoot,runAdapter,textOnlyParts,textPart,thinkingPart,toolCallPart,toolResultPart}from'../types/index.ts';const ROOTS=[...['Code','Code - Insiders','VSCodium'].map(v=>join(homedir(),'.config',v,'User','globalStorage','saoudrizwan.claude-dev','tasks')),join(process.env.CLINE_DATA_DIR??join(process.env.CLINE_DIR??join(homedir(),'.cline'),'data'),'tasks')];type J=Record<string,any>;const adapter:Adapter={info():AdapterInfo{return{name:'cline',displayName:'Cline',version:'1.0.0',defaultPaths:ROOTS}},async detect(p){if(!isUnderAnyCanonicalRoot(p,ROOTS))return null;return(await dirs(p,true)).length?0.9:null},async parse(p,o){const x:Conversation[]=[];for(const d of await dirs(p,false)){const c=await parse(d,o);if(c)x.push(c)}x.sort((a,b)=>b.createdAt-a.createdAt);return o?.limit?x.slice(0,o.limit):x}};async function dirs(p:string,s:boolean){const o:string[]=[];async function exists(f:string){try{return(await stat(f)).isFile()}catch{return false}}async function w(d:string,n:number){if(await exists(join(d,'api_conversation_history.json'))||await exists(join(d,'ui_messages.json'))){o.push(d);return}if(n>2)return;try{for(const e of await readdir(d,{withFileTypes:true})){if(e.isDirectory())await w(join(d,e.name),n+1);if(s&&o.length)return}}catch{}}await w(p,0);return o}async function parse(d:string,o?:ParseOptions):Promise<Conversation|null>{
let title:string|undefined;try{title=JSON.parse(await readFile(join(d,'task_metadata.json'),'utf8')).title}catch{}
const read=async(n:string):Promise<J[]>=>{try{const x=JSON.parse(await readFile(join(d,n),'utf8'));return Array.isArray(x)?x:[]}catch{return[]}};
const ui=await read('ui_messages.json'),api=await read('api_conversation_history.json');
const ms:Message[]=[];let created=0,updated=0;const seen=(ts?:number)=>{if(ts){created=created?Math.min(created,ts):ts;updated=Math.max(updated,ts)}};
for(const x of ui)seen(time(x.ts??x.timestamp));
const toolText=(c:any):string=>typeof c==='string'?c:Array.isArray(c)?c.map(b=>typeof b==='string'?b:b?.text??'').join('\n'):'';
if(api.length){
/* Cline: api_conversation_history.json is the Anthropic MessageParam list; blocks text|thinking|tool_use|tool_result */
for(const x of api){const role=x.role;if(role!=='user'&&role!=='assistant')continue;const ts=time(x.ts??x.timestamp);seen(ts);
if(typeof x.content==='string'){if(x.content)ms.push({role,content:x.content,createdAt:ts,parts:textOnlyParts(x.content)});continue}
if(!Array.isArray(x.content))continue;const parts:any[]=[],texts:string[]=[],calls:ToolCall[]=[],results:J[]=[];
for(const b of x.content){if(b?.type==='text'&&b.text){texts.push(b.text);parts.push(textPart(b.text))}else if(b?.type==='thinking'&&b.thinking)parts.push(thinkingPart(b.thinking));else if(b?.type==='tool_use'){parts.push(toolCallPart(String(b.id),String(b.name),b.input));calls.push({toolName:String(b.name),input:b.input,status:'pending'})}else if(b?.type==='tool_result')results.push(b)}
if(o?.includeTools!==false)for(const r of results){const out=toolText(r.content);ms.push({role:'tool',content:out,createdAt:ts,parts:[toolResultPart(String(r.tool_use_id),out,{isError:Boolean(r.is_error)})]})}
if(parts.length)ms.push({role,content:texts.join('\n'),createdAt:ts,parts,toolCalls:calls.length?calls:undefined})}
}else for(const x of ui){/* no api history: fall back to the UI transcript */
const role=x.role==='user'||x.role==='assistant'?x.role:x.type==='say'&&(x.say==='task'||x.say==='user_feedback')?'user':x.type==='say'&&(x.say==='text'||x.say==='completion_result')?'assistant':null;
const content=typeof x.content==='string'?x.content:typeof x.text==='string'?x.text:'';if(!role||!content)continue;ms.push({role,content,createdAt:time(x.timestamp??x.ts),parts:textOnlyParts(content)})}
if(!ms.length)return null;
/* task directories are named by the task start time (ms) */
const dirTs=/^\d{13}$/.test(basename(d))?Number(basename(d)):undefined;if(!created&&dirTs)seen(dirTs);
if(o?.since&&updated<o.since)return null;
return{externalId:basename(d),title:(title??ui.find(x=>x.type==='say'&&x.say==='task'&&x.text)?.text??ms.find(m=>m.role==='user')?.content??'(untitled)').slice(0,100),createdAt:created||Date.now(),updatedAt:updated||created,messages:ms,metadata:{source:'cline'}}}
function time(v:any){if(typeof v==='number')return Math.floor(v<1e11?v*1000:v);if(typeof v==='string'){const n=Date.parse(v);return Number.isFinite(n)?Math.floor(n):undefined}}runAdapter(adapter);
