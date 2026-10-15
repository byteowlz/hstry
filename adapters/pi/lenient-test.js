// Regression: indexing must be lenient per session. A session Pi is still
// writing (partial trailing line) or one with non-native content must stay
// searchable, must not abort sibling sessions, and must be marked as not
// qualified for native Pi export with explicit warnings.
// Usage: bun adapters/pi/lenient-test.js

import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const home = await mkdtemp(join(tmpdir(), 'hstry-pi-lenient-'));
const sessions = join(home, '.pi', 'agent', 'sessions', '--work--');
const header = id => ({ type: 'session', version: 3, id, timestamp: '2026-01-01T00:00:00.000Z', cwd: '/work' });
const message = (id, parentId, msg) => ({ type: 'message', id, parentId, timestamp: '2026-01-01T00:00:01.000Z', message: { timestamp: 1767225601000, ...msg } });
const jsonl = entries => entries.map(entry => JSON.stringify(entry)).join('\n') + '\n';

try {
  await mkdir(sessions, { recursive: true });
  const user = { role: 'user', content: [{ type: 'text', text: 'lenient-token question' }] };
  // Valid session.
  await writeFile(join(sessions, 'a.jsonl'), jsonl([header('valid'), message('e1', null, user)]));
  // Still being written: partial trailing line.
  await writeFile(join(sessions, 'b.jsonl'), jsonl([header('partial'), message('e1', null, user)]) + '{"type":"message","id":"e2","par');
  // Non-native content (Anthropic-style tool_result block) and a tool call without ID.
  await writeFile(join(sessions, 'c.jsonl'), jsonl([
    header('hybrid'),
    message('e1', null, { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'x', content: 'hybrid-result' }] }),
    message('e2', 'e1', { role: 'assistant', content: [{ type: 'toolCall', name: 'read', arguments: {} }], provider: 'p', model: 'm', api: 'a', stopReason: 'toolUse', usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } } }),
  ]));
  // No header: skipped without affecting the others.
  await writeFile(join(sessions, 'd.jsonl'), jsonl([message('e1', null, user)]));

  const result = Bun.spawnSync(['bun', 'run', new URL('./adapter.ts', import.meta.url).pathname], {
    env: { ...process.env, HOME: home, HSTRY_REQUEST: JSON.stringify({ method: 'parse', params: { path: join(home, '.pi', 'agent', 'sessions'), opts: {} } }) },
  });
  assert.ok(result.success, result.stderr.toString());
  const parsed = JSON.parse(result.stdout.toString());
  assert.ok(!('error' in parsed), JSON.stringify(parsed));
  const byId = Object.fromEntries(parsed.map(conversation => [conversation.externalId, conversation]));
  assert.deepEqual(Object.keys(byId).sort(), ['hybrid', 'partial', 'valid']);

  assert.deepEqual(byId.valid.metadata.nativeQualification, { qualified: true });
  assert.equal(byId.valid.metadata.parseWarnings, undefined);
  assert.ok(byId.valid.metadata.nativePiSession);

  assert.equal(byId.partial.messages.length, 1);
  assert.equal(byId.partial.metadata.nativeQualification.qualified, false);
  assert.equal(byId.partial.metadata.nativePiSession, undefined);
  assert.match(byId.partial.metadata.parseWarnings[0], /incomplete trailing line/);

  assert.equal(byId.hybrid.metadata.nativeQualification.qualified, false);
  assert.equal(byId.hybrid.metadata.nativePiSession, undefined);
  const parts = byId.hybrid.messages.flatMap(m => m.parts ?? []);
  assert.ok(parts.some(part => part.type === 'x-pi-content' && part.payload.type === 'toolCall'), 'id-less tool call kept opaque');
  assert.ok(!parts.some(part => part.type === 'tool_call'), 'no fabricated tool association');

  console.log('pi lenient indexing test passed');
} finally {
  await rm(home, { recursive: true, force: true });
}
