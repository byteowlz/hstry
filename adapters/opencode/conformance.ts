// Format checked against anomalyco/opencode (sst/opencode) @ 4397ee4 (2026-10-15, cli/core 2.0.29):
// packages/schema/src/v1/session.ts (v1 message/part shapes: text, reasoning, tool{callID, tool,
// state pending|running|completed|error(.error), time}, step-start/finish, patch) and
// packages/core/src/database/v1-migration.bun.ts (v1 tables project/session/message/part with JSON
// `data` blobs, ms timestamps).
// 2.x layouts (anomalyco/opencode @ dev): packages/core/src/session/sql.ts defines session_v2 +
// session_message (id, session_id, type, seq, time_*, data JSON) and
// packages/schema/src/session-message.ts defines the typed records (user/assistant/synthetic/system/
// skill/shell/compaction/idle/agent-switched/model-switched/location-switched; assistant content
// blocks text|reasoning|tool with tool state streaming|running|completed|error). The adapter reads
// all of: SQLite opencode.db v1 (1.1.45+), SQLite opencode.db v2 (session_v2/session_message),
// storage/{session,message,part} (1.1.25+) and project/<name>/storage/session/* (legacy).
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

/**
 * OpenCode 2.x sessions as `session_v2` + typed `session_message` rows
 * (packages/schema/src/session-message.ts).
 */
