// Integration test for the hstry sync extension providers.
//
// Stubs the platform APIs (chatgpt.com, claude.ai) with fixtures, then runs
// the real provider modules and pushes through a real hstry-api /ingest into
// a real database. Usage:
//
//   hstry-api --config <tmp>/config.toml --port 39132 --token testtoken &
//   bun extension/test/run.js 39132 testtoken

import { syncChatGPT } from '../providers/chatgpt.js';
import { syncClaude } from '../providers/claude.js';

const PORT = process.argv[2] ?? '39132';
const TOKEN = process.argv[3] ?? 'testtoken';

const NOW_S = Date.now() / 1000;
const NOW_ISO = new Date().toISOString();
// Updates must be older than the providers' 5-minute overlap window, or run 2
// legitimately re-fetches them (idempotent by server-side dedupe, but noisy
// for this test).
const OLD_S = NOW_S - 3600;
const OLD_ISO = new Date((NOW_S - 3600) * 1000).toISOString();

const FIXTURES = {
  'chatgpt.com': {
    '/api/auth/session': { accessToken: 'fixture-token' },
    '/backend-api/accounts/check/v4-2023-04-27': {
      accounts: {
        default: {
          account: { account_id: 'acc-personal', plan_type: 'plus', structure: 'personal' },
        },
        'team-1': {
          account: { account_id: 'acc-team1', organization_name: 'Fraunhofer', structure: 'workspace' },
        },
      },
    },
    '/backend-api/conversations': {
      total: 1,
      items: [
        { id: 'gpt-conv-1', title: 'GPT fixture chat', create_time: OLD_S, update_time: OLD_S },
      ],
    },
    '/backend-api/conversation/gpt-conv-1': {
      title: 'GPT fixture chat',
      create_time: OLD_S,
      update_time: OLD_S,
      current_node: 'n3',
      mapping: {
        n1: { id: 'n1', parent: null, message: { author: { role: 'system' }, content: { content_type: 'text', parts: [''] } } },
        n2: {
          id: 'n2',
          parent: 'n1',
          message: {
            author: { role: 'user' },
            create_time: NOW_S - 3600,
            content: { content_type: 'text', parts: ['hello from fixture'] },
          },
        },
        n3: {
          id: 'n3',
          parent: 'n2',
          message: {
            author: { role: 'assistant' },
            create_time: NOW_S - 3500,
            metadata: { model_slug: 'gpt-5-3' },
            content: { content_type: 'text', parts: ['fixture response'] },
          },
        },
      },
    },
  },
  'claude.ai': {
    '/api/organizations': [{ uuid: 'org-fixture-1', name: 'Personal', capabilities: ['chat'] }],
    '/api/organizations/org-fixture-1/chat_conversations': [
      { uuid: 'claude-conv-1', name: 'Claude fixture chat', created_at: OLD_ISO, updated_at: OLD_ISO },
    ],
    '/api/organizations/org-fixture-1/chat_conversations/claude-conv-1': {
      uuid: 'claude-conv-1',
      name: 'Claude fixture chat',
      created_at: OLD_ISO,
      updated_at: OLD_ISO,
      chat_messages: [
        { sender: 'human', created_at: NOW_ISO, content: [{ type: 'text', text: 'hi claude' }] },
        {
          sender: 'assistant',
          created_at: NOW_ISO,
          content: [
            { type: 'thinking', thinking: 'pondering' },
            { type: 'text', text: 'hello from claude fixture' },
            { type: 'tool_use', id: 't1', name: 'web_search', input: { q: 'hstry' } },
            { type: 'tool_result', tool_use_id: 't1', content: 'result blob' },
          ],
        },
      ],
    },
  },
};

const realFetch = globalThis.fetch;
globalThis.fetch = async (url, init = {}) => {
  const parsed = new URL(url);
  if (parsed.hostname === '127.0.0.1' || parsed.hostname === 'localhost') {
    return realFetch(url, init);
  }
  const fixture = FIXTURES[parsed.hostname]?.[parsed.pathname];
  if (fixture === undefined) {
    return new Response('not found', { status: 404 });
  }
  return new Response(JSON.stringify(fixture), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};

const pushes = [];
async function push(sourceId, adapter, conversations) {
  pushes.push({ sourceId, count: conversations.length });
  const res = await realFetch(`http://127.0.0.1:${PORT}/ingest`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${TOKEN}` },
    body: JSON.stringify({ source: sourceId, adapter, conversations }),
  });
  if (!res.ok) throw new Error(`/ingest -> ${res.status}: ${await res.text()}`);
  const data = await res.json();
  return data.conversations;
}

const log = message => console.log(`  [provider] ${message}`);
let failures = 0;
function check(label, actual, expected) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  console.log(`${ok ? 'PASS' : 'FAIL'} ${label}${ok ? '' : ` (got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)})`}`);
  if (!ok) failures++;
}

// --- First run: full sync ---
const gpt1 = await syncChatGPT({ state: {}, push, log });
const claude1 = await syncClaude({ state: {}, push, log });

check('chatgpt run 1 conversations', gpt1.conversations, 2); // personal + workspace account
check('claude run 1 conversations', claude1.conversations, 1);
check(
  'source ids',
  pushes.map(p => p.sourceId).sort(),
  ['chatgpt-web', 'chatgpt-web-accteam1', 'claude-web']
);

// --- Second run: incremental, nothing new ---
pushes.length = 0;
const gpt2 = await syncChatGPT({ state: gpt1.state, push, log });
const claude2 = await syncClaude({ state: claude1.state, push, log });
check('chatgpt run 2 conversations (incremental)', gpt2.conversations, 0);
check('claude run 2 conversations (incremental)', claude2.conversations, 0);
check('run 2 pushes', pushes.length, 0);

// --- Verify what landed in hstry via the search API ---
const hits = await (
  await realFetch(`http://127.0.0.1:${PORT}/search?query=fixture&limit=10`)
).json();
const sources = [...new Set(hits.map(h => h.source_id))].sort();
check('search finds all sources', sources, ['chatgpt-web', 'chatgpt-web-accteam1', 'claude-web']);
const gptHit = hits.find(h => h.source_id === 'chatgpt-web' && h.role === 'assistant');
check('gpt assistant content', gptHit?.content, 'fixture response');

console.log(failures === 0 ? '\nAll checks passed.' : `\n${failures} check(s) FAILED.`);
process.exit(failures === 0 ? 0 : 1);
