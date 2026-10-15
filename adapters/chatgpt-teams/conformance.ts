// The "ChatGPT Teams browser export" is the Open WebUI chat export shape
// (chat.history.messages tree with currentId, per-message `timestamp` in
// seconds, `model`/`models`), checked against Open WebUI v0.6.x chat export.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const exportData = [
  {
    id: 'chat-0001', user_id: 'user-1', title: 'Greeting', created_at: 1750000000, updated_at: 1750000300,
    archived: false, pinned: false, folder_id: null,
    chat: {
      timestamp: 1750000000, models: ['gpt-4o'],
      history: {
        currentId: 'm4',
        messages: {
          m1: { id: 'm1', parentId: null, childrenIds: ['m2', 'm3'], role: 'user', content: 'Hello Teams!', timestamp: 1750000010, models: ['gpt-4o'] },
          m2: { id: 'm2', parentId: 'm1', childrenIds: [], role: 'assistant', content: 'DISCARDED regenerated reply', model: 'gpt-4o', modelName: 'gpt-4o', timestamp: 1750000020, done: true },
          m3: { id: 'm3', parentId: 'm1', childrenIds: ['m4'], role: 'assistant', content: 'Hello from Teams.', model: 'gpt-4o', modelName: 'gpt-4o', timestamp: 1750000030, done: true },
          m4: { id: 'm4', parentId: 'm3', childrenIds: [], role: 'user', content: 'Thanks', timestamp: 1750000040, models: ['gpt-4o'] },
        },
      },
    },
  },
  {
    id: 'chat-0002', user_id: 'user-1', title: 'Second chat', created_at: 1750001000, updated_at: 1750001100,
    chat: {
      models: ['gpt-4o-mini'],
      history: {
        currentId: 'b',
        messages: {
          a: { id: 'a', parentId: null, childrenIds: ['b'], role: 'user', content: 'Ping', timestamp: 1750001001 },
          b: { id: 'b', parentId: 'a', childrenIds: [], role: 'assistant', content: 'Pong', model: 'gpt-4o-mini', timestamp: 1750001002, done: true },
        },
      },
    },
  },
];

export const cases: ConformanceCase[] = [{
  name: 'webui-chat-export',
  detection: 'content',
  async setup(home) {
    const dir = join(home, 'Downloads');
    await mkdir(dir, { recursive: true });
    const file = join(dir, 'history_20250101_120000.json');
    await writeFile(file, JSON.stringify(exportData));
    return file;
  },
}];
