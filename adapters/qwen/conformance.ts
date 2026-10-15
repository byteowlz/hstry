// LEGACY layout only. The adapter reads the Gemini-CLI-derived ~/.qwen/tmp/<hash>/chats/session-*.json
// ({sessionId,projectHash,startTime,lastUpdated,messages[{id,timestamp,type:'user'|'qwen',content,model}]}).
// Current Qwen Code (QwenLM/qwen-code @main, v0.25.1, 2026-10, packages/core/src/services/chatRecordingService.ts
// + config/storage.ts) instead writes ~/.qwen/projects/<sanitized-cwd>/chats/<sessionId>.jsonl tree records
// ({uuid,parentUuid,sessionId,timestamp,type:'user'|'assistant'|'tool_result'|'system',cwd,message:Content,model,...}).
// That is NOT covered here; tracked in a trx bug (qwen: current jsonl chat format unsupported).
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const session = {
  sessionId: '5b6c7d8e-0000-4000-8000-0000000000aa',
  projectHash: 'abc123hash',
  startTime: '2026-03-01T10:00:00.000Z',
  lastUpdated: '2026-03-01T10:00:09.000Z',
  messages: [
    { id: 'm1', timestamp: '2026-03-01T10:00:01.000Z', type: 'user', content: 'What does main.rs do?' },
    {
      id: 'm2', timestamp: '2026-03-01T10:00:05.000Z', type: 'qwen', model: 'qwen3-coder-plus',
      content: 'It parses CLI arguments and starts the server.',
      thoughts: [{ subject: 'Reading', description: 'Looking at main.rs', timestamp: '2026-03-01T10:00:03.000Z' }],
      toolCalls: [{ id: 'read_file-1', name: 'read_file', args: { absolute_path: '/work/main.rs' }, result: [{ functionResponse: { id: 'read_file-1', name: 'read_file', response: { output: 'fn main() {}' } } }], status: 'success' }],
    },
    { id: 'm3', timestamp: '2026-03-01T10:00:08.000Z', type: 'user', content: [{ text: 'Thanks' }, { text: 'bye' }] },
    { id: 'm4', timestamp: '2026-03-01T10:00:09.000Z', type: 'info', content: 'Request cancelled.' },
  ],
};

export const cases: ConformanceCase[] = [{
  name: 'legacy-session-json',
  async setup(home) {
    const dir = join(home, '.qwen', 'tmp', session.projectHash, 'chats');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'session-2026-03-01T10-00-5b6c7d8e.json'), JSON.stringify(session));
    return join(home, '.qwen', 'tmp');
  },
}];
