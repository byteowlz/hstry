// Format checked against anomalyco/opencode (sst/opencode) @ 4397ee4 (2026-10-15, cli/core 2.0.29):
// packages/schema/src/v1/session.ts (v1 message/part shapes: text, reasoning, tool{callID, tool,
// state pending|running|completed|error(.error), time}, step-start/finish, patch) and
// packages/core/src/database/v1-migration.bun.ts (v1 tables project/session/message/part with JSON
// `data` blobs, ms timestamps). The adapter reads those v1 layouts: SQLite opencode.db (1.1.45+),
// storage/{session,message,part} (1.1.25+) and project/<name>/storage/session/* (legacy).
// NOT covered: opencode 2.x writes session_v2/session_message tables (see trx issue filed).
import { Database } from 'bun:sqlite';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const T = Date.parse('2026-10-03T08:00:00.000Z');
const PROJECT = '9b7a3c5d1e2f4a6b8c0d1e2f3a4b5c6d7e8f9a0b';
const SESSION = 'ses_5f3a1c2d3ffeAbCdEfGhIjKlMn';
const CHILD = 'ses_5f3a1c2d4ffeZyXwVuTsRqPoNm';

interface Msg { id: string; time: number; data: Record<string, unknown>; parts: { id: string; data: Record<string, unknown> }[] }

const tokens = (input: number, output: number, reasoning: number) => ({ input, output, reasoning, cache: { read: 0, write: 0 } });
const assistant = (id: string, parentID: string, time: number, cost: number, tk: ReturnType<typeof tokens>, finish: string) => ({
  role: 'assistant', time: { created: time, completed: time + 2000 }, parentID, modelID: 'claude-sonnet-4-5', providerID: 'anthropic', mode: 'build', agent: 'build',
  path: { cwd: '/work/demo', root: '/work/demo' }, summary: false, cost, tokens: tk, finish,
});

function mainSession(): { session: Record<string, unknown>; messages: Msg[] } {
  const messages: Msg[] = [
    { id: 'msg_a01', time: T + 1000, data: { role: 'user', time: { created: T + 1000 }, agent: 'build', model: { providerID: 'anthropic', modelID: 'claude-sonnet-4-5' }, summary: { title: 'Fix the build', diffs: [] } }, parts: [
      { id: 'prt_a01', data: { type: 'text', text: 'The build is failing, can you fix it?' } },
    ] },
    { id: 'msg_a02', time: T + 3000, data: assistant('msg_a02', 'msg_a01', T + 3000, 0.012, tokens(1500, 120, 40), 'tool-calls'), parts: [
      { id: 'prt_a02', data: { type: 'step-start' } },
      { id: 'prt_a03', data: { type: 'reasoning', text: 'Run the build to see the error.', time: { start: T + 3000, end: T + 3500 } } },
      { id: 'prt_a04', data: { type: 'text', text: 'Let me run the build.' } },
      { id: 'prt_a05', data: { type: 'tool', callID: 'toolu_01', tool: 'bash', state: { status: 'error', input: { command: 'npm run buidl' }, error: 'Missing script: "buidl"', time: { start: T + 4000, end: T + 4100 } } } },
      { id: 'prt_a06', data: { type: 'tool', callID: 'toolu_02', tool: 'bash', state: { status: 'completed', input: { command: 'npm run build' }, output: 'error TS2304: Cannot find name "foo"', title: 'npm run build', metadata: { exit: 2 }, time: { start: T + 4200, end: T + 6000 } } } },
      { id: 'prt_a07', data: { type: 'step-finish', reason: 'tool-calls', cost: 0.012, tokens: tokens(1500, 120, 40) } },
    ] },
    { id: 'msg_a03', time: T + 9000, data: assistant('msg_a03', 'msg_a01', T + 9000, 0.008, tokens(1800, 60, 0), 'stop'), parts: [
      { id: 'prt_a08', data: { type: 'step-start' } },
      { id: 'prt_a09', data: { type: 'text', text: 'Fixed the undefined name in src/index.ts.' } },
      { id: 'prt_a10', data: { type: 'patch', hash: 'abc123', files: ['/work/demo/src/index.ts'] } },
      { id: 'prt_a11', data: { type: 'step-finish', reason: 'stop', cost: 0.008, tokens: tokens(1800, 60, 0) } },
    ] },
  ];
  return { session: { id: SESSION, slug: 'quiet-river', version: '1.2.0', projectID: PROJECT, directory: '/work/demo', title: 'Fix the build', time: { created: T, updated: T + 12000 } }, messages };
}

