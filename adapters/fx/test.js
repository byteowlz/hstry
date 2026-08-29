import { mkdtemp, mkdir, writeFile, rm } from 'fs/promises';
import { join } from 'path';
import { tmpdir } from 'os';
const home = await mkdtemp(join(tmpdir(), 'hstry-fx-'));
const root = join(home, '.fx', 'sessions');
const dir = join(root, '1700000000000-1700000000000000000-abcdef0123456789');
await mkdir(dir, { recursive: true });
await writeFile(join(dir, 'session.json'), JSON.stringify({ schema_version: 3, storage_format: 'event_log_v1', id: 'fx-1', created_at_ms: 1700000000000, updated_at_ms: 1700000003000, workspace_root: '/work/fx', preferences: { model: 'xai/grok', provider: 'grok' } }));
const events = [
  { schema_version: 1, seq: 1, timestamp_ms: 1700000000000, kind: 'session_started', payload: { id: 'fx-1', created_at_ms: 1700000000000, workspace_root: '/work/fx', preferences: { model: 'xai/grok', provider: 'grok' } } },
  { schema_version: 1, seq: 2, timestamp_ms: 1700000003000, kind: 'history_turn_committed', payload: { total_input_tokens: 12, total_output_tokens: 7, turn: { assistant: { user: { text: 'Build the file server' }, assistant: 'Done.', execution: { tool_steps: [{ assistant: 'Checking.', tool_calls: [{ id: 'c1', name: 'read_file', arguments_json: '{"path":"a"}' }], tool_results: [{ tool_call_id: 'c1', tool_name: 'read_file', status: 'success', output: 'contents', created_at_ms: 1700000002000 }] }] } } } } },
  '{malformed',
].map(v => typeof v === 'string' ? v : JSON.stringify(v)).join('\n');
await writeFile(join(dir, 'events.jsonl'), events + '\n');
function request(method, params = {}) { const r = Bun.spawnSync(['bun','run',new URL('./adapter.ts',import.meta.url).pathname], { env: { ...process.env, HOME: home, HSTRY_REQUEST: JSON.stringify({method,params}) } }); if (!r.success) throw new Error(r.stderr.toString()); return JSON.parse(r.stdout.toString()); }
try {
  if (request('detect',{path:root}) !== 0.95) throw new Error('detect failed');
  if (request('detect',{path:join(home,'.fx-other')}) !== null) throw new Error('claimed noncanonical path');
  const [c] = request('parse',{path:root,opts:{}});
  if (c.externalId !== 'fx-1' || c.workspace !== '/work/fx') throw new Error('metadata lost');
  if (c.tokensIn !== 12 || c.tokensOut !== 7) throw new Error('usage lost');
  if (!c.messages.some(m => m.role === 'tool') || c.messages[0].content !== 'Build the file server') throw new Error('turn lost');
  if (!Number.isInteger(c.createdAt)) throw new Error('timestamp not integer');
  console.log('fx adapter fixture test passed');
} finally { await rm(home,{recursive:true,force:true}); }
