// Format checked against the claude.ai data export (Settings > Privacy >
// Export data; conversations.json) as of 2025: uuid/name/created_at ISO,
// chat_messages[] with sender human|assistant, `text`, and typed `content`
// blocks (text, thinking, tool_use, tool_result).
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const account = { uuid: 'acct-0001' };
const exportData = [
  {
    uuid: 'c1111111-0000-0000-0000-000000000001', name: 'Fix a bug', summary: '',
    created_at: '2025-06-01T10:00:00.000000Z', updated_at: '2025-06-01T10:05:00.000000Z', account,
    chat_messages: [
      {
        uuid: 'm1', sender: 'human', text: 'Why does my loop never end?',
        content: [{ start_timestamp: '2025-06-01T10:00:01.000000Z', stop_timestamp: '2025-06-01T10:00:01.000000Z', type: 'text', text: 'Why does my loop never end?', citations: [] }],
        created_at: '2025-06-01T10:00:01.000000Z', updated_at: '2025-06-01T10:00:01.000000Z', attachments: [], files: [],
      },
      {
        uuid: 'm2', sender: 'assistant', text: 'You never increment i.',
        content: [
          { type: 'thinking', thinking: 'Looks like a missing increment.', summaries: [] },
          { start_timestamp: '2025-06-01T10:00:05.000000Z', type: 'text', text: 'You never increment i.', citations: [] },
        ],
        created_at: '2025-06-01T10:00:05.000000Z', updated_at: '2025-06-01T10:00:05.000000Z', attachments: [], files: [],
      },
      {
        uuid: 'm3', sender: 'human', text: 'Thanks!',
        content: [{ type: 'text', text: 'Thanks!', citations: [] }],
        created_at: '2025-06-01T10:05:00.000000Z', updated_at: '2025-06-01T10:05:00.000000Z', attachments: [], files: [],
      },
    ],
  },
  {
    uuid: 'c2222222-0000-0000-0000-000000000002', name: '',
    created_at: '2025-06-02T08:00:00.000000Z', updated_at: '2025-06-02T08:00:09.000000Z', account,
    chat_messages: [
      { uuid: 'n1', sender: 'human', text: 'Hi', content: [{ type: 'text', text: 'Hi' }], created_at: '2025-06-02T08:00:01.000000Z', attachments: [], files: [] },
      { uuid: 'n2', sender: 'assistant', text: 'Hello!', content: [{ type: 'text', text: 'Hello!' }], created_at: '2025-06-02T08:00:09.000000Z', attachments: [], files: [] },
    ],
  },
];

export const cases: ConformanceCase[] = [{
  name: 'conversations-json-export',
  detection: 'content',
  async setup(home) {
    const dir = join(home, 'Downloads', 'data-2025-06-03-claude');
    await mkdir(dir, { recursive: true });
    const file = join(dir, 'conversations.json');
    await writeFile(file, JSON.stringify(exportData));
    return file;
  },
}];
