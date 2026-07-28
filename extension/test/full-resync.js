// Integration-style service worker test for targeted provider full resync.
// Usage: bun extension/test/full-resync.js

import assert from 'node:assert/strict';

const nowSeconds = Date.now() / 1000;
const oldWatermark = Date.now() + 86_400_000;
const stored = {
  settings: {
    port: 3000,
    token: '',
    intervalMinutes: 15,
    providers: { chatgpt: true, claude: false, gemini: false, perplexity: false },
  },
  status: {
    chatgpt: {
      state: { accounts: { default: { lastSyncMs: oldWatermark } } },
      lastRunMs: 1,
    },
    claude: { state: { sentinel: 'untouched' }, lastRunMs: 2 },
    gemini: { state: { sentinel: 'stale-run' }, lastRunMs: 3, running: true },
  },
};

let messageListener;
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

const requests = [];
globalThis.fetch = async (url, init = {}) => {
  const parsed = new URL(url);
  requests.push({ pathname: parsed.pathname, method: init.method ?? 'GET' });

  if (parsed.hostname === '127.0.0.1') {
    if (parsed.pathname === '/sources') return Response.json({ ok: true });
    if (parsed.pathname === '/ingest') {
      const payload = JSON.parse(init.body);
      assert.equal(payload.source, 'chatgpt-web');
      assert.equal(payload.conversations.length, 1);
      return Response.json({ conversations: 1, created: 0, updated: 1 });
    }
  }
  if (parsed.pathname === '/api/auth/session') {
    return Response.json({ accessToken: 'fixture-token' });
  }
  if (parsed.pathname === '/backend-api/accounts/check/v4-2023-04-27') {
    return Response.json({ accounts: {} });
  }
  if (parsed.pathname === '/backend-api/conversations') {
    return Response.json({
      total: 1,
      items: [{ id: 'full-sync-conversation', update_time: nowSeconds }],
    });
  }
  if (parsed.pathname === '/backend-api/conversation/full-sync-conversation') {
    return Response.json({
      current_node: 'answer',
      mapping: {
        question: {
          id: 'question',
          parent: null,
          message: {
            author: { role: 'user' },
            create_time: nowSeconds - 1,
            content: { content_type: 'text', parts: ['repair this conversation'] },
          },
        },
        answer: {
          id: 'answer',
          parent: 'question',
          message: {
            author: { role: 'assistant' },
            create_time: nowSeconds,
            content: { content_type: 'text', parts: ['repaired'] },
          },
        },
      },
    });
  }
  return new Response('not found', { status: 404 });
};

await import(`../background.js?full-resync-test=${Date.now()}`);
assert.ok(messageListener, 'service worker registered a message listener');

const response = await new Promise(resolve => {
  const asyncResponse = messageListener(
    { type: 'fullSyncProvider', provider: 'chatgpt' },
    {},
    resolve
  );
  assert.equal(asyncResponse, true);
});
assert.deepEqual(response, { ok: true });

for (let attempt = 0; attempt < 100 && stored.status.chatgpt.running !== false; attempt++) {
  await new Promise(resolve => setTimeout(resolve, 5));
}

assert.equal(stored.status.chatgpt.running, false);
assert.equal(stored.status.chatgpt.trigger, 'full');
assert.equal(stored.status.chatgpt.lastError, null);
assert.notEqual(
  stored.status.chatgpt.state.accounts.default.lastSyncMs,
  oldWatermark,
  'full resync replaces the previous watermark'
);
assert.deepEqual(stored.status.claude, {
  state: { sentinel: 'untouched' },
  lastRunMs: 2,
});
assert.deepEqual(stored.status.gemini, {
  state: { sentinel: 'stale-run' },
  lastRunMs: 3,
  running: false,
  lastError: 'Previous sync was interrupted. Run it again.',
  progress: { phase: 'failed' },
});
assert.ok(
  requests.some(request => request.pathname === '/backend-api/conversation/full-sync-conversation'),
  'full resync fetched conversation detail despite the future watermark'
);
assert.equal(
  requests.filter(request => request.pathname === '/ingest').length,
  1,
  'only the selected provider was ingested'
);

console.log('PASS targeted provider full resync resets only its watermark');
