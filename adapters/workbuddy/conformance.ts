// WorkBuddy (Tencent) is closed source; no upstream repo to diff against. The event shape
// (ai-title, reasoning/rawContent, message input_text/output_text, function_call,
// function_call_result with providerData.model) is taken from the existing real-format fixture
// testdata/workbuddy/project-a/session-test.jsonl and the adapter's documented
// ~/.workbuddy/projects/<project>/<session>.jsonl layout. Fixtures here are synthetic.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';
const T = 1790000000000;
const model = { model: 'workbuddy-model' };

export const cases: ConformanceCase[] = [
  {
    name: 'project-sessions',
    async setup(home) {
      const root = join(home, '.workbuddy', 'projects');
      const project = join(root, 'project-a');
      await mkdir(join(project, 'subagents'), { recursive: true });
      await writeFile(join(project, 'session-one.jsonl'), lines([
        { id: 'title', timestamp: T, type: 'ai-title', aiTitle: 'Deploy the service' },
        { id: 'snap', timestamp: T + 10, type: 'file-history-snapshot', snapshot: {} },
        { id: 'u1', timestamp: T + 200, type: 'message', role: 'user', cwd: '/projects/demo', content: [{ type: 'input_text', text: '<system-reminder>private setup</system-reminder><user_query>Deploy the service</user_query>' }] },
        { id: 'r1', timestamp: T + 300, type: 'reasoning', rawContent: [{ type: 'reasoning_text', text: 'Run the deploy script, then verify.' }] },
        { id: 'c1', timestamp: T + 400, type: 'function_call', callId: 'call-1', name: 'shell', arguments: '{"command":"./deploy.sh"}', providerData: model },
        { id: 'o1', timestamp: T + 500, type: 'function_call_result', callId: 'call-1', name: 'shell', output: { type: 'text', text: 'deploy.sh: permission denied' }, status: 'failed' },
        { id: 'c2', timestamp: T + 600, type: 'function_call', callId: 'call-2', name: 'shell', arguments: '{"command":"chmod +x deploy.sh && ./deploy.sh"}', providerData: model },
        { id: 'o2', timestamp: T + 700, type: 'function_call_result', callId: 'call-2', name: 'shell', output: { type: 'text', text: 'deployed' }, status: 'success' },
        { id: 'a1', timestamp: T + 800, type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Deployment complete.' }], providerData: model },
      ]));
      await writeFile(join(project, 'session-two.jsonl'), lines([
        { id: 'u1', timestamp: T + 5000, type: 'message', role: 'user', cwd: '/projects/other', content: [{ type: 'input_text', text: 'Summarize notes.md' }] },
        { id: 'a1', timestamp: T + 6000, type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'The notes cover the Q3 plan.' }], providerData: model },
      ]));
      // Sub-agent transcripts are intentionally not imported.
      await writeFile(join(project, 'subagents', 'agent-1.jsonl'), lines([
        { id: 'u1', timestamp: T, type: 'message', role: 'user', content: [{ type: 'input_text', text: 'sub-agent prompt' }] },
      ]));
      return root;
    },
  },
];
