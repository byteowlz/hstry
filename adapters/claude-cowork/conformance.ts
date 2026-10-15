// Layout checked against the adapter's documented desktop-app layout (Claude desktop "Cowork" /
// local agent mode, ~/.config/Claude/local-agent-mode-sessions/<account>/<org>/local_<uuid>.json
// plus local_<uuid>/.claude/projects/<encoded>/<cliSessionId>.jsonl and audit.jsonl). The desktop
// app is closed source; the embedded transcript is the public Claude Code JSONL format
// (type user|assistant, message{role,content[]}, tool_use / tool_result blocks with is_error).
// No upstream repo to diff against; fixtures are synthetic.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';

function orgDir(home: string): string {
  return join(home, '.config', 'Claude', 'local-agent-mode-sessions', 'acct-0001', 'org-0001');
}

const root = (home: string) => join(home, '.config', 'Claude', 'local-agent-mode-sessions');

export const cases: ConformanceCase[] = [
  {
    name: 'claude-code-transcript',
    async setup(home) {
      const org = orgDir(home);
      const sid = 'local_11111111-2222-4333-8444-555555555555';
      const cli = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
      const projects = join(org, sid, '.claude', 'projects', '-sessions-quirky-wizard');
      await mkdir(projects, { recursive: true });
      await writeFile(join(org, `${sid}.json`), JSON.stringify({
        sessionId: sid, cliSessionId: cli, processName: 'quirky-wizard', title: 'Summarize the quarterly report',
        isArchived: false, initialMessage: 'Summarize the quarterly report', createdAt: 1789982398000, lastActivityAt: 1789982430000,
        cwd: '/sessions/quirky-wizard', userSelectedFolders: ['/home/user/reports'], model: 'claude-sonnet-5-5', permissionMode: 'default',
      }));
      await writeFile(join(projects, `${cli}.jsonl`), lines([
        { type: 'user', uuid: 'u1', sessionId: cli, timestamp: '2026-09-21T09:20:00.000Z', message: { role: 'user', content: 'Summarize the quarterly report' } },
        { type: 'assistant', uuid: 'a1', sessionId: cli, timestamp: '2026-09-21T09:20:05.000Z', message: { role: 'assistant', model: 'claude-sonnet-5-5', content: [
          { type: 'thinking', thinking: 'I need to find the report first.', signature: 'sig' },
          { type: 'text', text: 'Let me look for the report.' },
          { type: 'tool_use', id: 'toolu_01', name: 'Bash', input: { command: 'ls /home/user/reports' } },
        ] } },
        { type: 'user', uuid: 'u2', sessionId: cli, timestamp: '2026-09-21T09:20:08.000Z', message: { role: 'user', content: [
          { type: 'tool_result', tool_use_id: 'toolu_01', content: 'q3.pdf\nq3-notes.md' },
        ] } },
        { type: 'assistant', uuid: 'a2', sessionId: cli, timestamp: '2026-09-21T09:20:12.000Z', message: { role: 'assistant', model: 'claude-sonnet-5-5', content: [
          { type: 'tool_use', id: 'toolu_02', name: 'Read', input: { file_path: '/home/user/reports/q3.pdf' } },
        ] } },
        { type: 'user', uuid: 'u3', sessionId: cli, timestamp: '2026-09-21T09:20:14.000Z', message: { role: 'user', content: [
          { type: 'tool_result', tool_use_id: 'toolu_02', is_error: true, content: [{ type: 'text', text: 'Error: cannot read binary PDF' }] },
        ] } },
        { type: 'user', uuid: 'u4', isMeta: true, sessionId: cli, timestamp: '2026-09-21T09:20:15.000Z', message: { role: 'user', content: 'meta caveat' } },
        { type: 'assistant', uuid: 'a3', sessionId: cli, timestamp: '2026-09-21T09:20:30.000Z', message: { role: 'assistant', model: 'claude-sonnet-5-5', content: [
          { type: 'text', text: 'Revenue grew 12% quarter over quarter.' },
        ] } },
      ]));
      return root(home);
    },
  },
  {
    name: 'audit-fallback-and-multiple-sessions',
    async setup(home) {
      const org = orgDir(home);
      const sid = 'local_99999999-8888-4777-8666-555555555555';
      await mkdir(join(org, sid), { recursive: true });
      await writeFile(join(org, `${sid}.json`), JSON.stringify({
        sessionId: sid, processName: 'calm-otter', createdAt: '2026-09-22T08:00:00.000Z', lastActivityAt: '2026-09-22T08:01:00.000Z', model: 'claude-opus-5-5',
      }));
      await writeFile(join(org, sid, 'audit.jsonl'), lines([
        { type: 'user', message: { role: 'user', content: [{ type: 'text', text: 'Rename the files in my folder' }] }, _audit_timestamp: '2026-09-22T08:00:01.000Z' },
        { type: 'assistant', message: { role: 'assistant', content: [{ type: 'text', text: 'Done: 12 files renamed.' }] }, _audit_timestamp: '2026-09-22T08:00:30.000Z' },
      ]));
      // Second session: metadata only, falls back to the initial message stub.
      const sid2 = 'local_00000000-1111-4222-8333-444444444444';
      await writeFile(join(org, `${sid2}.json`), JSON.stringify({
        sessionId: sid2, title: 'Stub session', initialMessage: 'Draft an email to the team', createdAt: 1790100000000, lastActivityAt: 1790100005000,
      }));
      return root(home);
    },
  },
];
