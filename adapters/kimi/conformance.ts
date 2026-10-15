// Format checked against Kimi Code CLI wire.jsonl as documented from 41 real files in
// MemPalace/mempalace#2180 (Aug 2026): ~/.kimi-code/sessions/<workspace-slug>/session-<uuid>/agents/<agent-id>/wire.jsonl,
// first line {"type":"metadata","protocol_version":...}; user input in turn.prompt and context.append_message
// (origin.kind 'user' | injection | system_trigger | background_task | skill_activation); assistant output in
// context.append_loop_event -> event.type 'content.part' (part.type 'think'|'text'), 'tool.call', 'tool.result'.
// state.json beside agents/ carries session metadata. NOT independently verifiable from public docs: the exact
// key names for time ('time'), tool.call (toolCallId/name/args), tool.result (toolCallId/result/isError) and
// state.json (id/title/workDir/model/createdAt/updatedAt); they follow what the adapter reads.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';
const loop = (time: string, event: unknown) => ({ type: 'context.append_loop_event', time, event });

export const cases: ConformanceCase[] = [{
  name: 'session-with-tools',
  async setup(home) {
    const root = join(home, '.kimi-code', 'sessions');
    const session = join(root, 'work-proj', 'session-9d8c7b6a-0000-4000-8000-0000000000e1');
    const agent = join(session, 'agents', 'root');
    await mkdir(agent, { recursive: true });
    await writeFile(join(session, 'state.json'), JSON.stringify({
      id: 'session-9d8c7b6a-0000-4000-8000-0000000000e1', title: 'Count lines', workDir: '/work/proj',
      model: 'kimi-k2-thinking', createdAt: '2026-06-01T12:00:00.000Z', updatedAt: '2026-06-01T12:00:10.000Z',
    }));
    await writeFile(join(agent, 'wire.jsonl'), jsonl([
      { type: 'metadata', protocol_version: '1.0' },
      { type: 'config.update', time: '2026-06-01T12:00:00.000Z' },
      { type: 'turn.prompt', time: '2026-06-01T12:00:01.000Z', origin: { kind: 'user' }, input: [{ type: 'text', text: 'How many lines in main.py?' }] },
      { type: 'context.append_message', time: '2026-06-01T12:00:01.000Z', origin: { kind: 'user' }, message: { role: 'user', content: [{ type: 'text', text: 'How many lines in main.py?' }] } },
      { type: 'context.append_message', time: '2026-06-01T12:00:01.500Z', origin: { kind: 'injection' }, message: { role: 'user', content: [{ type: 'text', text: '<system-reminder>permission mode: default</system-reminder>' }] } },
      { type: 'llm.request', time: '2026-06-01T12:00:02.000Z' },
      loop('2026-06-01T12:00:03.000Z', { type: 'content.part', part: { type: 'think', text: 'Use wc -l.' } }),
      loop('2026-06-01T12:00:04.000Z', { type: 'content.part', part: { type: 'text', text: 'Counting lines.' } }),
      loop('2026-06-01T12:00:05.000Z', { type: 'tool.call', toolCallId: 'tc_1', name: 'Shell', args: { command: 'wc -l main.py' } }),
      loop('2026-06-01T12:00:06.000Z', { type: 'tool.result', toolCallId: 'tc_1', result: 'wc: main.py: No such file or directory', isError: true }),
      { type: 'usage.record', time: '2026-06-01T12:00:07.000Z' },
      loop('2026-06-01T12:00:09.000Z', { type: 'content.part', part: { type: 'text', text: 'main.py does not exist.' } }),
    ]));
    return root;
  },
}];
