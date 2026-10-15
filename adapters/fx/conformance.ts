// Format checked against vercel-labs/fx @ 5a3a5f3 (CHANGELOG 0.0.14), src/core/session:
//   session_projection.zig (session.json manifest, schema_version 3 / event_log_v1) and
//   session_event.zig (ConversationEnvelope, schema_version 1..3: the current events.jsonl,
//   externally tagged {"seq","timestamp_ms","event":{"<kind>":{...}}}; tool_result carries
//   only a preview + artifact_ref). The legacy v3 envelope ({"kind","payload"}) is the
//   read-only importer format and is covered by the second case.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';

function manifest(id: string, created: number, updated: number, extra: Record<string, unknown> = {}) {
  return {
    schema_version: 3, storage_format: 'event_log_v1', id,
    authority_id: '0a'.repeat(16), log_generation: '0b'.repeat(16),
    created_at_ms: created, updated_at_ms: updated,
    origin_workspace_root: '/work/demo', workspace_root: '/work/demo', conversation_language: 'en',
    history_len: 1, total_input_tokens: 120, total_output_tokens: 45, last_event_seq: 9,
    event_log_bytes: 0, event_log_stat_fingerprint: '0c'.repeat(16),
    generation_base_seq: 0, generation_base_bytes: 0, checkpoint_seq: null, checkpoint_sha256: null,
    preferences: { model: 'anthropic/claude-sonnet-4.6', provider: 'vercel' },
    ...extra,
  };
}

async function session(home: string, name: string, meta: unknown, events: unknown[]): Promise<string> {
  const root = join(home, '.fx', 'sessions');
  const dir = join(root, name);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, 'session.json'), JSON.stringify(meta));
  await writeFile(join(dir, 'events.jsonl'), lines(events));
  return root;
}

const frame = (seq: number, ts: number, event: unknown) => ({ schema_version: 3, seq, timestamp_ms: ts, event });
const result = (callId: string, tool: string, status: string, preview: string, ts: number) => ({
  call_id: callId, tool_name: tool, status, artifact_ref: `${callId}.txt`, stored_bytes: preview.length,
  completeness: 'complete', preview, provider_native: false, created_at_ms: ts,
});

export const cases: ConformanceCase[] = [
  {
    name: 'conversation-frames',
    async setup(home) {
      return session(home, '1700000000000-1700000000000000000-aaaa0000bbbb1111', manifest('fx-frames-1', 1700000000000, 1700000009000), [
        frame(1, 1700000001000, { user: { text: 'Fix the failing test in src/lib.rs' } }),
        frame(2, 1700000002000, { assistant: { text: 'Let me look at the file.' } }),
        frame(3, 1700000002100, { tool_call: { call_id: 'call_1', tool_name: 'read_file', arguments_json: '{"path":"src/lib.rs"}' } }),
        frame(4, 1700000002200, { tool_call: { call_id: 'call_2', tool_name: 'shell', arguments_json: '{"command":"cargo test"}' } }),
        frame(5, 1700000003000, { tool_result: result('call_1', 'read_file', 'success', 'fn add(a: i32, b: i32) -> i32 { a - b }', 1700000003000) }),
        frame(6, 1700000004000, { tool_result: result('call_2', 'shell', 'failure', 'error: test failed, exit code 101', 1700000004000) }),
        frame(7, 1700000006000, { assistant: { text: 'The add function subtracts; fixed.' } }),
        frame(8, 1700000007000, { turn_completed: { files: [] } }),
        frame(9, 1700000008000, { user: { text: 'Thanks' } }),
        frame(10, 1700000009000, { interrupted: { reason: 'cancelled', partial_text: 'You are welc', files: [] } }),
      ]);
    },
  },
  {
    name: 'legacy-event-log',
    async setup(home) {
      const meta = manifest('fx-legacy-1', 1700100000000, 1700100003000, { total_input_tokens: 12, total_output_tokens: 7 });
      return session(home, '1700100000000-1700100000000000000-cccc2222dddd3333', meta, [
        { schema_version: 1, seq: 1, timestamp_ms: 1700100000000, kind: 'session_started', payload: { id: 'fx-legacy-1', created_at_ms: 1700100000000, workspace_root: '/work/demo', preferences: { model: 'anthropic/claude-sonnet-4.6', provider: 'vercel' } } },
        { schema_version: 1, seq: 2, timestamp_ms: 1700100003000, kind: 'history_turn_committed', payload: { total_input_tokens: 12, total_output_tokens: 7, turn: { assistant: { user: { text: 'Build the file server' }, assistant: 'Done.', execution: { tool_steps: [{ assistant: 'Checking.', tool_calls: [{ id: 'c1', name: 'read_file', arguments_json: '{"path":"a"}' }], tool_results: [{ tool_call_id: 'c1', tool_name: 'read_file', status: 'failure', output: 'ENOENT: a', created_at_ms: 1700100002000 }] }] } } } } },
      ]);
    },
  },
];