function v2Sessions(): { id: string; parentID?: string; title: string; time: { created: number; updated: number }; messages: { id: string; type: string; seq: number; time: number; data: Record<string, unknown> }[] }[] {
  const main = {
    id: SESSION,
    title: 'Fix the build',
    time: { created: T, updated: T + 12000 },
    messages: [
      { id: 'msg_v01', type: 'user', seq: 1, time: T + 1000, data: { type: 'user', time: { created: T + 1000 }, text: 'The build is failing, can you fix it?', files: [], agents: [], skills: [] } },
      { id: 'msg_v02', type: 'assistant', seq: 2, time: T + 3000, data: { type: 'assistant', time: { created: T + 3000, completed: T + 6000 }, agent: 'build', model: { id: 'claude-sonnet-4-5', providerID: 'anthropic' }, content: [
        { type: 'reasoning', text: 'Run the build to see the error.', time: { created: T + 3000 } },
        { type: 'text', text: 'Let me run the build.' },
        { type: 'tool', id: 'toolu_01', name: 'bash', state: { status: 'completed', input: { command: 'npm run build' }, content: [{ type: 'text', text: 'error TS2304: Cannot find name "foo"' }] }, time: { created: T + 4000, completed: T + 6000 } },
        { type: 'tool', id: 'toolu_02', name: 'bash', state: { status: 'error', input: { command: 'npm run buidl' }, error: { message: 'Missing script: "buidl"' } }, time: { created: T + 5000, completed: T + 5100 } },
      ], cost: 0.012, tokens: { input: 1500, output: 120, reasoning: 40, cache: { read: 0, write: 0 } }, finish: 'tool-calls' } },
      { id: 'msg_v03', type: 'shell', seq: 3, time: T + 7000, data: { type: 'shell', shellID: 'sh_01', command: 'git status', status: 'completed', exit: 0, output: 'On branch main', time: { created: T + 7000, completed: T + 7100 } } },
      { id: 'msg_v04', type: 'system', seq: 4, time: T + 7200, data: { type: 'system', text: 'Environment reloaded.', description: 'env' } },
      { id: 'msg_v05', type: 'compaction', seq: 5, time: T + 7400, data: { type: 'compaction', status: 'completed', reason: 'auto', summary: 'Earlier the user asked to fix the build.' } },
      { id: 'msg_v06', type: 'idle', seq: 6, time: T + 8000, data: { type: 'idle', outcome: 'succeeded' } },
      { id: 'msg_v07', type: 'assistant', seq: 7, time: T + 9000, data: { type: 'assistant', time: { created: T + 9000 }, agent: 'build', model: { id: 'claude-sonnet-4-5', providerID: 'anthropic' }, content: [{ type: 'text', text: 'Fixed the undefined name in src/index.ts.' }], cost: 0.008, tokens: { input: 1800, output: 60, reasoning: 0, cache: { read: 0, write: 0 } }, finish: 'stop' } },
    ],
  };
  const child = {
    id: CHILD,
    parentID: SESSION,
    title: 'Subagent: check types',
    time: { created: T + 5000, updated: T + 8000 },
    messages: [
      { id: 'msg_w01', type: 'user', seq: 1, time: T + 5000, data: { type: 'user', time: { created: T + 5000 }, text: 'Check the TypeScript types', files: [], agents: [], skills: [] } },
      { id: 'msg_w02', type: 'assistant', seq: 2, time: T + 6000, data: { type: 'assistant', time: { created: T + 6000 }, agent: 'general', model: { id: 'claude-haiku-4-5', providerID: 'anthropic' }, content: [{ type: 'reasoning', text: 'Look for type errors.' }, { type: 'text', text: 'Types look fine.' }], finish: 'stop' } },
    ],
  };
  return [main, child];
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
  {
    name: 'sqlite-v2-layout',
    async setup(home) {
      const root = dataRoot(home);
      await mkdir(root, { recursive: true });
      const db = new Database(join(root, 'opencode.db'));
      db.exec(`CREATE TABLE project (id text PRIMARY KEY, worktree text NOT NULL);
        CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text, directory text, title text, version text NOT NULL,
          summary_additions integer, summary_deletions integer, summary_files integer, cost real NOT NULL DEFAULT 0,
          tokens_input integer NOT NULL DEFAULT 0, tokens_output integer NOT NULL DEFAULT 0, tokens_reasoning integer NOT NULL DEFAULT 0,
          model text, agent text, time_created integer NOT NULL, time_updated integer NOT NULL);
        CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL,
          time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`);
      db.prepare('INSERT INTO project VALUES (?, ?)').run(PROJECT, '/work/demo');
      const insertSession = db.prepare(
        `INSERT INTO session_v2 (id, project_id, parent_id, slug, directory, title, version, cost, tokens_input, tokens_output, tokens_reasoning, model, agent, time_created, time_updated)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      const insertMessage = db.prepare('INSERT INTO session_message VALUES (?, ?, ?, ?, ?, ?, ?)');
      for (const { id, parentID, title, time, messages } of v2Sessions()) {
        insertSession.run(id, PROJECT, parentID ?? null, 'quiet-river', '/work/demo', title, '2.0.29', 0.02, 3300, 180, 40, JSON.stringify({ id: 'claude-sonnet-4-5', providerID: 'anthropic' }), 'build', time.created, time.updated);
        for (const message of messages) {
          insertMessage.run(message.id, id, message.type, message.seq, message.time, message.time, JSON.stringify(message.data));
        }
      }
      db.close();
      return root;
    },
    check(conversations) {
      const convs = conversations as { externalId: string; parentExternalId?: string; forkType?: string; messages: { role: string; content: string; parts?: Record<string, any>[] }[] }[];
      const main = convs.find(c => c.externalId === SESSION)!;
      const child = convs.find(c => c.externalId === CHILD)!;
      if (child.parentExternalId !== SESSION || child.forkType !== 'thread') throw new Error('session_v2 parent_id not mapped to parentExternalId/forkType');
      const parts = main.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking' && p.text.includes('Run the build'))) throw new Error('assistant reasoning block dropped');
      if (!parts.some(p => p.type === 'tool_result' && p.isError === true && String(p.output).includes('buidl'))) throw new Error('errored tool state lost its error');
      if (!parts.some(p => p.type === 'tool_result' && String(p.output).includes('TS2304'))) throw new Error('completed tool content dropped');
      if (!main.messages.some(m => m.role === 'tool' && m.content.includes('On branch main'))) throw new Error('shell record not surfaced as a tool message');
      if (!main.messages.some(m => m.role === 'system' && m.content.includes('Earlier the user asked'))) throw new Error('compaction summary dropped');
      if (main.messages.some(m => m.content.includes('succeeded'))) throw new Error('idle bookkeeping leaked into messages');
      if (!child.parentExternalId) throw new Error('child session missing parent');
    },
  },
];
