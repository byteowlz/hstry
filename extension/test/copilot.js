// Regression test for the M365 Copilot Chat provider.
// Usage: bun extension/test/copilot.js
//
// Stubs the m365.cloud.microsoft data endpoints and asserts the provider
// lists conversations, pulls each detail, and shapes user/assistant messages
// (assistant text from adaptive cards) into the hstry conversation shape.

import assert from 'node:assert/strict';
import { syncCopilot } from '../providers/copilot.js';

const CONV_ID = 'copilot-conv-1';
const LIST = {
  store: {
    conversationPageHistoryList: {
      chats: [
        {
          conversationId: CONV_ID,
          chatName: 'Fixture Copilot chat',
          createTimeUtc: 1700000000000,
          updateTimeUtc: 1700000060000,
          threadId: '19:fixture@thread.v2',
        },
      ],
    },
  },
};

const DETAIL = {
  store: {
    rawConversationResponse: {
      messages: [
        {
          author: 'user',
          text: 'Draft an update for the Q3 review.',
          createdAt: '2023-11-14T10:13:20.0000000+00:00',
          messageId: 'u1',
        },
        {
          author: 'bot',
          createdAt: '2023-11-14T10:13:32.0000000+00:00',
          messageId: 'b1',
          adaptiveCards: [
            { body: [{ type: 'TextBlock', text: '**Draft:**\n\nQuarterly progress is on track.' }] },
            { body: [{ type: 'TextBlock', text: 'Second card paragraph.' }] },
          ],
        },
      ],
    },
  },
};

const realFetch = globalThis.fetch;
globalThis.fetch = async (url, init = {}) => {
  const parsed = new URL(url);
  if (parsed.hostname !== 'm365.cloud.microsoft') return new Response('nope', { status: 404 });
  if (parsed.pathname === '/chat' && init.method === 'POST') {
    return Response.json(LIST);
  }
  if (parsed.pathname === `/chat/conversation/${CONV_ID}`) {
    return Response.json(DETAIL);
  }
  return new Response('unknown', { status: 404 });
};

try {
  const captured = [];
  const result = await syncCopilot({
    state: {},
    log: () => {},
    register: async () => {},
    push: async (sourceId, adapter, conversations) => {
      captured.push({ sourceId, adapter, conversations });
      return conversations.length;
    },
  });

  assert.equal(result.conversations, 1, 'one conversation ingested');
  assert.ok(Number.isFinite(result.state.lastSyncMs), 'watermark set on a clean first run');
  assert.ok(result.state.lastSyncMs >= 1700000060000, 'watermark advances past imported chat');
  assert.equal(captured.length, 1);
  assert.equal(captured[0].sourceId, 'm365-copilot');
  assert.equal(captured[0].adapter, 'm365-copilot');

  const conv = captured[0].conversations[0];
  assert.equal(conv.externalId, CONV_ID);
  assert.equal(conv.title, 'Fixture Copilot chat');
  assert.equal(conv.metadata.url, `https://m365.cloud.microsoft/chat/conversation/${CONV_ID}`);
  assert.deepEqual(
    conv.messages.map(m => [m.role, m.content]),
    [
      ['user', 'Draft an update for the Q3 review.'],
      ['assistant', '**Draft:**\n\nQuarterly progress is on track.\n\nSecond card paragraph.'],
    ]
  );
  // createTimeUtc is in ms; createdAt must be derived, not the ISO message time.
  assert.equal(conv.createdAt, 1700000000000);
  assert.equal(conv.updatedAt, 1700000060000);

  // Second run with a fresh watermark skips the unchanged conversation.
  const second = await syncCopilot({
    state: { lastSyncMs: Date.now() },
    log: () => {},
    register: async () => {},
    push: async () => 0,
  });
  assert.equal(second.conversations, 0, 'incremental sync skips stale chats');

  console.log('PASS m365 copilot provider list/detail shaping and incremental skip');
} finally {
  globalThis.fetch = realFetch;
}