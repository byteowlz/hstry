// Format checked against cline/cline @main (2026-10, apps/vscode/src/sdk/legacy-state-reader.ts,
// legacy-task-handling.ts, shared/ExtensionMessage.ts): task dir <dataDir>/tasks/<task-id ms timestamp>/ with
// api_conversation_history.json (Anthropic MessageParam list: text|thinking|tool_use|tool_result blocks),
// ui_messages.json (ClineMessage {ts,type:'ask'|'say',say,text}) and task_metadata.json
// ({files_in_context,model_usage}; no title). Classic VS Code location is
// ~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks (used here); the SDK-era data dir
// ~/.cline/data/tasks holds the same files but is env-dependent (CLINE_DATA_DIR/CLINE_DIR) so it is not exercised.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const taskId = '1790000000000';

const api = [
  { role: 'user', content: [{ type: 'text', text: '<task>\nCreate hello.txt\n</task>' }, { type: 'text', text: '<environment_details>\ncwd: /work\n</environment_details>' }] },
  { role: 'assistant', content: [
    { type: 'text', text: 'I will create the file.' },
    { type: 'tool_use', id: 'toolu_w1', name: 'write_to_file', input: { path: 'hello.txt', content: 'hello' } },
  ] },
  { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'toolu_w1', content: [{ type: 'text', text: 'Permission denied: hello.txt' }], is_error: true }] },
  { role: 'assistant', content: [{ type: 'text', text: 'I could not write the file.' }] },
];

const ui = [
  { ts: 1790000000000, type: 'say', say: 'task', text: 'Create hello.txt' },
  { ts: 1790000001000, type: 'say', say: 'api_req_started', text: '{"request":"..."}' },
  { ts: 1790000003000, type: 'say', say: 'text', text: 'I will create the file.' },
  { ts: 1790000005000, type: 'ask', ask: 'tool', text: '{"tool":"newFileCreated","path":"hello.txt"}' },
  { ts: 1790000009000, type: 'say', say: 'completion_result', text: 'I could not write the file.' },
];

export const cases: ConformanceCase[] = [{
  name: 'task-with-tool-error',
  async setup(home) {
    const tasks = join(home, '.config', 'Code', 'User', 'globalStorage', 'saoudrizwan.claude-dev', 'tasks');
    const dir = join(tasks, taskId);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'api_conversation_history.json'), JSON.stringify(api));
    await writeFile(join(dir, 'ui_messages.json'), JSON.stringify(ui));
    await writeFile(join(dir, 'task_metadata.json'), JSON.stringify({ files_in_context: [], model_usage: [] }));
    return tasks;
  },
}];
