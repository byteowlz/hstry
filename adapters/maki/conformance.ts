// Format checked against tontinton/maki @54a1529 (v0.6.3, 2026-10): maki-storage/src/sessions.rs
// (LogRecord: header{v,id,model,cwd,created_at} | msg{d} | out{id,d} | sub_msg | frame | meta{title,
// token_usage,updated_at,usage_by_model,...}) and maki-providers/src/types.rs (Message{role,content[blocks],
// display_text,kind}, ContentBlock text|thinking|tool_use|tool_result{tool_use_id,content,is_error}).
// Sessions live under $XDG_STATE_HOME/maki/sessions/*.jsonl.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';

const usage = { input: 100, output: 40, cache_creation: 0, cache_read: 20, cost: 0.0123 };

export const cases: ConformanceCase[] = [{
  name: 'sessions-with-tools',
  async setup(home) {
    const dir = join(home, '.local', 'state', 'maki', 'sessions');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, '019e0000-0000-7000-8000-000000000001.jsonl'), jsonl([
      { t: 'header', v: 3, id: '019e0000-0000-7000-8000-000000000001', model: 'anthropic/claude-sonnet-4-5', cwd: '/work/proj', created_at: 1790000000000 },
      { t: 'msg', d: { role: 'user', content: [{ type: 'text', text: 'Show me the config file' }] } },
      { t: 'msg', d: { role: 'assistant', content: [
        { type: 'thinking', thinking: 'Need to read config.toml', signature: 'sig' },
        { type: 'text', text: 'Let me read it.' },
        { type: 'tool_use', id: 'tu_1', name: 'read', input: { path: 'config.toml' } },
      ] } },
      { t: 'msg', d: { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'tu_1', content: 'No such file: config.toml', is_error: true }] } },
      { t: 'out', id: 'tu_1', d: { Plain: { text: 'No such file: config.toml' } } },
      { t: 'msg', d: { role: 'user', content: [{ type: 'text', text: '<observation>cwd changed</observation>' }], kind: 'observation' } },
      { t: 'msg', d: { role: 'assistant', content: [{ type: 'text', text: 'The file does not exist in this directory.' }] } },
      { t: 'meta', title: 'Show me the config file', token_usage: { input_tokens: 100, output_tokens: 40, cache_creation_input_tokens: 0, cache_read_input_tokens: 20 }, updated_at: 1790000060000, usage_by_model: { 'anthropic/claude-sonnet-4-5': usage }, context_size: 1000 },
    ]));
    await writeFile(join(dir, '019e0000-0000-7000-8000-000000000002.jsonl'), jsonl([
      { t: 'header', v: 3, id: '019e0000-0000-7000-8000-000000000002', model: 'anthropic/claude-haiku-4-5', cwd: '/work/other', created_at: 1790001000000 },
      { t: 'msg', d: { role: 'user', content: [{ type: 'text', text: 'ping' }] } },
      { t: 'msg', d: { role: 'assistant', content: [{ type: 'text', text: 'pong' }] } },
      { t: 'meta', title: 'ping', token_usage: { input_tokens: 5, output_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 }, updated_at: 1790001005000 },
    ]));
    return dir;
  },
}];
