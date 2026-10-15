// Format checked against sageox/ox ox-adapter-droid docs (pkg.go.dev, "confirmed against a real droid 0.126.0
// install"): ~/.factory/sessions/<cwd-slug>/<uuid>.jsonl; first line {"type":"session_start",id,title,cwd},
// then {"type":"message", message:{role, content:[text|thinking|tool_use|tool_result|image blocks]}};
// companion <uuid>.settings.json is not read by the adapter. Per-line timestamp and block field names
// (tool_use.id/name/input, tool_result.tool_use_id/content/is_error) follow the Anthropic block convention.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';
const msg = (id: string, timestamp: string, role: string, content: unknown[]) => ({ type: 'message', id, timestamp, message: { role, content } });

export const cases: ConformanceCase[] = [{
  name: 'session-with-tools',
  async setup(home) {
    const root = join(home, '.factory', 'sessions');
    const dir = join(root, '-work-proj');
    await mkdir(dir, { recursive: true });
    const id = '3f2a1b00-0000-4000-8000-0000000000d1';
    await writeFile(join(dir, `${id}.jsonl`), jsonl([
      { type: 'session_start', id, title: 'Fix the failing build', cwd: '/work/proj' },
      msg('m1', '2026-05-03T09:00:00.000Z', 'user', [{ type: 'text', text: 'Fix the failing build' }]),
      msg('m2', '2026-05-03T09:00:02.000Z', 'assistant', [
        { type: 'thinking', thinking: 'Run the build first.' },
        { type: 'text', text: 'Running the build.' },
        { type: 'tool_use', id: 'toolu_a', name: 'Execute', input: { command: 'make' } },
      ]),
      msg('m3', '2026-05-03T09:00:04.000Z', 'user', [{ type: 'tool_result', tool_use_id: 'toolu_a', content: 'make: *** No rule to make target', is_error: true }]),
      msg('m4', '2026-05-03T09:00:06.000Z', 'assistant', [{ type: 'text', text: 'The Makefile is missing a target.' }]),
    ]));
    await writeFile(join(dir, '3f2a1b00-0000-4000-8000-0000000000d2.jsonl'), jsonl([
      { type: 'session_start', id: '3f2a1b00-0000-4000-8000-0000000000d2', title: 'Second', cwd: '/work/proj' },
      msg('n1', '2026-05-04T09:00:00.000Z', 'user', [{ type: 'text', text: 'hi' }]),
      msg('n2', '2026-05-04T09:00:01.000Z', 'assistant', [{ type: 'text', text: 'hello' }]),
    ]));
    return root;
  },
}];
