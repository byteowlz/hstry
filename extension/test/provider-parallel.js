// Regression test: a slow/rate-limited provider must not block another provider.
// Usage: bun extension/test/provider-parallel.js

import assert from 'node:assert/strict';

const now = new Date(Date.now() - 3_600_000).toISOString();
const nowSec = Date.now() / 1000 - 3600;
const stored = {
  settings: {
    port: 3000,
    token: '',
    intervalMinutes: 15,
    providers: { chatgpt: true, claude: true, gemini: false, perplexity: false },
  },
  status: {},
};
let messageListener;
let resolveChatGptDetail;
let claudeDetailSeen = false;

globalThis.chrome = {
  action: {
    onClicked: { addListener: () => {} },
    setBadgeText: async () => {},
    setBadgeBackgroundColor: async () => {},
  },
  alarms: {
    get: async () => null,
    create: async () => {},
    onAlarm: { addListener: () => {} },
  },
  runtime: {
    onInstalled: { addListener: () => {} },
    onStartup: { addListener: () => {} },
    onMessage: { addListener: listener => (messageListener = listener) },
  },
  storage: {
    local: {
      get: async keys => {
        const names = Array.isArray(keys) ? keys : [keys];
        return Object.fromEntries(names.filter(name => name in stored).map(name => [name, stored[name]]));
      },
      set: async values => Object.assign(stored, structuredClone(values)),
    },
  },
};

globalThis.fetch = async (url, init = {}) => {
  const parsed = new URL(url);
  if (parsed.hostname === '127.0.0.1') {
    if (parsed.pathname === '/sources') return Response.json({ ok: true });
    if (parsed.pathname === '/ingest') {
      const payload = JSON.parse(init.body);
      return Response.json({ conversations: payload.conversations.length, created: 1, updated: 0 });
    }
  }
  if (parsed.hostname === 'chatgpt.com') {
    if (parsed.pathname === '/api/auth/session') return Response.json({ accessToken: 'token' });
    if (parsed.pathname.includes('/accounts/check/')) return Response.json({ accounts: {} });
    if (parsed.pathname === '/backend-api/conversations') {
      return Response.json({ total: 1, items: [{ id: 'slow-chat', update_time: nowSec }] });
    }
    if (parsed.pathname === '/backend-api/conversation/slow-chat') {
      return new Promise(resolve => (resolveChatGptDetail = resolve));
    }
  }
  if (parsed.hostname === 'claude.ai') {
    if (parsed.pathname === '/api/organizations') {
      return Response.json({ organizations: [{ id: 'accessible-org' }] });
    }
    if (parsed.pathname === '/api/organizations/accessible-org/chat_conversations') {
      return Response.json([{ uuid: 'claude-chat', updated_at: now }]);
    }
    if (parsed.pathname === '/api/organizations/accessible-org/chat_conversations/claude-chat') {
      claudeDetailSeen = true;
      return Response.json({
        uuid: 'claude-chat',
        created_at: now,
        updated_at: now,
        chat_messages: [
          { sender: 'human', created_at: now, content: [{ type: 'text', text: 'hello' }] },
          { sender: 'assistant', created_at: now, content: [{ type: 'text', text: 'response' }] },
        ],
      });
    }
  }
  return new Response('not found', { status: 404 });
};

await import(`../background.js?parallel-test=${Date.now()}`);
messageListener({ type: 'syncNow' }, {}, () => {});
for (let attempt = 0; attempt < 100 && !claudeDetailSeen; attempt++) {
  await new Promise(resolve => setTimeout(resolve, 5));
}

assert.equal(claudeDetailSeen, true, 'Claude progressed while ChatGPT detail remained pending');
assert.equal(stored.status.claude.running, false);
assert.equal(stored.status.claude.lastError, null);
assert.equal(stored.status.chatgpt.running, true);

resolveChatGptDetail(
  Response.json({
    current_node: 'message',
    mapping: {
      message: {
        id: 'message',
        parent: null,
        message: {
          author: { role: 'user' },
          create_time: nowSec,
          content: { content_type: 'text', parts: ['slow message'] },
        },
      },
    },
  })
);

console.log('PASS providers sync independently in parallel');
