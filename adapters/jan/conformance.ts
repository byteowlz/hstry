// Format checked against the Jan docs (jan.ai/docs/desktop/data-folder, 2026):
// <data folder>/threads/<id>/{thread.json,messages.jsonl}; the Linux data
// folder is ~/.local/share/Jan/data. thread.json: {id,object:'thread',title,
// created,updated,assistants[{id:'jan',model:'*'}],model:{id,provider},metadata};
// messages.jsonl: OpenAI-style thread.message with ULID ids, created_at in ms,
// content[{type:'text',text:{value,annotations}}], metadata.tokenSpeed.
// Tool-call encoding in current Jan could not be verified, so it is not covered.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const message = (id: string, thread: string, role: string, value: string, createdAt: number, metadata: unknown = {}) => ({
  completed_at: 0, content: [{ text: { annotations: [], value }, type: 'text' }], created_at: createdAt,
  id, metadata, object: 'thread.message', role, status: 'ready', thread_id: thread, type: 'text',
});

export const cases: ConformanceCase[] = [{
  name: 'threads-two-threads',
  async setup(home) {
    const threads = join(home, '.local', 'share', 'Jan', 'data', 'threads');
    const t1 = '8f2c9922-db49-4d1e-8620-279c05baf2d0';
    const t2 = 'jan_1751100000';
    await mkdir(join(threads, t1), { recursive: true });
    await mkdir(join(threads, t2), { recursive: true });
    await writeFile(join(threads, t1, 'thread.json'), JSON.stringify({
      assistants: [{ id: 'jan', object: 'assistant', name: 'Jan', model: '*', created_at: 1750945742.536 }],
      created: 1751012633, id: t1, metadata: {}, model: { id: 'jan-v1-4b', provider: 'llamacpp' },
      object: 'thread', title: 'Hello Jan', updated: 1751012640,
    }));
    await writeFile(join(threads, t1, 'messages.jsonl'), [
      message('01JYR7S0GW5M9PSHMRE7T8VQJM', t1, 'user', 'What can you do?', 1751012635000),
      message('01JYR7S0JB5ZBGMJV52KWMW5VW', t1, 'assistant', 'I can search and summarize.', 1751012639307,
        { tokenSpeed: { lastTimestamp: 1751012637097, message: '01JYR7S0GW5M9PSHMRE7T8VQJM', tokenCount: 49, tokenSpeed: 22.6 } }),
    ].map(m => JSON.stringify(m)).join('\n') + '\n');
    await writeFile(join(threads, t2, 'thread.json'), JSON.stringify({
      assistants: [], created: 1751100000, id: t2, metadata: {}, object: 'thread', title: 'New Thread', updated: 1751100005,
    }));
    await writeFile(join(threads, t2, 'messages.jsonl'), [
      message('01JYSAAAAAAAAAAAAAAAAAAAAA', t2, 'user', 'Ping', 1751100001000),
      message('01JYSBBBBBBBBBBBBBBBBBBBBB', t2, 'assistant', 'Pong', 1751100004000),
    ].map(m => JSON.stringify(m)).join('\n') + '\n');
    return threads;
  },
}];
