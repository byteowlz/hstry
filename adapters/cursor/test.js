// Regression test for Cursor adapter SQLite access under Bun.
// Usage: bun adapters/cursor/test.js

import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Database } from 'bun:sqlite';

const root = await mkdtemp(join(tmpdir(), 'hstry-cursor-'));
const workspace = join(root, 'fixture-workspace');
await mkdir(workspace);
const db = new Database(join(workspace, 'state.vscdb'), { create: true });
db.run('CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)');
db.query('INSERT INTO ItemTable (key, value) VALUES (?, ?)').run(
  'workbench.panel.aichat.view.aichat.chatdata',
  JSON.stringify({
    tabs: [{
      id: 'cursor-fixture',
      title: 'Cursor fixture',
      createdAt: 1_700_000_000_000,
      bubbles: [
        { type: 'user', text: 'fixture question' },
        { type: 'assistant', text: 'fixture response', modelType: 'fixture-model' },
      ],
    }],
  })
);
db.close();

try {
  const child = Bun.spawn([process.execPath, 'run', resolve('adapters/cursor/adapter.ts')], {
    env: {
      ...process.env,
      HSTRY_REQUEST: JSON.stringify({ method: 'parse', params: { path: root, opts: {} } }),
    },
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  assert.equal(exitCode, 0, stderr);
  const conversations = JSON.parse(stdout);
  assert.equal(conversations.length, 1);
  assert.equal(conversations[0].externalId, 'cursor-fixture');
  assert.deepEqual(
    conversations[0].messages.map(message => message.content),
    ['fixture question', 'fixture response']
  );
  console.log('PASS Cursor adapter uses Bun native SQLite without crashing');
} finally {
  await rm(root, { recursive: true, force: true });
}
