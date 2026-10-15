// Format checked against NousResearch/hermes-agent (May 2026): run_agent.py
// writes ~/.hermes/sessions/session_<id>.json {session_id, model, base_url,
// platform, session_start/last_updated naive-UTC ISO, system_prompt, tools,
// message_count, messages[] in OpenAI chat shape + `reasoning`}. NOTE: upstream
// docs now call ~/.hermes/state.db canonical; the adapter does not read it
// (see trx-fjzz). This case covers the JSON snapshot.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const session = {
  session_id: '20260418_045325_a1b2c3',
  model: 'anthropic/claude-sonnet-4.5',
  base_url: 'https://openrouter.ai/api/v1',
  platform: 'cli',
  session_start: '2026-04-18T04:53:25.274422',
  last_updated: '2026-04-18T04:54:10.500000',
  system_prompt: 'You are Hermes Agent.',
  tools: [{ type: 'function', function: { name: 'terminal', description: 'Run a command', parameters: { type: 'object', properties: {} } } }],
  message_count: 5,
  messages: [
    { role: 'user', content: 'How much disk space is free?' },
    { role: 'assistant', content: '', reasoning: 'I should run df.', finish_reason: 'tool_calls',
      tool_calls: [{ id: 'call_df_1', call_id: 'call_df_1', type: 'function', function: { name: 'terminal', arguments: '{"command":"df -h /"}' } }] },
    { role: 'tool', tool_call_id: 'call_df_1', tool_name: 'terminal', content: '{"output":"/dev/sda1 100G 40G 60G 40% /","exit_code":0}' },
    { role: 'assistant', content: '', finish_reason: 'tool_calls',
      tool_calls: [{ id: 'call_bad_2', call_id: 'call_bad_2', type: 'function', function: { name: 'terminal', arguments: '{"command":"nosuchcmd"}' } }] },
    { role: 'tool', tool_call_id: 'call_bad_2', tool_name: 'terminal', content: '{"output":"nosuchcmd: command not found","exit_code":127}' },
    { role: 'assistant', content: 'You have 60G free on /.', finish_reason: 'stop' },
  ],
};

export const cases: ConformanceCase[] = [{
  name: 'session-json-with-tools',
  async setup(home) {
    const dir = join(home, '.hermes', 'sessions');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, `session_${session.session_id}.json`), JSON.stringify(session, null, 2));
    // partial jsonl sibling must be ignored
    await writeFile(join(dir, `${session.session_id}.jsonl`), JSON.stringify({ role: 'user', content: 'ignored' }) + '\n');
    return dir;
  },
}];
