// Format checked against kiro-cli 2.22.0 (vshulcz/deja-vu#4299, sample records captured from the TUI) and
// kirodotdev/Kiro#6110: ~/.kiro/sessions/cli/<session-id>.json (header: session_id, cwd, ...) next to
// <session-id>.jsonl with {version:"v1",kind:"Prompt"|"AssistantMessage"|"ToolResults",
// data:{message_id,content:[{kind:"text"|"toolUse"|"toolResult",data}],meta:{timestamp(seconds)}}}.
// Header keys title/created_at/updated_at (what the adapter reads) are not confirmed by those sources.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';
const rec = (kind: string, message_id: string, content: unknown[], timestamp?: number) =>
  ({ version: 'v1', kind, data: { message_id, content, ...(timestamp ? { meta: { timestamp } } : {}) } });

export const cases: ConformanceCase[] = [{
  name: 'v3-session-with-tools',
  async setup(home) {
    const dir = join(home, '.kiro', 'sessions', 'cli');
    await mkdir(dir, { recursive: true });
    const id = '11111111-2222-4333-8444-555555555555';
    await writeFile(join(dir, `${id}.json`), JSON.stringify({
      session_id: id, cwd: '/tmp/proj', title: 'Fix the retry loop',
      created_at: '2026-10-01T13:00:00.000Z', updated_at: '2026-10-01T13:00:30.000Z',
    }));
    await writeFile(join(dir, `${id}.jsonl`), jsonl([
      rec('Prompt', 'm1', [{ kind: 'text', data: 'fix the retry loop' }], 1790859601),
      rec('AssistantMessage', 'm2', [
        { kind: 'text', data: 'Checking the loop.' },
        { kind: 'toolUse', data: { toolUseId: 't1', name: 'shell', input: { command: 'go test ./retry' } } },
        { kind: 'toolUse', data: { toolUseId: 't2', name: 'write', input: { command: 'create', path: '/tmp/proj/retry.go', content: 'package retry\n' } } },
      ]),
      rec('ToolResults', 'm3', [
        { kind: 'toolResult', data: { toolUseId: 't1', content: [{ kind: 'json', data: { exit_status: 'exit status: 1', stdout: '', stderr: 'FAIL' } }], status: 'error' } },
        { kind: 'toolResult', data: { toolUseId: 't2', content: [{ kind: 'text', data: 'Successfully created /tmp/proj/retry.go' }], status: 'success' } },
      ]),
      rec('AssistantMessage', 'm4', [{ kind: 'text', data: 'The loop now stops after three attempts.' }], 1790859620),
    ]));
    return dir;
  },
}];
