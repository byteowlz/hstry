// Format checked against NousResearch/hermes-agent (main, 2026-10):
//  - state.db SCHEMA_SQL in hermes_state_common.py: `sessions` (id, source, model,
//    system_prompt, parent_session_id, started_at/last_activity_at epoch-seconds REAL,
//    tokens, cost, title, cwd, hidden, ...) and `messages` (id, session_id, role, content,
//    tool_call_id, tool_calls JSON, tool_name, timestamp REAL, finish_reason, reasoning,
//    reasoning_content, active, compacted, _compressed_summary, ...).
//  - `hermes sessions export --format jsonl` (hermes_cli/session_export.py): one session
//    object per line with the same columns plus `messages`.
//  - legacy ~/.hermes/sessions/session_<id>.json snapshots (run_agent.py).
// Verified against the real public export at cfahlgren1/hermes-agent-traces (sessions.jsonl).
import { Database } from 'bun:sqlite';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const T = Date.parse('2026-04-18T04:53:25.274Z');
const S = T / 1000; // epoch seconds, as state.db stores it

const jsonSession = {
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

/** A state.db / export session (same shape, epoch-second timestamps). */
function dbSession() {
  return {
    id: '20260418_045325_a1b2c3',
    source: 'cli',
    model: 'anthropic/claude-sonnet-4.5',
    system_prompt: 'You are Hermes Agent.',
    parent_session_id: null,
    started_at: S,
    last_active: S + 45,
    title: null,
    cwd: '/work',
    input_tokens: 1500,
    output_tokens: 120,
    cache_read_tokens: 10,
    cache_write_tokens: 0,
    reasoning_tokens: 30,
    estimated_cost_usd: 0.02,
    message_count: 5,
    messages: jsonSession.messages.map((m, index) => ({
      id: index + 1,
      session_id: '20260418_045325_a1b2c3',
      role: m.role,
      content: m.content,
      tool_call_id: (m as Record<string, unknown>).tool_call_id ?? null,
      tool_calls: (m as Record<string, unknown>).tool_calls ? JSON.stringify((m as Record<string, unknown>).tool_calls) : null,
      tool_name: (m as Record<string, unknown>).tool_name ?? null,
      timestamp: S + index,
      finish_reason: (m as Record<string, unknown>).finish_reason ?? null,
      reasoning: (m as Record<string, unknown>).reasoning ?? null,
      reasoning_content: null,
      active: 1,
      _compressed_summary: 0,
      compacted: 0,
    })),
  };
}

const childSession = {
  id: '20260418_050000_child1',
  source: 'cli',
  model: 'anthropic/claude-haiku-4.5',
  system_prompt: 'You are Hermes Agent.',
  parent_session_id: '20260418_045325_a1b2c3',
  started_at: S + 100,
  last_active: S + 120,
  title: 'Subagent: check disk',
  cwd: '/work',
  input_tokens: 200,
  output_tokens: 20,
  cache_read_tokens: 0,
  cache_write_tokens: 0,
  reasoning_tokens: 0,
  estimated_cost_usd: 0.001,
  message_count: 2,
  messages: [
    { id: 1, session_id: '20260418_050000_child1', role: 'user', content: 'Check the disk again.', tool_call_id: null, tool_calls: null, tool_name: null, timestamp: S + 100, finish_reason: null, reasoning: null, reasoning_content: null, active: 1, _compressed_summary: 0, compacted: 0 },
    { id: 2, session_id: '20260418_050000_child1', role: 'assistant', content: 'Still 60G free.', tool_call_id: null, tool_calls: null, tool_name: null, timestamp: S + 101, finish_reason: 'stop', reasoning: 'Nothing changed.', reasoning_content: null, active: 1, _compressed_summary: 0, compacted: 0 },
  ],
};

/** A superseded/compressed row that must never be imported. */
const compressedRow = {
  id: 99, session_id: '20260418_045325_a1b2c3', role: 'assistant', content: 'COMPRESSED SUMMARY THAT MUST NOT APPEAR',
  tool_call_id: null, tool_calls: null, tool_name: null, timestamp: S, finish_reason: null, reasoning: null,
  reasoning_content: null, active: 1, _compressed_summary: 1, compacted: 1,
};

const SESSION_COLUMNS = ['id', 'source', 'model', 'system_prompt', 'parent_session_id', 'started_at', 'last_active', 'title', 'cwd', 'input_tokens', 'output_tokens', 'cache_read_tokens', 'cache_write_tokens', 'reasoning_tokens', 'estimated_cost_usd', 'message_count', 'hidden', 'archived'];
const MESSAGE_COLUMNS = ['id', 'session_id', 'role', 'content', 'tool_call_id', 'tool_calls', 'tool_name', 'timestamp', 'finish_reason', 'reasoning', 'reasoning_content', 'active', '_compressed_summary', 'compacted'];

function createStateDb(path: string): void {
  const db = new Database(path);
  db.exec(`CREATE TABLE sessions (
      id text, source text, model text, system_prompt text, parent_session_id text,
      started_at real, last_active real, title text, cwd text,
      input_tokens real, output_tokens real, cache_read_tokens real, cache_write_tokens real, reasoning_tokens real,
      estimated_cost_usd real, message_count real, hidden real, archived real);
    CREATE TABLE messages (
      id integer, session_id text, role text, content text, tool_call_id text, tool_calls text, tool_name text,
      timestamp real, finish_reason text, reasoning text, reasoning_content text,
      active real, _compressed_summary real, compacted real);`);
  const sessionInsert = db.prepare(`INSERT INTO sessions (${SESSION_COLUMNS.join(', ')}) VALUES (${SESSION_COLUMNS.map(() => '?').join(', ')})`);
  const messageInsert = db.prepare(`INSERT INTO messages (${MESSAGE_COLUMNS.join(', ')}) VALUES (${MESSAGE_COLUMNS.map(() => '?').join(', ')})`);
  const putSession = (session: Record<string, unknown>, hidden: number) => {
    sessionInsert.run(...SESSION_COLUMNS.map(c => (c === 'hidden' ? hidden : c === 'archived' ? 0 : (session[c] ?? null))));
    for (const message of session.messages as Record<string, unknown>[]) {
      messageInsert.run(...MESSAGE_COLUMNS.map(c => message[c] ?? null));
    }
  };
  putSession(dbSession(), 0);
  putSession(childSession, 0);
  // Hidden sessions are excluded from indexing.
  putSession({ ...dbSession(), id: '20260418_999999_hidden', messages: [] }, 1);
  messageInsert.run(...MESSAGE_COLUMNS.map(c => compressedRow[c] ?? null));
  db.close();
}

export const cases: ConformanceCase[] = [
  {
    name: 'session-json-with-tools',
    async setup(home) {
      const dir = join(home, '.hermes', 'sessions');
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, `session_${jsonSession.session_id}.json`), JSON.stringify(jsonSession, null, 2));
      // partial jsonl sibling must be ignored
      await writeFile(join(dir, `${jsonSession.session_id}.jsonl`), JSON.stringify({ role: 'user', content: 'ignored' }) + '\n');
      return dir;
    },
  },
  {
    name: 'state-db-sessions',
    async setup(home) {
      const dir = join(home, '.hermes');
      await mkdir(dir, { recursive: true });
      createStateDb(join(dir, 'state.db'));
      return join(dir, 'state.db');
    },
    check(conversations) {
      const convs = conversations as { externalId: string; parentExternalId?: string; forkType?: string; messages: { role: string; content: string; parts?: Record<string, any>[] }[] }[];
      const main = convs.find(c => c.externalId === '20260418_045325_a1b2c3')!;
      const child = convs.find(c => c.externalId === '20260418_050000_child1')!;
      if (!main || !child) throw new Error('state.db sessions missing');
      if (child.parentExternalId !== main.externalId || child.forkType !== 'thread') throw new Error('parent_session_id not mapped');
      const parts = main.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking' && p.text.includes('run df'))) throw new Error('reasoning column dropped');
      if (!parts.some(p => p.type === 'tool_call' && p.name === 'terminal')) throw new Error('tool_calls JSON not parsed');
      if (!parts.some(p => p.type === 'tool_result' && String(p.output).includes('/dev/sda1'))) throw new Error('tool result missing');
      if (convs.some(c => c.externalId === '20260418_999999_hidden')) throw new Error('hidden session imported');
      if (main.messages.some(m => m.content.includes('MUST NOT APPEAR'))) throw new Error('_compressed_summary row imported');
    },
  },
  {
    name: 'export-jsonl',
    detection: 'content',
    async setup(home) {
      const dir = join(home, 'exports');
      await mkdir(dir, { recursive: true });
      const lines = [dbSession(), childSession].map(session => JSON.stringify(session)).join('\n') + '\n';
      await writeFile(join(dir, 'hermes-sessions.jsonl'), lines);
      return join(dir, 'hermes-sessions.jsonl');
    },
    check(conversations) {
      const convs = conversations as { externalId: string; parentExternalId?: string; messages: { parts?: Record<string, any>[] }[] }[];
      if (convs.length !== 2) throw new Error(`expected 2 exported sessions, got ${convs.length}`);
      const main = convs.find(c => c.externalId === '20260418_045325_a1b2c3')!;
      if (!main.messages.flatMap(m => m.parts ?? []).some(p => p.type === 'tool_call')) throw new Error('export tool calls dropped');
      if (!convs.some(c => c.parentExternalId === '20260418_045325_a1b2c3')) throw new Error('export parent link dropped');
    },
  },
];
