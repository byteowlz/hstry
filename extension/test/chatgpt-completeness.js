// Regression test for broken ChatGPT mapping chains and new content types.
// Usage: bun extension/test/chatgpt-completeness.js

import assert from 'node:assert/strict';
import { syncChatGPT } from '../providers/chatgpt.js';

const detail = await Bun.file(
  new URL('./fixtures/chatgpt-broken-chain.json', import.meta.url)
).json();

const realFetch = globalThis.fetch;
globalThis.fetch = async url => {
  const { pathname } = new URL(url);
  let fixture;
  if (pathname === '/api/auth/session') {
    fixture = { accessToken: 'fixture-token' };
  } else if (pathname === '/backend-api/accounts/check/v4-2023-04-27') {
    fixture = {
      accounts: {
        default: {
          account: { account_id: 'fixture-account', structure: 'personal' },
        },
      },
    };
  } else if (pathname === '/backend-api/conversations') {
    fixture = {
      total: 1,
      items: [
        {
          id: detail.id,
          title: detail.title,
          create_time: detail.create_time,
          update_time: detail.update_time,
        },
      ],
    };
  } else if (pathname === `/backend-api/conversation/${detail.id}`) {
    fixture = detail;
  } else {
    return new Response('not found', { status: 404 });
  }

  return Response.json(fixture);
};

async function captureSync() {
  const captured = [];
  const result = await syncChatGPT({
    state: {},
    log: () => {},
    push: async (sourceId, adapter, conversations) => {
      captured.push({ sourceId, adapter, conversations });
      return conversations.length;
    },
  });
  assert.equal(result.conversations, 1);
  assert.equal(captured.length, 1);
  return captured[0].conversations[0];
}

try {
  const first = await captureSync();
  const second = await captureSync();

  assert.deepEqual(
    first.messages.map(message => [message.role, message.content]),
    [
      ['user', 'first question'],
      ['assistant', 'execution result'],
      ['user', 'quoted follow-up'],
      ['assistant', 'regenerated alternate'],
      ['assistant', 'future-compatible answer'],
      ['user', 'metadata-only context'],
      ['assistant', 'tail answer'],
    ]
  );
  const stableShape = messages =>
    messages.map(message => ({
      role: message.role,
      content: message.content,
      createdAt: message.createdAt,
      model: message.model,
      parts: message.parts.map(({ id: _id, ...part }) => part),
    }));
  assert.deepEqual(
    stableShape(second.messages),
    stableShape(first.messages),
    'repeated sync ordering and content must be stable'
  );
  console.log('PASS chatgpt broken-chain completeness and stable ordering');
} finally {
  globalThis.fetch = realFetch;
}
