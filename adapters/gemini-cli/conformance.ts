// Format checked against google-gemini/gemini-cli main
// packages/core/src/services/chatRecordingService.ts (2026): sessions are
// ~/.gemini/tmp/<project>/chats/session-<timestamp>-<shortId>.jsonl with a
// metadata header line (sessionId, projectHash, startTime, lastUpdated, kind),
// then one record per message: type user|gemini (+info/warning/error logs),
// content (string or Part[]), thoughts[], tokens, model, toolCalls[
// {id,name,args,result:[{functionResponse}],status,timestamp,displayName}].
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = [
  { sessionId: 'sess-aaaa1111', projectHash: 'deadbeef', startTime: '2026-03-01T09:00:00.000Z', lastUpdated: '2026-03-01T09:00:30.000Z', kind: 'main' },
  { id: 'u1', timestamp: '2026-03-01T09:00:01.000Z', type: 'user', content: [{ text: 'List the files in src' }] },
  { id: 'g1', timestamp: '2026-03-01T09:00:03.000Z', type: 'gemini', content: '', model: 'gemini-2.5-pro',
    thoughts: [{ subject: 'Listing', description: 'Use the ls tool.', timestamp: '2026-03-01T09:00:02.000Z' }],
    tokens: { input: 10, output: 5, cached: 0, thoughts: 3, tool: 0, total: 18 },
    toolCalls: [
      { id: 'list_directory-1', name: 'list_directory', args: { dir_path: 'src' }, status: 'success', timestamp: '2026-03-01T09:00:03.000Z', displayName: 'ReadFolder',
        result: [{ functionResponse: { id: 'list_directory-1', name: 'list_directory', response: { output: 'Directory listing for src:\nmain.ts' } } }],
        resultDisplay: 'Listed 1 item(s).' },
      { id: 'read_file-2', name: 'read_file', args: { file_path: 'src/missing.ts' }, status: 'error', timestamp: '2026-03-01T09:00:04.000Z', displayName: 'ReadFile',
        result: [{ functionResponse: { id: 'read_file-2', name: 'read_file', response: { error: 'File not found: src/missing.ts' } } }],
        resultDisplay: 'File not found' },
    ] },
  { type: 'info', content: 'Request cancelled.' },
  { id: 'g2', timestamp: '2026-03-01T09:00:10.000Z', type: 'gemini', content: 'src contains main.ts; missing.ts does not exist.', model: 'gemini-2.5-pro' },
  { id: 'u2', timestamp: '2026-03-01T09:00:20.000Z', type: 'user', content: 'Thanks' },
];

export const cases: ConformanceCase[] = [{
  name: 'jsonl-session-with-tools',
  async setup(home) {
    const chats = join(home, '.gemini', 'tmp', 'demo-project', 'chats');
    await mkdir(chats, { recursive: true });
    await writeFile(join(chats, 'session-2026-03-01T09-00-aaaa1111.jsonl'), lines.map(l => JSON.stringify(l)).join('\n') + '\n');
    return join(home, '.gemini', 'tmp');
  },
}];