function childSession(): { session: Record<string, unknown>; messages: Msg[] } {
  return {
    session: { id: CHILD, slug: 'brave-owl', version: '1.2.0', projectID: PROJECT, parentID: SESSION, directory: '/work/demo', title: 'Subagent: check types', time: { created: T + 5000, updated: T + 8000 } },
    messages: [
      { id: 'msg_b01', time: T + 5000, data: { role: 'user', time: { created: T + 5000 }, agent: 'general' }, parts: [{ id: 'prt_b01', data: { type: 'text', text: 'Check the TypeScript types' } }] },
      { id: 'msg_b02', time: T + 6000, data: { ...assistant('msg_b02', 'msg_b01', T + 6000, 0.001, tokens(200, 20, 0), 'stop'), agent: 'general' }, parts: [{ id: 'prt_b02', data: { type: 'text', text: 'Types look fine.' } }] },
    ],
  };
}

const dataRoot = (home: string) => join(home, '.local', 'share', 'opencode');
async function put(path: string, value: unknown): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, JSON.stringify(value));
}

export const cases: ConformanceCase[] = [
  {
    name: 'sqlite-layout',
    async setup(home) {
      const root = dataRoot(home);
      await mkdir(root, { recursive: true });
      const db = new Database(join(root, 'opencode.db'));
      db.exec(`CREATE TABLE project (id text PRIMARY KEY, worktree text NOT NULL, vcs text, name text, time_created integer NOT NULL, time_updated integer NOT NULL);
        CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, version text NOT NULL,
          share_url text, summary_additions integer, summary_deletions integer, summary_files integer, time_created integer NOT NULL, time_updated integer NOT NULL);
        CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
        CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`);
      db.prepare('INSERT INTO project VALUES (?, ?, ?, ?, ?, ?)').run(PROJECT, '/work/demo', 'git', null, T, T);
      for (const { session, messages } of [mainSession(), childSession()]) {
        const time = session.time as { created: number; updated: number };
        db.prepare('INSERT INTO session VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)').run(
          session.id as string, PROJECT, (session.parentID as string | undefined) ?? null, session.slug as string, session.directory as string, session.title as string, session.version as string,
          null, null, null, null, time.created, time.updated);
        for (const message of messages) {
          db.prepare('INSERT INTO message VALUES (?, ?, ?, ?, ?)').run(message.id, session.id as string, message.time, message.time, JSON.stringify(message.data));
          for (const part of message.parts) {
            db.prepare('INSERT INTO part VALUES (?, ?, ?, ?, ?, ?)').run(part.id, message.id, session.id as string, message.time, message.time, JSON.stringify(part.data));
          }
        }
      }
      db.close();
      return root;
    },
  },
  {
    name: 'json-storage-layout',
    async setup(home) {
      const root = dataRoot(home);
      for (const { session, messages } of [mainSession(), childSession()]) {
        await put(join(root, 'storage', 'session', PROJECT, `${session.id}.json`), session);
        for (const message of messages) {
          await put(join(root, 'storage', 'message', session.id as string, `${message.id}.json`), { ...message.data, id: message.id, sessionID: session.id });
          for (const part of message.parts) {
            await put(join(root, 'storage', 'part', message.id, `${part.id}.json`), { ...part.data, id: part.id, messageID: message.id, sessionID: session.id });
          }
        }
      }
      return root;
    },
  },
  {
    name: 'legacy-project-layout',
    async setup(home) {
      const root = dataRoot(home);
      const base = join(root, 'project', 'work-demo', 'storage', 'session');
      const { session, messages } = mainSession();
      await put(join(base, 'info', `${session.id}.json`), session);
      for (const message of messages) {
        await put(join(base, 'message', session.id as string, `${message.id}.json`), { ...message.data, id: message.id, sessionID: session.id });
        for (const part of message.parts) {
          await put(join(base, 'part', session.id as string, message.id, `${part.id}.json`), { ...part.data, id: part.id, messageID: message.id, sessionID: session.id });
        }
      }
      return root;
    },
  },
];
