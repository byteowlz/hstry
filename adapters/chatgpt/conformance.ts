// Format checked against the OpenAI ChatGPT data export (Settings > Data
// controls > Export; conversations.json) as of 2025: a tree `mapping` with
// `current_node`, retained regenerated branches, hidden system nodes,
// `content_type` text/code/execution_output, `metadata.model_slug`.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const node = (id: string, parent: string | null, children: string[], message: unknown) =>
  ({ id, parent, children, message });
const msg = (id: string, role: string, content: unknown, time: number | null, metadata: unknown = {}) => ({
  id, author: { role, name: null, metadata: {} }, create_time: time, update_time: null,
  content, status: 'finished_successfully', end_turn: true, weight: 1.0, metadata,
  recipient: 'all', channel: null,
});

const conversations = [
  {
    id: 'conv-0001', title: 'Sum a list', create_time: 1750000000.5, update_time: 1750000100.25,
    current_node: 'n-final', conversation_id: 'conv-0001', default_model_slug: 'gpt-4o',
    mapping: {
      'client-root': node('client-root', null, ['n-sys'], null),
      'n-sys': node('n-sys', 'client-root', ['n-user'], msg('n-sys', 'system',
        { content_type: 'text', parts: [''] }, null, { is_visually_hidden_from_conversation: true })),
      'n-user': node('n-user', 'n-sys', ['n-bad', 'n-code'], msg('n-user', 'user',
        { content_type: 'text', parts: ['Sum [1,2,3] in python'] }, 1750000001.1)),
      'n-bad': node('n-bad', 'n-user', [], msg('n-bad', 'assistant',
        { content_type: 'text', parts: ['REGENERATED-DISCARDED answer'] }, 1750000002.2, { model_slug: 'gpt-4o' })),
      'n-code': node('n-code', 'n-user', ['n-out'], msg('n-code', 'assistant',
        { content_type: 'code', language: 'python', text: 'print(sum([1,2,3]))' }, 1750000003.3, { model_slug: 'gpt-4o' })),
      'n-out': node('n-out', 'n-code', ['n-final'], msg('n-out', 'tool',
        { content_type: 'execution_output', text: '6' }, 1750000004.4)),
      'n-final': node('n-final', 'n-out', [], msg('n-final', 'assistant',
        { content_type: 'text', parts: ['The sum is 6.'] }, 1750000005.5, { model_slug: 'gpt-4o' })),
    },
  },
  {
    id: 'conv-0002', title: null, create_time: 1750001000, update_time: 1750001010,
    current_node: 'b', default_model_slug: 'gpt-4o-mini',
    mapping: {
      root: node('root', null, ['a'], null),
      a: node('a', 'root', ['b'], msg('a', 'user', { content_type: 'text', parts: ['Hi'] }, 1750001001)),
      b: node('b', 'a', [], msg('b', 'assistant', { content_type: 'text', parts: ['Hello!'] }, 1750001002, { model_slug: 'gpt-4o-mini' })),
    },
  },
];

export const cases: ConformanceCase[] = [{
  name: 'conversations-json-export',
  detection: 'content',
  async setup(home) {
    const dir = join(home, 'Downloads', 'chatgpt-export');
    await mkdir(dir, { recursive: true });
    const file = join(dir, 'conversations.json');
    await writeFile(file, JSON.stringify(conversations));
    return file;
  },
}];
