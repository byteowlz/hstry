// Regression test: one inaccessible Claude organization must not abort others.
// Usage: bun extension/test/claude-organizations.js

import assert from 'node:assert/strict';
import { syncClaude } from '../providers/claude.js';

const now = new Date(Date.now() - 3_600_000).toISOString();
globalThis.fetch = async url => {
  const { pathname } = new URL(url);
  if (pathname === '/api/organizations') {
    return Response.json({
      organizations: [
        { id: 'denied-org', capabilities: ['claude_pro'] },
        { id: 'accessible-org', capabilities: ['claude_pro'] },
      ],
    });
  }
  if (pathname === '/api/organizations/denied-org/chat_conversations') {
    return Response.json({ error: 'forbidden' }, { status: 403 });
  }
  if (pathname === '/api/organizations/accessible-org/chat_conversations') {
    return Response.json([{ uuid: 'accessible-chat', updated_at: now }]);
  }
  if (pathname === '/api/organizations/accessible-org/chat_conversations/accessible-chat') {
    return Response.json({
      uuid: 'accessible-chat',
      name: 'Accessible fixture',
      created_at: now,
      updated_at: now,
      chat_messages: [
        { sender: 'human', created_at: now, content: [{ type: 'text', text: 'hello' }] },
        { sender: 'assistant', created_at: now, content: [{ type: 'text', text: 'response' }] },
      ],
    });
  }
  return new Response('not found', { status: 404 });
};

const pushed = [];
const logs = [];
const result = await syncClaude({
  state: {},
  register: async () => {},
  report: async () => {},
  log: message => logs.push(message),
  push: async (source, adapter, conversations) => {
    pushed.push({ source, adapter, conversations });
    return conversations.length;
  },
});

assert.equal(result.conversations, 1);
assert.equal(pushed.length, 1);
assert.equal(pushed[0].source, 'claude-web-accessib');
assert.deepEqual(
  pushed[0].conversations[0].messages.map(message => message.content),
  ['hello', 'response']
);
assert.ok(logs.some(message => message.includes('skipping organization deniedor')));
assert.equal(result.state.orgs.deniedor.lastSyncMs, null);
assert.ok(result.state.orgs.accessib.lastSyncMs > 0);

console.log('PASS Claude continues after an inaccessible organization');
