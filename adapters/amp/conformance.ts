// Format checked against the ampcode crate's thread schema (docs.rs/ampcode, types/thread.rs,
// "matches ~/.local/share/amp/threads/T-<uuid>.json") and the Amp streaming-json docs (2026-10).
// Thread files: {v,id,created(ms),title,agentMode,messages[{role,messageId,content[blocks],usage}],nextMessageId};
// blocks: text | thinking | tool_use{id,name,input} | tool_result{toolUseID,run:{status,result|error}}.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const thread = {
  v: 6,
  id: 'T-0a1b2c3d-0000-4000-8000-000000000001',
  created: 1_790_000_000_000,
  title: 'List project files',
  agentMode: 'smart',
  nextMessageId: 6,
  env: { initial: { trees: [{ displayName: 'proj', uri: 'file:///work/proj' }] } },
  messages: [
    { role: 'user', messageId: 0, content: [{ type: 'text', text: 'List the files in /work/proj' }] },
    {
      role: 'assistant', messageId: 1,
      content: [
        { type: 'thinking', thinking: 'I should read the directory.' },
        { type: 'text', text: 'Reading the directory.' },
        { type: 'tool_use', id: 'toolu_01', name: 'Read', input: { path: '/work/proj' } },
      ],
      usage: { model: 'claude-opus-4-6', inputTokens: 10, outputTokens: 20 },
      state: { type: 'complete', stopReason: 'tool_use' },
    },
    { role: 'user', messageId: 2, content: [{ type: 'tool_result', toolUseID: 'toolu_01', run: { status: 'done', result: ['index.js', 'README.md'] } }] },
    {
      role: 'assistant', messageId: 3,
      content: [{ type: 'tool_use', id: 'toolu_02', name: 'Read', input: { path: '/work/proj/missing' } }],
      usage: { model: 'claude-opus-4-6', inputTokens: 12, outputTokens: 8 },
    },
    { role: 'user', messageId: 4, content: [{ type: 'tool_result', toolUseID: 'toolu_02', run: { status: 'error', error: { message: 'ENOENT: no such file' } } }] },
    {
      role: 'assistant', messageId: 5,
      content: [{ type: 'text', text: 'The project has index.js and README.md; the second path does not exist.' }],
      usage: { model: 'claude-opus-4-6', inputTokens: 15, outputTokens: 18 },
    },
  ],
};

export const cases: ConformanceCase[] = [{
  name: 'thread-with-tools',
  async setup(home) {
    const dir = join(home, '.local', 'share', 'amp', 'threads');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, `${thread.id}.json`), JSON.stringify(thread));
    return join(home, '.local', 'share', 'amp');
  },
}];
