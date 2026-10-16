// Format checked against open-webui/open-webui main (package.json 0.12.2), backend/open_webui/models/chats.py:
// table `chat` (id, user_id, title, chat JSON, created_at/updated_at epoch seconds, share_id, archived,
// pinned, meta, folder_id, ...). Current chats store the message tree in chat.history.messages{id: msg}
// with history.currentId; chat.messages (flat array) is the legacy shape, still read by upstream.
// Assistant messages may also carry an `output` item list; the visible text stays in `content`.
import { Database } from 'bun:sqlite';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

function createDb(path: string, chats: Record<string, unknown>[]): void {
  const db = new Database(path);
  db.exec(`CREATE TABLE auth (id VARCHAR(255) PRIMARY KEY, email VARCHAR(255), password TEXT, active INTEGER);
    CREATE TABLE chat (id VARCHAR(255) PRIMARY KEY, user_id VARCHAR(255), title TEXT, chat JSON, created_at BIGINT, updated_at BIGINT,
      share_id VARCHAR(255), archived BOOLEAN, pinned BOOLEAN, meta JSON, folder_id VARCHAR(255));`);
  const insert = db.prepare('INSERT INTO chat VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)');
  for (const chat of chats) {
    insert.run(chat.id as string, 'user-1', chat.title as string, JSON.stringify(chat.chat), chat.created_at as number, chat.updated_at as number, null, 0, chat.pinned ? 1 : 0, JSON.stringify(chat.meta ?? {}), null);
  }
  db.close();
}

async function dataDir(home: string): Promise<string> {
  const dir = join(home, '.open-webui', 'data');
  await mkdir(dir, { recursive: true });
  return dir;
}

const t0 = 1790000000;

export const cases: ConformanceCase[] = [
  {
    name: 'history-tree',
    detection: 'content',
    async setup(home) {
      const dir = await dataDir(home);
      createDb(join(dir, 'webui.db'), [
        {
          id: '7c1d7a2e-0001-4a55-9a10-aaaaaaaaaaaa', title: 'Explain Rust lifetimes', created_at: t0, updated_at: t0 + 30, pinned: true, meta: { tags: ['rust'] },
          chat: {
            id: '', title: 'Explain Rust lifetimes', models: ['llama3.3:70b'], params: {}, tags: [], timestamp: t0 * 1000,
            history: {
              currentId: 'm-asst-1',
              messages: {
                'm-user-1': { id: 'm-user-1', parentId: null, childrenIds: ['m-asst-1'], role: 'user', content: 'Explain Rust lifetimes in one paragraph', timestamp: t0 + 1, models: ['llama3.3:70b'] },
                'm-asst-1': {
                  id: 'm-asst-1', parentId: 'm-user-1', childrenIds: [], role: 'assistant', content: 'Lifetimes tell the compiler how long references stay valid.',
                  model: 'llama3.3:70b', modelName: 'Llama 3.3 70B', modelIdx: 0, timestamp: t0 + 5, done: true,
                  usage: { prompt_tokens: 12, completion_tokens: 11 }, output: [{ type: 'message', id: 'o1', status: 'completed', role: 'assistant', content: [{ type: 'output_text', text: 'Lifetimes tell the compiler how long references stay valid.' }] }],
                },
              },
            },
          },
        },
        {
          id: '7c1d7a2e-0002-4a55-9a10-bbbbbbbbbbbb', title: 'Second chat', created_at: t0 + 100, updated_at: t0 + 120,
          chat: {
            title: 'Second chat', models: ['gpt-4.1'],
            history: {
              currentId: 'b2',
              messages: {
                b1: { id: 'b1', parentId: null, childrenIds: ['b2'], role: 'user', content: 'Say hi', timestamp: t0 + 101 },
                b2: { id: 'b2', parentId: 'b1', childrenIds: [], role: 'assistant', content: 'Hi!', model: 'gpt-4.1', timestamp: t0 + 103, done: true },
              },
            },
          },
        },
      ]);
      return dir;
    },
  },
  {
    name: 'legacy-messages-array',
    detection: 'content',
    async setup(home) {
      const dir = await dataDir(home);
      createDb(join(dir, 'webui.db'), [{
        id: 'legacy-0001', title: 'Legacy chat', created_at: t0, updated_at: t0 + 10,
        chat: { models: ['mistral'], messages: [
          { id: 'l1', role: 'user', content: 'Hello', timestamp: t0 + 1 },
          { id: 'l2', role: 'assistant', content: 'Hello! How can I help?', model: 'mistral', timestamp: t0 + 2, done: true },
        ] },
      }]);
      return dir;
    },
  },
  {
    name: 'branched-regeneration-with-tools',
    detection: 'content',
    async setup(home) {
      const dir = await dataDir(home);
      createDb(join(dir, 'webui.db'), [{
        id: 'branch-0001', title: 'Regenerated with tools', created_at: t0, updated_at: t0 + 20,
        chat: {
          models: ['gpt-5'],
          history: {
            currentId: 'a2b',
            messages: {
              u1: { id: 'u1', parentId: null, childrenIds: ['a2a', 'a2b'], role: 'user', content: 'Run the tool', timestamp: t0 + 1 },
              a2a: { id: 'a2a', parentId: 'u1', childrenIds: [], role: 'assistant', content: 'DISCARDED ANSWER', timestamp: t0 + 2, done: true },
              a2b: {
                id: 'a2b', parentId: 'u1', childrenIds: [], role: 'assistant', content: '', model: 'gpt-5', timestamp: t0 + 3, done: true,
                output: [
                  { type: 'reasoning', id: 'r1', status: 'completed', summary: [{ type: 'summary_text', text: 'I should call the tool.' }] },
                  { type: 'function_call', id: 'fc1', call_id: 'call_1', name: 'run_shell', arguments: '{"command":"echo hi"}' },
                  { type: 'function_call_output', call_id: 'call_1', status: 'completed', output: [{ type: 'output_text', text: 'hi' }] },
                  { type: 'message', id: 'm1', status: 'completed', content: [{ type: 'output_text', text: 'The tool printed hi.' }] },
                ],
              },
            },
          },
        },
      }]);
      return dir;
    },
    check(conversations) {
      const [conversation] = conversations as { messages: { content: string; parts?: Record<string, any>[] }[] }[];
      const all = conversation.messages.map(m => m.content).join('\n');
      if (all.includes('DISCARDED ANSWER')) throw new Error('regenerated sibling branch was imported');
      const parts = conversation.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking' && p.text.includes('call the tool'))) throw new Error('reasoning output item dropped');
      const call = parts.find(p => p.type === 'tool_call' && p.toolCallId === 'call_1');
      if (!call || (call.input as { command?: string })?.command !== 'echo hi') throw new Error('function_call output item dropped or arguments unparsed');
      if (!parts.some(p => p.type === 'tool_result' && p.toolCallId === 'call_1' && String(p.output).includes('hi'))) throw new Error('function_call_output not associated');
      if (!conversation.messages.some(m => m.content.includes('tool printed hi'))) throw new Error('assistant message text missing');
    },
  },
];
