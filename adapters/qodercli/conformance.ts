// Format checked against the Qoder docs (docs.qoder.com/extensions/hooks "Transcript File Format" and
// docs.qoder.com/cli/sessions, 2026-10): CLI logs at ~/.qoder/projects/<project>/<session-id>.jsonl, IDE
// transcripts at ~/.qoder/projects/<project>/transcript/<session-id>.jsonl; lines are
// session_meta | user | assistant | progress with {sessionId,uuid,timestamp(ISO),cwd,message}. User questions
// have string content; tool results are user messages with tool_result blocks {tool_use_id,content,is_error};
// assistant content is always an array of text/tool_use{id,name,input} blocks.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';
const line = (sessionId: string, n: number, timestamp: string, type: string, message: unknown, extra: Record<string, unknown> = {}) =>
  ({ type, sessionId, uuid: `${sessionId}-${n}`, timestamp, cwd: '/work/proj', message, ...extra });

export const cases: ConformanceCase[] = [{
  name: 'cli-and-ide-sessions',
  async setup(home) {
    const root = join(home, '.qoder', 'projects');
    const project = join(root, '-work-proj');
    await mkdir(join(project, 'transcript'), { recursive: true });
    const a = '86379a0e-0000-4000-8000-0000000000a1';
    await writeFile(join(project, `${a}.jsonl`), jsonl([
      { type: 'session_meta', sessionId: a, data: { meta_type: 'session_info', content: { mode: 'agent', session_type: 'assistant' } } },
      { type: 'progress', sessionId: a, data: { type: 'hook_progress', hookEvent: 'UserPromptSubmit', hookName: 'UserPromptSubmit', command: '.qoder/hooks/x.sh' } },
      line(a, 1, '2026-08-01T09:00:00.000Z', 'user', { role: 'user', content: 'Delete comments from test.py' }),
      line(a, 2, '2026-08-01T09:00:02.000Z', 'assistant', { role: 'assistant', model: 'qwen3-coder', content: [{ type: 'text', text: "I'll read the file first." }] }),
      line(a, 3, '2026-08-01T09:00:03.000Z', 'assistant', { role: 'assistant', model: 'qwen3-coder', content: [{ type: 'tool_use', id: 'call_1', name: 'read_file', input: { file_path: '/work/proj/test.py' } }] }),
      line(a, 4, '2026-08-01T09:00:04.000Z', 'user', { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'call_1', content: 'File not found: /work/proj/test.py', is_error: true }] }, { toolUseResult: 'File not found: /work/proj/test.py' }),
      line(a, 5, '2026-08-01T09:00:06.000Z', 'assistant', { role: 'assistant', model: 'qwen3-coder', content: [{ type: 'text', text: 'test.py does not exist.' }] }),
    ]));
    const b = '5a4b3c2d-0000-4000-8000-0000000000b2';
    await writeFile(join(project, 'transcript', `${b}.jsonl`), jsonl([
      { type: 'session_meta', sessionId: b, data: { meta_type: 'session_info', content: { mode: 'plan', session_type: 'assistant' } } },
      line(b, 1, '2026-08-02T09:00:00.000Z', 'user', { role: 'user', content: 'hello' }),
      line(b, 2, '2026-08-02T09:00:01.000Z', 'assistant', { role: 'assistant', content: [{ type: 'text', text: 'hi!' }] }),
    ]));
    return root;
  },
}];
