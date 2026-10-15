// - legacy-session-json: the Gemini-CLI-derived ~/.qwen/tmp/<hash>/chats/session-*.json
//   layout ({sessionId,projectHash,startTime,lastUpdated,messages[{id,timestamp,type,content,model}]}).
// - current-jsonl: the current Qwen Code layout (QwenLM/qwen-code @main, v0.25.x, 2026-10,
//   packages/core/src/services/chatRecordingService.ts + config/storage.ts):
//   ~/.qwen/projects/<sanitized-cwd>/chats/<sessionId>.jsonl, one ChatRecord per line
//   {uuid,parentUuid,sessionId,timestamp,type:user|assistant|tool_result|system,
//    message:Content{role,parts[text|thought|functionCall|functionResponse]},model,...}.
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

const jsonl = [
  { uuid: 'u1', parentUuid: null, sessionId: 'jw-session', timestamp: '2026-05-01T09:00:00.000Z', type: 'system', subtype: 'custom_title', cwd: '/work/proj', version: '0.25.1', systemPayload: { customTitle: 'Investigate the parser', titleSource: 'manual' } },
  { uuid: 'u2', parentUuid: 'u1', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:01.000Z', type: 'system', subtype: 'session_model', cwd: '/work/proj', version: '0.25.1', systemPayload: { modelId: 'qwen3-coder-plus', authType: 'oauth' } },
  { uuid: 'm1', parentUuid: 'u2', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:02.000Z', type: 'user', cwd: '/work/proj', version: '0.25.1', message: { role: 'user', parts: [{ text: 'Where is the parser defined?' }] } },
  { uuid: 'm2', parentUuid: 'm1', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:03.000Z', type: 'assistant', model: 'qwen3-coder-plus', cwd: '/work/proj', version: '0.25.1', message: { role: 'model', parts: [{ text: 'Checking the tree.', thought: true }, { text: 'It lives in src/parser.rs.' }, { functionCall: { id: 'call_1', name: 'read_file', args: { absolute_path: '/work/proj/src/parser.rs' } } }] } },
  { uuid: 'm3', parentUuid: 'm2', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:04.000Z', type: 'tool_result', cwd: '/work/proj', version: '0.25.1', message: { role: 'user', parts: [{ functionResponse: { id: 'call_1', name: 'read_file', response: { output: 'pub fn parse() {}' } } }] }, toolCallResult: { callId: 'call_1', status: 'success' } },
  { uuid: 'm4', parentUuid: 'm3', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:05.000Z', type: 'assistant', model: 'qwen3-coder-plus', cwd: '/work/proj', version: '0.25.1', message: { role: 'model', parts: [{ functionCall: { id: 'call_2', name: 'run_shell_command', args: { command: 'cargo test' } } }] } },
  { uuid: 'm5', parentUuid: 'm4', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:06.000Z', type: 'tool_result', cwd: '/work/proj', version: '0.25.1', message: { role: 'user', parts: [{ functionResponse: { id: 'call_2', name: 'run_shell_command', response: {} } }] }, toolCallResult: { callId: 'call_2', status: 'error', error: 'exit code 1' } },
  { uuid: 'm6', parentUuid: 'm5', sessionId: 'jw-session', timestamp: '2026-05-01T09:00:07.000Z', type: 'system', subtype: 'chat_compression', cwd: '/work/proj', version: '0.25.1', systemPayload: { info: { originalTokenCount: 900, newTokenCount: 300 } } },
].map(record => JSON.stringify(record)).join('\n') + '\n';

export const cases: ConformanceCase[] = [
  {
    name: 'legacy-session-json',
    async setup(home) {
      const dir = join(home, '.qwen', 'tmp', session.projectHash, 'chats');
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, 'session-2026-03-01T10-00-5b6c7d8e.json'), JSON.stringify(session));
      return join(home, '.qwen', 'tmp');
    },
    check(conversations) {
      const [conversation] = conversations as { messages: { role: string; parts?: Record<string, any>[] }[] }[];
      const parts = conversation.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking')) throw new Error('legacy thoughts were dropped');
      if (!parts.some(p => p.type === 'tool_call')) throw new Error('legacy toolCalls were dropped');
      if (!parts.some(p => p.type === 'tool_result')) throw new Error('legacy tool results were dropped');
    },
  },
  {
    name: 'current-jsonl',
    async setup(home) {
      const dir = join(home, '.qwen', 'projects', '-work-proj', 'chats');
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, 'jw-session.jsonl'), jsonl);
      return join(home, '.qwen', 'projects');
    },
    check(conversations) {
      const [conversation] = conversations as { title?: string; model?: string; messages: { role: string; content: string; parts?: Record<string, any>[] }[] }[];
      if (conversation.title !== 'Investigate the parser') throw new Error(`custom_title not used: ${conversation.title}`);
      if (conversation.model !== 'qwen3-coder-plus') throw new Error(`session_model not used: ${conversation.model}`);
      const parts = conversation.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking' && p.text.includes('Checking the tree'))) throw new Error('thought part dropped');
      const errorResult = parts.find(p => p.type === 'tool_result' && p.isError === true);
      if (!errorResult) throw new Error('failed tool result lost its error status');
      if (conversation.messages.some(m => m.content.includes('originalTokenCount'))) throw new Error('chat_compression payload leaked into messages');
    },
  },
];
