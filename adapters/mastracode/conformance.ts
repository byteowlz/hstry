// Format checked against Mastra's storage reference (mastra.ai/reference/storage/overview, 2026-10):
// tables mastra_threads(id,resourceId,title,metadata,createdAt,updatedAt) and
// mastra_messages(id,thread_id,content,role user|assistant,type,createdAt,resourceId) where content is
// MastraMessageContentV2 JSON {format:2, parts:[text|reasoning|tool-invocation|file|step-start...]}, and
// Mastra Code's data dir (code.mastra.ai/configuration): ~/.local/share/mastracode/mastra.db (LibSQL).
// Mastra Code's own thread metadata keys (the adapter reads projectPath/cwd) are not documented publicly.
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { Database } from 'bun:sqlite';
import type { ConformanceCase } from '../conformance.ts';

export const cases: ConformanceCase[] = [{
  name: 'threads-with-tools',
  async setup(home) {
    const dir = join(home, '.local', 'share', 'mastracode');
    await mkdir(dir, { recursive: true });
    const db = new Database(join(dir, 'mastra.db'));
    db.run('CREATE TABLE mastra_threads (id TEXT PRIMARY KEY, resourceId TEXT NOT NULL, title TEXT NOT NULL, metadata TEXT, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL)');
    db.run('CREATE TABLE mastra_messages (id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, content TEXT NOT NULL, role TEXT NOT NULL, type TEXT, createdAt TEXT NOT NULL, resourceId TEXT)');
    db.run("INSERT INTO mastra_threads VALUES ('thread-1', 'res-1', 'Inspect the repo', ?, '2026-09-01T10:00:00.000Z', '2026-09-01T10:00:20.000Z')", [JSON.stringify({ projectPath: '/work/proj' })]);
    db.run("INSERT INTO mastra_threads VALUES ('thread-2', 'res-1', 'Quick question', NULL, '2026-09-02T10:00:00.000Z', '2026-09-02T10:00:05.000Z')");
    const msg = (id: string, thread: string, role: string, createdAt: string, parts: unknown[]) =>
      db.run('INSERT INTO mastra_messages VALUES (?, ?, ?, ?, ?, ?, ?)', [id, thread, JSON.stringify({ format: 2, parts }), role, 'v2', createdAt, 'res-1']);
    msg('m1', 'thread-1', 'user', '2026-09-01T10:00:01.000Z', [{ type: 'text', text: 'What is in src/?' }]);
    msg('m2', 'thread-1', 'assistant', '2026-09-01T10:00:05.000Z', [
      { type: 'step-start' },
      { type: 'reasoning', reasoning: 'List the directory.', details: [{ type: 'text', text: 'List the directory.' }] },
      { type: 'text', text: 'Listing src/.' },
      { type: 'tool-invocation', toolInvocation: { state: 'result', toolCallId: 'call_a', toolName: 'list_files', args: { path: 'src' }, result: ['main.ts', 'util.ts'] } },
      { type: 'tool-invocation', toolInvocation: { state: 'result', toolCallId: 'call_b', toolName: 'read_file', args: { path: 'src/gone.ts' }, result: { error: 'ENOENT: src/gone.ts' } } },
    ]);
    msg('m3', 'thread-1', 'assistant', '2026-09-01T10:00:20.000Z', [{ type: 'text', text: 'src/ has main.ts and util.ts.' }]);
    msg('m4', 'thread-2', 'user', '2026-09-02T10:00:01.000Z', [{ type: 'text', text: 'ping' }]);
    msg('m5', 'thread-2', 'assistant', '2026-09-02T10:00:05.000Z', [{ type: 'text', text: 'pong' }]);
    db.close();
    return dir;
  },
}];
