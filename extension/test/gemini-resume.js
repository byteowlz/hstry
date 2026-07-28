// Regression test: large Gemini syncs resume in bounded service-worker chunks.
// Usage: bun extension/test/gemini-resume.js

import assert from 'node:assert/strict';
import { syncGemini } from '../providers/gemini.js';

function rpc(rpcId, data) {
  return `)]}'\n${JSON.stringify([["wrb.fr", rpcId, JSON.stringify(data), null]])}\n`;
}

const nowSec = Math.floor(Date.now() / 1000) - 3600;
const summaries = Array.from({ length: 12 }, (_, index) => [
  `c_chat-${index}`,
  `Chat ${index}`,
  null,
  null,
  null,
  [nowSec - index, 0],
]);
let listCalls = 0;

globalThis.fetch = async (url, init = {}) => {
  const parsed = new URL(url);
  if (parsed.pathname === '/app') {
    return new Response('<script>{"SNlM0e":"csrf","FdrFJe":"sid","cfb2h":"build"}</script>');
  }
  const rpcId = parsed.searchParams.get('rpcids');
  if (rpcId === 'MaZiqc') {
    listCalls++;
    return new Response(rpc('MaZiqc', [null, null, summaries]));
  }
  if (rpcId === 'hNvQHb') {
    const request = JSON.parse(init.body.get('f.req'));
    const [chatId] = JSON.parse(request[0][0][1]);
    const turn = [null, null, [[`question ${chatId}`]], [[[null, [`answer ${chatId}`]]]]];
    return new Response(rpc('hNvQHb', [[turn], null]));
  }
  throw new Error(`unexpected request: ${url}`);
};

async function run(state) {
  const pushed = [];
  const result = await syncGemini({
    state,
    register: async () => {},
    report: async () => {},
    log: () => {},
    push: async (_source, _adapter, conversations) => {
      pushed.push(...conversations);
      return conversations.length;
    },
  });
  return { result, pushed };
}

const first = await run({});
assert.equal(first.result.conversations, 10);
assert.equal(first.result.hasMore, true);
assert.equal(first.result.state.pending.nextIndex, 10);
assert.equal(first.pushed.length, 10);

const second = await run(first.result.state);
assert.equal(second.result.conversations, 2);
assert.equal(second.result.hasMore, false);
assert.equal(second.result.state.pending, undefined);
assert.equal(second.pushed.length, 2);
assert.equal(listCalls, 1, 'continuation reuses the persisted summary queue');

console.log('PASS Gemini resumes large syncs in bounded chunks');
