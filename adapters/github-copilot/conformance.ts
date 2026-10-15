// Format checked against the Copilot CLI event-log description at
// jazzyalex.github.io/agent-sessions/guides/copilot-cli-local-history.html (verified through Copilot CLI 1.0.80):
// ~/.copilot/session-state/<session-id>/events.jsonl (flat <session-id>.jsonl before 1.0.11), one
// {type,data,id,parentId,timestamp} envelope per line; session.start{sessionId,copilotVersion,selectedModel,context},
// user.message{content}, assistant.message{content,toolRequests[{toolCallId,name,arguments}],reasoningText},
// tool.execution_start/complete{toolCallId,success,result.content}. workspace.yaml sits beside events.jsonl.
// Field-level details beyond that guide (e.g. error object shape) are synthetic.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';
let n = 0;
const ev = (type: string, timestamp: string, data: unknown) => ({ type, data, id: `evt-${++n}`, parentId: n > 1 ? `evt-${n - 1}` : null, timestamp });

export const cases: ConformanceCase[] = [{
  name: 'session-state-with-tools',
  async setup(home) {
    const root = join(home, '.copilot', 'session-state');
    const id = '7e1f0c2a-0000-4000-8000-0000000000c1';
    const dir = join(root, id);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'workspace.yaml'), `id: ${id}\ncwd: /work/proj\ngit_root: /work/proj\nbranch: main\n`);
    await writeFile(join(dir, 'events.jsonl'), jsonl([
      ev('session.start', '2026-04-02T08:00:00.000Z', { sessionId: id, copilotVersion: '1.0.80', selectedModel: 'gpt-5.1', context: { cwd: '/work/proj', branch: 'main' } }),
      ev('user.message', '2026-04-02T08:00:01.000Z', { content: 'Run the tests', attachments: [] }),
      ev('assistant.turn_start', '2026-04-02T08:00:02.000Z', { turnId: '0' }),
      ev('assistant.message', '2026-04-02T08:00:03.000Z', {
        messageId: 'a1', content: 'Running the test suite.', reasoningText: 'Use the shell tool.',
        toolRequests: [{ toolCallId: 'call_1', name: 'bash', arguments: { command: 'npm test' }, type: 'function' }],
      }),
      ev('tool.execution_start', '2026-04-02T08:00:03.500Z', { toolCallId: 'call_1', toolName: 'bash', arguments: { command: 'npm test' } }),
      ev('tool.execution_complete', '2026-04-02T08:00:05.000Z', { toolCallId: 'call_1', success: false, error: { message: 'exit code 1: 2 tests failed' } }),
      ev('assistant.message', '2026-04-02T08:00:06.000Z', { messageId: 'a2', content: 'Two tests failed.', toolRequests: [] }),
      ev('assistant.turn_end', '2026-04-02T08:00:06.500Z', { turnId: '0' }),
    ]));
    // pre-1.0.11 flat layout, second session
    await writeFile(join(root, 'a0b1c2d3-0000-4000-8000-0000000000c2.jsonl'), jsonl([
      ev('session.start', '2026-04-01T08:00:00.000Z', { sessionId: 'a0b1c2d3-0000-4000-8000-0000000000c2', selectedModel: 'claude-sonnet-4.5', context: { cwd: '/work/old' } }),
      ev('user.message', '2026-04-01T08:00:01.000Z', { content: 'hello' }),
      ev('assistant.message', '2026-04-01T08:00:02.000Z', { messageId: 'b1', content: 'hi there', toolRequests: [] }),
    ]));
    return root;
  },
}];
