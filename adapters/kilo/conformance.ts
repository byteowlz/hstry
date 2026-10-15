// Kilo Code CLI is an OpenCode fork and keeps OpenCode's storage model. Checked against the layouts documented
// in adapters/kilo/adapter.ts (kilo.db SQLite v1.1.45+; storage/{session,message,part} JSON v1.1.25+) and
// OpenCode's message/part shapes: message {role,time{created,completed},modelID,providerID,agent,cost,tokens},
// part {type:text|reasoning|tool|step-start|step-finish|patch}; tool part state
// {status:'completed',input,output,title} or {status:'error',input,error}. kilo.ai docs do not publish the
// on-disk schema, so column sets here are the minimal ones the adapter queries.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { Database } from 'bun:sqlite';
import type { ConformanceCase } from '../conformance.ts';

const sessionTime = { created: 1_790_000_000_000, updated: 1_790_000_060_000 };

// [message id, data, parts[]] shared by both layouts
const messages: [string, Record<string, unknown>, Record<string, unknown>[]][] = [
  ['msg_001', { role: 'user', time: { created: 1_790_000_001_000 }, agent: 'code' }, [
    { type: 'text', text: 'Read package.json and the missing file' },
  ]],
  ['msg_002', { role: 'assistant', time: { created: 1_790_000_002_000, completed: 1_790_000_009_000 }, parentID: 'msg_001', modelID: 'claude-sonnet-4-5', providerID: 'anthropic', agent: 'code', cost: 0.01, tokens: { input: 100, output: 50, reasoning: 10 } }, [
    { type: 'step-start' },
    { type: 'reasoning', text: 'Need two reads.' },
    { type: 'text', text: 'Reading both files.' },
    { type: 'tool', callID: 'call_1', tool: 'read', state: { status: 'completed', input: { filePath: '/work/proj/package.json' }, output: '{"name":"proj"}', title: 'package.json' } },
    { type: 'tool', callID: 'call_2', tool: 'read', state: { status: 'error', input: { filePath: '/work/proj/missing.txt' }, error: 'File not found: /work/proj/missing.txt' } },
    { type: 'step-finish' },
  ]],
  ['msg_003', { role: 'assistant', time: { created: 1_790_000_010_000 }, modelID: 'claude-sonnet-4-5', providerID: 'anthropic' }, [
    { type: 'text', text: 'package.json is named proj; missing.txt does not exist.' },
  ]],
];

export const cases: ConformanceCase[] = [{
  name: 'sqlite',
  async setup(home) {
    const base = join(home, '.local', 'share', 'kilo');
    await mkdir(base, { recursive: true });
    const db = new Database(join(base, 'kilo.db'));
    db.run('CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT NOT NULL)');
    db.run(`CREATE TABLE session (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT, slug TEXT NOT NULL, directory TEXT NOT NULL,
      title TEXT NOT NULL, version TEXT NOT NULL, summary_additions INTEGER, summary_deletions INTEGER, summary_files INTEGER,
      time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL)`);
    db.run('CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL)');
    db.run('CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL)');
    db.run("INSERT INTO project VALUES ('proj1', '/work/proj')");
    db.run("INSERT INTO session VALUES ('ses_aaa', 'proj1', NULL, 'quiet-fox', '/work/proj', 'Read two files', '1.1.50', 3, 1, 1, ?, ?)", [sessionTime.created, sessionTime.updated]);
    db.run("INSERT INTO session VALUES ('ses_bbb', 'proj1', NULL, 'calm-owl', '/work/proj', 'Second session', '1.1.50', NULL, NULL, NULL, ?, ?)", [sessionTime.created + 500_000, sessionTime.updated + 500_000]);
    let partSeq = 0;
    for (const [id, data, parts] of messages) {
      db.run('INSERT INTO message VALUES (?, ?, ?, ?, ?)', [id, 'ses_aaa', (data.time as { created: number }).created, (data.time as { created: number }).created, JSON.stringify(data)]);
      for (const part of parts) {
        const partId = `prt_${String(++partSeq).padStart(3, '0')}`;
        db.run('INSERT INTO part VALUES (?, ?, ?, ?, ?, ?)', [partId, id, 'ses_aaa', sessionTime.created, sessionTime.created, JSON.stringify(part)]);
      }
    }
    db.run("INSERT INTO message VALUES ('msg_101', 'ses_bbb', ?, ?, ?)", [sessionTime.created + 501_000, sessionTime.created + 501_000, JSON.stringify({ role: 'user', time: { created: sessionTime.created + 501_000 } })]);
    db.run("INSERT INTO part VALUES ('prt_101', 'msg_101', 'ses_bbb', ?, ?, ?)", [sessionTime.created, sessionTime.created, JSON.stringify({ type: 'text', text: 'ping' })]);
    db.close();
    return base;
  },
}, {
  name: 'storage-json',
  async setup(home) {
    const base = join(home, '.local', 'share', 'kilo');
    const storage = join(base, 'storage');
    await mkdir(join(storage, 'session', 'proj1'), { recursive: true });
    await writeFile(join(storage, 'session', 'proj1', 'ses_aaa.json'), JSON.stringify({
      id: 'ses_aaa', version: '1.1.30', projectID: 'proj1', directory: '/work/proj', title: 'Read two files', time: sessionTime,
    }));
    await mkdir(join(storage, 'message', 'ses_aaa'), { recursive: true });
    let partSeq = 0;
    for (const [id, data, parts] of messages) {
      await writeFile(join(storage, 'message', 'ses_aaa', `${id}.json`), JSON.stringify({ id, sessionID: 'ses_aaa', ...data }));
      await mkdir(join(storage, 'part', id), { recursive: true });
      for (const part of parts) {
        const partId = `prt_${String(++partSeq).padStart(3, '0')}`;
        await writeFile(join(storage, 'part', id, `${partId}.json`), JSON.stringify({ id: partId, messageID: id, sessionID: 'ses_aaa', ...part }));
      }
    }
    return base;
  },
}];
