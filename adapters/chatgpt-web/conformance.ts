// Format checked against the OpenAI ChatGPT data export (Settings > Data controls > Export data)
// conversations.json as of 2026: array of {title, create_time, update_time, mapping{node-id: {id,
// message, parent, children}}, current_node, conversation_id, id, default_model_slug, ...};
// messages are {author{role,name}, content{content_type: text|code|execution_output|thoughts|
// reasoning_recap|multimodal_text, ...}, metadata{model_slug,...}, recipient}. The structure is
// reproduced synthetically from the documented/observed export layout (no upstream source repo).
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const base = 1738000000;

function node(id: string, parent: string | null, children: string[], message: unknown) {
  return { id, message, parent, children };
}

function message(id: string, role: string, content: unknown, time: number | null, metadata: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) {
  return {
    id, author: { role, name: null, metadata: {} }, create_time: time, update_time: null, content,
    status: 'finished_successfully', end_turn: role === 'assistant' ? true : null, weight: 1, metadata, recipient: 'all', channel: null, ...extra,
  };
}

const conversation = {
  title: 'Parse a CSV file',
  create_time: base + 0.5,
  update_time: base + 95.25,
  mapping: {
    'client-created-root': node('client-created-root', null, ['sys-1'], null),
    'sys-1': node('sys-1', 'client-created-root', ['usr-1'],
      message('sys-1', 'system', { content_type: 'text', parts: [''] }, null, { is_visually_hidden_from_conversation: true }, { weight: 0, end_turn: true })),
    'usr-1': node('usr-1', 'sys-1', ['think-1'],
      message('usr-1', 'user', { content_type: 'multimodal_text', parts: ['Parse this CSV and sum column b', { content_type: 'image_asset_pointer', asset_pointer: 'file-service://file-abc123', size_bytes: 1024, width: 64, height: 64 }] }, base + 1.125,
        { request_id: 'req-1', message_source: null, timestamp_: 'absolute' })),
    'think-1': node('think-1', 'usr-1', ['recap-1'],
      message('think-1', 'assistant', { content_type: 'thoughts', thoughts: [{ summary: 'Planning', content: 'Use python to read the CSV.', chunks: [], finished: true }], source_analysis_msg_id: 'x' }, base + 2.5, { model_slug: 'gpt-5', reasoning_status: 'is_reasoning' }, { end_turn: false })),
    'recap-1': node('recap-1', 'think-1', ['code-1'],
      message('recap-1', 'assistant', { content_type: 'reasoning_recap', content: 'Thought for 3s' }, base + 5, { model_slug: 'gpt-5' }, { end_turn: false })),
    'code-1': node('code-1', 'recap-1', ['out-1'],
      message('code-1', 'assistant', { content_type: 'code', language: 'unknown', response_format_name: null, text: "import pandas as pd\nprint(pd.read_csv('/mnt/data/a.csv')['b'].sum())" }, base + 6, { model_slug: 'gpt-5' }, { recipient: 'python', end_turn: false })),
    'out-1': node('out-1', 'code-1', ['asst-1'],
      message('out-1', 'tool', { content_type: 'execution_output', text: '42' }, base + 8, { model_slug: 'gpt-5' }, { recipient: 'all' })),
    'asst-1': node('asst-1', 'out-1', [],
      message('asst-1', 'assistant', { content_type: 'text', parts: ['The sum of column **b** is 42.'] }, base + 9.75, { model_slug: 'gpt-5', finish_details: { type: 'stop', stop_tokens: [200002] } })),
  },
  moderation_results: [],
  current_node: 'asst-1',
  plugin_ids: null,
  conversation_id: 'conv-0001',
  conversation_template_id: null,
  gizmo_id: null,
  is_archived: false,
  default_model_slug: 'gpt-5',
  id: 'conv-0001',
};

const second = {
  title: 'Haiku',
  create_time: base + 1000,
  update_time: base + 1010,
  mapping: {
    'client-created-root': node('client-created-root', null, ['u1'], null),
    u1: node('u1', 'client-created-root', ['a1'], message('u1', 'user', { content_type: 'text', parts: ['Write a haiku about rain'] }, base + 1001)),
    a1: node('a1', 'u1', [], message('a1', 'assistant', { content_type: 'text', parts: ['Soft rain on the roof\nthe garden drinks in silence\nmorning smells of earth'] }, base + 1005, { model_slug: 'gpt-4o' })),
  },
  current_node: 'a1',
  conversation_id: 'conv-0002',
  default_model_slug: 'gpt-4o',
  id: 'conv-0002',
};

export const cases: ConformanceCase[] = [{
  name: 'conversations-json-export',
  detection: 'content',
  async setup(home) {
    const dir = join(home, 'Downloads', 'chatgpt-export');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'conversations.json'), JSON.stringify([conversation, second]));
    return join(dir, 'conversations.json');
  },
}];
