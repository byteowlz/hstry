// Format checked against Devin CLI's SQLite store as described in getagentseal/codeburn docs/providers/devin.md
// (2026-10): ~/.local/share/devin/cli/sessions.db with sessions(id,title,working_directory,model,created_at,
// last_activity_at,hidden,main_chain_id,agent_mode...) and message_nodes(session_id,node_id,parent_node_id,
// chat_message JSON,created_at); chat_message.tool_calls[].name and metadata.is_user_input are documented.
// NOT confirmed publicly: the tool-result message shape (role 'tool' + tool_call_id) and epoch unit of
// created_at (seconds assumed; the adapter also accepts ms) — both follow what the adapter reads.
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { Database } from 'bun:sqlite';
import type { ConformanceCase } from '../conformance.ts';

export const cases: ConformanceCase[] = [{
  name: 'main-chain-with-tools',
  async setup(home) {
    const dir = join(home, '.local', 'share', 'devin', 'cli');
    await mkdir(dir, { recursive: true });
    const db = new Database(join(dir, 'sessions.db'));
    db.run('CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, working_directory TEXT, model TEXT, created_at INTEGER, last_activity_at INTEGER, agent_mode TEXT, main_chain_id INTEGER, hidden INTEGER NOT NULL DEFAULT 0)');
    db.run('CREATE TABLE message_nodes (session_id TEXT, node_id INTEGER, parent_node_id INTEGER, chat_message TEXT, created_at INTEGER, PRIMARY KEY (session_id, node_id))');
    db.run("INSERT INTO sessions VALUES ('dev-1', 'Run the linter', '/work/proj', 'swe-1-5', 1790000000, 1790000030, 'normal', 5, 0)");
    db.run("INSERT INTO sessions VALUES ('dev-2', 'Hidden helper', '/work/proj', 'swe-1-5', 1790000100, 1790000110, 'normal', 1, 1)");
    const node = (session: string, id: number, parent: number | null, at: number, message: unknown) =>
      db.run('INSERT INTO message_nodes VALUES (?, ?, ?, ?, ?)', [session, id, parent, JSON.stringify(message), at]);
    node('dev-1', 1, null, 1790000001, { role: 'user', content: 'Run the linter', metadata: { is_user_input: true } });
    node('dev-1', 2, 1, 1790000005, { role: 'assistant', content: 'Running eslint.', tool_calls: [{ id: 'call_1', name: 'exec', arguments: { command: 'eslint .' } }] });
    node('dev-1', 3, 2, 1790000010, { role: 'tool', tool_call_id: 'call_1', content: 'eslint: command not found' });
    // retry sibling off the main chain (same parent as node 4): must not appear
    node('dev-1', 6, 3, 1790000012, { role: 'assistant', content: 'discarded retry' });
    node('dev-1', 4, 3, 1790000015, { role: 'assistant', content: 'eslint is not installed; installing it.' , tool_calls: [{ id: 'call_2', name: 'exec', arguments: { command: 'npm i -D eslint' } }] });
    node('dev-1', 5, 4, 1790000030, { role: 'assistant', content: 'Done.' });
    node('dev-2', 1, null, 1790000101, { role: 'user', content: 'hidden' });
    db.close();
    return dir;
  },
}];
