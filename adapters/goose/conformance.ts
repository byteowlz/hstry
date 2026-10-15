// Format checked against block/goose main crates/goose/src/session/
// session_manager.rs (schema v17, 2026): ~/.local/share/goose/sessions/sessions.db
// with `sessions` (id, name, description, working_dir, created_at/updated_at as
// UTC "YYYY-MM-DD HH:MM:SS") and `messages` (message_id, session_id, role,
// content_json, created_timestamp seconds). content_json is an array of
// camelCase-tagged MessageContent: text, thinking, toolRequest{id,
// toolCall:{status,value:{name,arguments}}}, toolResponse{id,toolResult:
// {status,value}}; tool responses are stored under role "user".
// The legacy ~/.local/share/goose/sessions/*.jsonl format (metadata line then
// {role,created,content[]} lines) is covered separately.
import { Database } from 'bun:sqlite';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

function sessionsDir(home: string): string {
  return join(home, '.local', 'share', 'goose', 'sessions');
}

const text = (t: string) => ({ type: 'text', text: t });

export const cases: ConformanceCase[] = [
  {
    name: 'sqlite-sessions-with-tools',
    async setup(home) {
      const dir = sessionsDir(home);
      await mkdir(dir, { recursive: true });
      const db = new Database(join(dir, 'sessions.db'));
      db.run(`CREATE TABLE sessions (
        id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '',
        user_set_name BOOLEAN DEFAULT FALSE, session_type TEXT NOT NULL DEFAULT 'user',
        working_dir TEXT NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, extension_data TEXT DEFAULT '{}')`);
      db.run(`CREATE TABLE messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT NOT NULL REFERENCES sessions(id),
        role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL,
        timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, tokens INTEGER, metadata_json TEXT)`);
      db.run('INSERT INTO sessions (id, name, working_dir, created_at, updated_at) VALUES (?,?,?,?,?)',
        ['20250601_1', 'List files', '/work/demo', '2025-06-01 10:00:00', '2025-06-01 10:00:30']);
      db.run('INSERT INTO sessions (id, name, working_dir, created_at, updated_at) VALUES (?,?,?,?,?)',
        ['20250602_1', 'Greeting', '/work/other', '2025-06-02 08:00:00', '2025-06-02 08:00:05']);
      const msg = (sid: string, mid: string, role: string, content: unknown[], ts: number) =>
        db.run('INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp) VALUES (?,?,?,?,?)',
          [mid, sid, role, JSON.stringify(content), ts]);
      const base = 1748772000; // 2025-06-01T10:00:00Z
      msg('20250601_1', 'm1', 'user', [text('List the files here')], base + 1);
      msg('20250601_1', 'm2', 'assistant', [
        { type: 'thinking', thinking: 'I should run ls.', signature: 'sig' },
        text('Let me check.'),
        { type: 'toolRequest', id: 'call_1', toolCall: { status: 'success', value: { name: 'developer__shell', arguments: { command: 'ls' } } } },
      ], base + 2);
      msg('20250601_1', 'm3', 'user', [
        { type: 'toolResponse', id: 'call_1', toolResult: { status: 'success', value: { content: [{ type: 'text', text: 'a.txt\nb.txt' }], isError: false } } },
      ], base + 3);
      msg('20250601_1', 'm4', 'assistant', [
        { type: 'toolRequest', id: 'call_2', toolCall: { status: 'success', value: { name: 'developer__shell', arguments: { command: 'cat nope' } } } },
      ], base + 4);
      msg('20250601_1', 'm5', 'user', [
        { type: 'toolResponse', id: 'call_2', toolResult: { status: 'error', error: 'cat: nope: No such file or directory' } },
      ], base + 5);
      msg('20250601_1', 'm6', 'assistant', [text('There are two files: a.txt and b.txt.')], base + 30);
      msg('20250602_1', 'n1', 'user', [text('Hello')], 1748851201);
      msg('20250602_1', 'n2', 'assistant', [text('Hi there!')], 1748851205);
      db.close();
      return dir;
    },
  },
  {
    name: 'legacy-jsonl',
    async setup(home) {
      const dir = sessionsDir(home);
      await mkdir(dir, { recursive: true });
      const lines = [
        { working_dir: '/work/legacy', description: 'Legacy chat', message_count: 2 },
        { id: 'a', role: 'user', created: 1700000001, content: [text('Hello Goose!')] },
        { id: 'b', role: 'assistant', created: 1700000002, content: [text('Hello from Goose.')] },
      ];
      await writeFile(join(dir, 'legacy-session.jsonl'), lines.map(l => JSON.stringify(l)).join('\n') + '\n');
      return dir;
    },
  },
];
