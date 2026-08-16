// Regression test for Cursor adapter SQLite access under Bun.
// Usage: bun adapters/cursor/test.js

import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Database } from 'bun:sqlite';
import { gzipSync } from 'node:zlib';

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

const globalStorage = join(root, 'globalStorage');
await mkdir(globalStorage);
const globalDb = new Database(join(globalStorage, 'state.vscdb'), { create: true });
globalDb.run('CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)');
globalDb.query('INSERT INTO ItemTable (key, value) VALUES (?, ?)').run(
  'composer.composerHeaders',
  JSON.stringify({
    allComposers: [{
      composerId: 'composer-fixture',
      name: 'Composer fixture',
      createdAt: 1_700_000_001,
      workspaceIdentifier: { uri: { fsPath: '/fixture/project' } },
    }],
  })
);
globalDb.query('INSERT INTO ItemTable (key, value) VALUES (?, ?)').run(
  'composerData:composer-fixture',
  JSON.stringify({
    createdAt: 1_700_000_001,
    lastUpdatedAt: 1_700_000_003,
    fullConversationHeadersOnly: [
      { bubbleId: 'user-bubble', type: 1 },
      { bubbleId: 'assistant-bubble', type: 2 },
    ],
  })
);
globalDb.query('INSERT INTO ItemTable (key, value) VALUES (?, ?)').run(
  'bubbleId:composer-fixture:user-bubble',
  JSON.stringify({ type: 1, text: 'composer question', createdAt: 1_700_000_001 })
);
globalDb.query('INSERT INTO ItemTable (key, value) VALUES (?, ?)').run(
  'bubbleId:composer-fixture:assistant-bubble',
  JSON.stringify({
    type: 2,
    text: 'composer response',
    createdAt: 1_700_000_003,
    toolResults: [{ name: 'read_file', params: { path: 'README.md' }, result: 'contents' }],
  })
);
globalDb.close();

const snapshots = join(root, 'snapshots');
await mkdir(snapshots);
await writeFile(
  join(snapshots, 'snapshot-fixture.json.gz'),
  gzipSync(JSON.stringify({
    composerId: 'snapshot-fixture',
    sourceProjectPath: '/fixture/snapshot-project',
    composerData: {
      name: 'Snapshot fixture',
      createdAt: 1_700_000_010,
      fullConversationHeadersOnly: [
        { bubbleId: 'snapshot-user', type: 1 },
        { bubbleId: 'snapshot-assistant', type: 2 },
      ],
    },
    bubbleEntries: {
      'snapshot-user': { type: 1, text: 'snapshot question', createdAt: 1_700_000_010 },
      'snapshot-assistant': { type: 2, text: 'snapshot response', createdAt: 1_700_000_011 },
    },
  }))
);

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
  assert.equal(conversations.length, 3);
  const legacy = conversations.find(conversation => conversation.externalId === 'cursor-fixture');
  assert.deepEqual(
    legacy.messages.map(message => message.content),
    ['fixture question', 'fixture response']
  );
  const composer = conversations.find(conversation => conversation.externalId === 'composer-fixture');
  assert.equal(composer.title, 'Composer fixture');
  assert.equal(composer.workspace, '/fixture/project');
  assert.deepEqual(
    composer.messages.map(message => message.content),
    ['composer question', 'composer response']
  );
  assert.equal(composer.messages[1].toolCalls[0].toolName, 'read_file');
  assert.equal(composer.createdAt, 1_700_000_001_000);
  const snapshot = conversations.find(conversation => conversation.externalId === 'snapshot-fixture');
  assert.equal(snapshot.workspace, '/fixture/snapshot-project');
  assert.deepEqual(
    snapshot.messages.map(message => message.content),
    ['snapshot question', 'snapshot response']
  );
  console.log('PASS Cursor adapter parses legacy, Composer, and snapshot sessions with Bun SQLite');
} finally {
  await rm(root, { recursive: true, force: true });
}
