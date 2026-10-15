// Format checked against Cursor 2.x/3.x local storage (2026): globalStorage/state.vscdb
// table cursorDiskKV with `composerData:<id>` (name, createdAt ms,
// fullConversationHeadersOnly[{bubbleId,type 1=user|2=assistant}]) and
// `bubbleId:<composerId>:<bubbleId>` (text, createdAt ISO, one
// toolFormerData{name,params,result,status} per assistant bubble), plus
// ItemTable `composer.composerHeaders`. Sources: cursor-logger README,
// toolpath-cursor docs, vibe-replay "Where does Cursor store chat history".
// Legacy workspaceStorage/<hash>/state.vscdb aichat chatdata is covered too.
import { Database } from 'bun:sqlite';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

function userDir(home: string): string {
  return join(home, '.config', 'Cursor', 'User');
}

export const cases: ConformanceCase[] = [
  {
    name: 'global-composer-with-tools',
    async setup(home) {
      const dir = join(userDir(home), 'globalStorage');
      await mkdir(dir, { recursive: true });
      const db = new Database(join(dir, 'state.vscdb'));
      db.run('CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)');
      db.run('CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)');
      const put = (table: string, key: string, value: unknown) =>
        db.run(`INSERT INTO ${table} (key, value) VALUES (?, ?)`, [key, JSON.stringify(value)]);

      put('ItemTable', 'composer.composerHeaders', { allComposers: [
        { type: 'head', composerId: 'comp-1', name: 'Read the config', createdAt: 1750000000000, lastUpdatedAt: 1750000060000,
          workspaceIdentifier: { id: 'ws1', uri: { fsPath: '/work/demo', path: '/work/demo' } } },
      ] });
      put('cursorDiskKV', 'composerData:comp-1', {
        _v: 10, composerId: 'comp-1', name: 'Read the config', createdAt: 1750000000000, lastUpdatedAt: 1750000060000,
        fullConversationHeadersOnly: [
          { bubbleId: 'b1', type: 1 }, { bubbleId: 'b2', type: 2 }, { bubbleId: 'b3', type: 2 },
          { bubbleId: 'b4', type: 2 }, { bubbleId: 'b5', type: 2 },
        ],
      });
      put('cursorDiskKV', 'bubbleId:comp-1:b1', { _v: 3, type: 1, bubbleId: 'b1', text: 'What is in config.toml?', createdAt: '2025-06-15T15:06:40.000Z' });
      put('cursorDiskKV', 'bubbleId:comp-1:b2', { _v: 3, type: 2, bubbleId: 'b2', text: '', thinking: { text: 'I should read the file.' }, createdAt: '2025-06-15T15:06:41.000Z' });
      put('cursorDiskKV', 'bubbleId:comp-1:b3', { _v: 3, type: 2, bubbleId: 'b3', text: '', createdAt: '2025-06-15T15:06:42.000Z',
        toolFormerData: { tool: 5, toolCallId: 'call_1', status: 'completed', name: 'read_file_v2',
          rawArgs: '{"targetFile":"config.toml"}', params: '{"targetFile":"config.toml"}', result: '{"contents":"port = 8080"}' } });
      put('cursorDiskKV', 'bubbleId:comp-1:b4', { _v: 3, type: 2, bubbleId: 'b4', text: '', createdAt: '2025-06-15T15:06:43.000Z',
        toolFormerData: { tool: 5, toolCallId: 'call_2', status: 'error', name: 'read_file_v2',
          rawArgs: '{"targetFile":"missing.toml"}', params: '{"targetFile":"missing.toml"}', result: '{"error":"File not found"}' } });
      put('cursorDiskKV', 'bubbleId:comp-1:b5', { _v: 3, type: 2, bubbleId: 'b5', text: 'The config sets port = 8080.', createdAt: '2025-06-15T15:07:40.000Z' });

      put('cursorDiskKV', 'composerData:comp-2', {
        _v: 10, composerId: 'comp-2', createdAt: 1750100000000, lastUpdatedAt: 1750100005000,
        fullConversationHeadersOnly: [{ bubbleId: 'c1', type: 1 }, { bubbleId: 'c2', type: 2 }],
      });
      put('cursorDiskKV', 'bubbleId:comp-2:c1', { _v: 3, type: 1, bubbleId: 'c1', text: 'Say hi', createdAt: '2025-06-16T19:33:20.000Z' });
      put('cursorDiskKV', 'bubbleId:comp-2:c2', { _v: 3, type: 2, bubbleId: 'c2', text: 'Hi!', createdAt: '2025-06-16T19:33:25.000Z' });
      db.close();
      return dir;
    },
  },
  {
    name: 'legacy-workspace-chatdata',
    async setup(home) {
      const dir = join(userDir(home), 'workspaceStorage', 'abc123hash');
      await mkdir(dir, { recursive: true });
      const db = new Database(join(dir, 'state.vscdb'));
      db.run('CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)');
      db.run('INSERT INTO ItemTable (key, value) VALUES (?, ?)', ['workbench.panel.aichat.view.aichat.chatdata', JSON.stringify({
        tabs: [{
          tabId: 'tab-1', id: 'tab-1', title: 'Explain this', createdAt: 1700000000000, lastUpdatedAt: 1700000020000,
          bubbles: [
            { type: 'user', text: 'Explain this function', timingInfo: { startTime: 1700000001000 } },
            { type: 'ai', text: 'It adds two numbers.', modelType: 'gpt-4', timingInfo: { startTime: 1700000010000 } },
          ],
        }],
      })]);
      db.close();
      return join(userDir(home), 'workspaceStorage');
    },
  },
];
