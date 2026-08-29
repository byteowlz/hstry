import { mkdtemp, mkdir, writeFile, rm } from 'fs/promises';
import { join } from 'path';
import { tmpdir } from 'os';
const home = await mkdtemp(join(tmpdir(), 'hstry-grok-'));
const root = join(home,'.grok','sessions');
const dir = join(root,'work%2Frepo','grok-1');
await mkdir(dir,{recursive:true});
await writeFile(join(dir,'summary.json'),JSON.stringify({ info:{id:'grok-1',cwd:'/work/repo'}, generated_title:'File server', created_at:'2026-01-01T10:00:00Z', updated_at:'2026-01-01T10:02:00Z', current_model_id:'grok-code', chat_format_version:1, parent_session_id:'parent-1', agent_name:'coding' }));
const items = [
  {type:'system',content:'rules'},
  {type:'user',content:[{type:'text',text:'Build it'},{type:'image',url:'file:///tmp/a.png'}]},
  {type:'user',content:[{type:'text',text:'injected'}],synthetic_reason:'system_reminder'},
  {type:'reasoning',summary:[{text:'Think'}]},
  {type:'assistant',content:'Reading.',model_id:'grok-code',tool_calls:[{id:'call-1',name:'read_file',arguments:'{"path":"a"}'}]},
  {type:'tool_result',tool_call_id:'call-1',content:'data'},
  '{bad',
].map(v=>typeof v==='string'?v:JSON.stringify(v)).join('\n');
await writeFile(join(dir,'chat_history.jsonl'),items+'\n');
function request(method,params={}) { const r=Bun.spawnSync(['bun','run',new URL('./adapter.ts',import.meta.url).pathname],{env:{...process.env,HOME:home,HSTRY_REQUEST:JSON.stringify({method,params})}}); if(!r.success) throw new Error(r.stderr.toString()); return JSON.parse(r.stdout.toString()); }
try {
  if(request('detect',{path:root})!==0.95) throw new Error('detect failed');
  if(request('detect',{path:join(home,'.grok-other')})!==null) throw new Error('claimed noncanonical path');
  const [c]=request('parse',{path:root,opts:{}});
  if(c.externalId!=='grok-1'||c.parentExternalId!=='parent-1'||c.workspace!=='/work/repo') throw new Error('metadata lost');
  if(c.messages.some(m=>m.content==='injected')) throw new Error('synthetic user leaked');
  const assistant=c.messages.find(m=>m.role==='assistant');
  if(!assistant.parts.some(p=>p.type==='thinking')||!assistant.toolCalls?.length) throw new Error('rich parts lost');
  if(!Number.isInteger(c.createdAt)) throw new Error('timestamp not integer');
  console.log('grok adapter fixture test passed');
} finally { await rm(home,{recursive:true,force:true}); }
