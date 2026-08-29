import { mkdtemp, mkdir, readFile, writeFile, rm } from 'fs/promises';
import { join } from 'path';
import { tmpdir } from 'os';
const home=await mkdtemp(join(tmpdir(),'hstry-omp-')); const root=join(home,'.omp','agent','sessions'); await mkdir(root,{recursive:true}); await writeFile(join(root,'session.jsonl'),await readFile(new URL('../../testdata/pi/test-session.jsonl',import.meta.url)));
function req(method,params={}){const r=Bun.spawnSync(['bun','run',new URL('./adapter.ts',import.meta.url).pathname],{env:{...process.env,HOME:home,HSTRY_REQUEST:JSON.stringify({method,params})}});if(!r.success)throw new Error(r.stderr.toString());return JSON.parse(r.stdout.toString())}
try{if(req('detect',{path:root})===null)throw new Error('detect failed');if(req('detect',{path:join(home,'.pi','agent','sessions')})!==null)throw new Error('claimed pi');const c=req('parse',{path:root,opts:{}});if(!c.length||!Number.isInteger(c[0].createdAt))throw new Error('parse failed');console.log('omp adapter fixture test passed')}finally{await rm(home,{recursive:true,force:true})}
