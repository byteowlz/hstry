// Format checked against xai-org/grok-build public snapshot (SOURCE_REV 559751f, crates/codegen):
//   xai-grok-sampling-types/src/conversation.rs (ConversationItem, tagged by "type": system|user|
//   assistant|tool_result|backend_tool_call|reasoning; Human user items omit synthetic_reason),
//   xai-grok-shell/src/session/persistence.rs (CHAT_FORMAT_VERSION = 1, summary.json),
//   plus pond docs/adapters/grok-build.md (verified against grok-build 1.0.5 captures).
// NOTE: grok's stream of record is updates.jsonl; chat_history.jsonl (read by this adapter) is its
// derived projection. Tool results carry no error flag in that file.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';

async function session(home: string, bucket: string, id: string, summary: unknown, history: unknown[]): Promise<string> {
  const root = join(home, '.grok', 'sessions');
  const dir = join(root, bucket, id);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, 'summary.json'), JSON.stringify(summary, null, 2));
  await writeFile(join(dir, 'chat_history.jsonl'), lines(history));
  return root;
}

export const cases: ConformanceCase[] = [
  {
    name: 'chat-format-v1',
    async setup(home) {
      const id = '0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b';
      return session(home, '%2Fwork%2Fdemo', id, {
        info: { id, cwd: '/work/demo' }, agent_id: 'grok-build', session_summary: 'Add a README', generated_title: 'Add a README',
        created_at: '2026-08-24T10:00:00.250Z', updated_at: '2026-08-24T10:05:00Z', last_active_at: '2026-08-24T10:04:30Z',
        num_messages: 7, current_model_id: 'grok-4.6', session_kind: 'fork', parent_session_id: '0198a1b2-0000-7000-8000-000000000000',
        chat_format_version: 1,
      }, [
        { type: 'system', content: 'You are Grok Build, a coding agent.' },
        { type: 'user', content: [{ type: 'text', text: '<system-reminder>Project rules</system-reminder>' }], synthetic_reason: 'project_instructions' },
        { type: 'user', content: [{ type: 'text', text: 'Add a README for this repo' }], prompt_index: 0 },
        { type: 'reasoning', id: 'rs_1', summary: [{ type: 'summary_text', text: 'I should read the files first.' }] },
        { type: 'assistant', content: 'Let me look around.', model_id: 'grok-4.6', tool_calls: [
          { id: 'call_1', name: 'read_file', arguments: '{"path":"README.md"}' },
          { id: 'call_2', name: 'run_terminal_command', arguments: '{"command":"cat calc.py"}' },
        ] },
        { type: 'tool_result', tool_call_id: 'call_1', content: 'Error: file not found: README.md' },
        { type: 'tool_result', tool_call_id: 'call_2', content: 'def add(a, b):\n    return a + b\n' },
        { type: 'user', content: [{ type: 'text', text: 'continue' }], synthetic_reason: 'auto_continue' },
        { type: 'assistant', content: 'README.md created.', model_id: 'grok-4.6' },
      ]);
    },
  },
  {
    name: 'legacy-chat-format-v0',
    async setup(home) {
      const id = '0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5c';
      return session(home, '%2Fwork%2Fold', id, {
        info: { id, cwd: '/work/old' }, created_at: '2026-01-01T10:00:00Z', updated_at: '2026-01-01T10:01:00Z', current_model_id: 'grok-code-fast-1',
      }, [
        { role: 'system', content: 'rules' },
        { role: 'user', content: 'List files' },
        { role: 'assistant', content: '', tool_calls: [{ id: 'c1', type: 'function', function: { name: 'ls', arguments: '{"dir":"."}' } }] },
        { role: 'tool', tool_call_id: 'c1', content: 'a.txt' },
        { role: 'assistant', content: 'One file: a.txt' },
      ]);
    },
  },
];
