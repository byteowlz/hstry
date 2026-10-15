// Regression tests for Codex/Claude Code storage parity with grok-build
// foreign-session discovery (trx-kx41).
// Usage: bun adapters/codex/test.js

import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { zstdCompressSync } from 'node:zlib';
import { Database } from 'bun:sqlite';

const repo = new URL('../../', import.meta.url).pathname;
const root = await mkdtemp(join(tmpdir(), 'hstry-codex-parity-'));

function call(adapter, method, path, env) {
  const result = Bun.spawnSync(['bun', 'run', join(repo, 'adapters', adapter, 'adapter.ts')], {
    env: {
      ...process.env,
      HOME: join(root, 'home'),
      CODEX_HOME: undefined,
      CLAUDE_CONFIG_DIR: undefined,
      ...env,
      HSTRY_REQUEST: JSON.stringify({ method, params: { path, opts: {} } }),
    },
  });
  assert.ok(result.success, `${adapter} ${method}: ${result.stderr}`);
  return JSON.parse(result.stdout.toString());
}

try {
  const fixture = await readFile(join(repo, 'testdata/codex/test-session-rollout.jsonl'));

  // CODEX_HOME relocates the canonical root, and .jsonl.zst rollouts parse
  // identically to their plain twin.
  const codexHome = join(root, 'custom-codex');
  const plainDir = join(codexHome, 'sessions', '2026', '01', '01');
  const zstDir = join(codexHome, 'archived_sessions');
  await mkdir(plainDir, { recursive: true });
  await mkdir(zstDir, { recursive: true });
  await writeFile(join(plainDir, 'rollout-plain.jsonl'), fixture);
  const env = { CODEX_HOME: codexHome };

  assert.equal(call('codex', 'detect', join(codexHome, 'sessions'), env), 0.9);
  assert.equal(call('codex', 'detect', join(codexHome, 'sessions'), {}), null, 'non-canonical without CODEX_HOME');
  const plain = call('codex', 'parse', join(codexHome, 'sessions'), env);
  assert.equal(plain.length, 1);

  await writeFile(join(zstDir, 'rollout-plain.jsonl.zst'), zstdCompressSync(fixture));
  assert.equal(call('codex', 'detect', zstDir, env), 0.9, 'compressed-only directory is detected');
  const compressed = call('codex', 'parse', zstDir, env);
  assert.deepEqual(compressed.map(({ metadata, ...rest }) => rest), plain.map(({ metadata, ...rest }) => rest));

  // Plain and compressed copies of one rollout import once.
  await writeFile(join(plainDir, 'rollout-plain.jsonl.zst'), zstdCompressSync(fixture));
  assert.equal(call('codex', 'parse', join(codexHome, 'sessions'), env).length, 1);

  // The newest state_<N>.sqlite generation supplies thread titles.
  const id = plain[0].externalId;
  for (const [generation, title] of [[5, 'stale generation title'], [7, 'current generation title']]) {
    const db = new Database(join(codexHome, `state_${generation}.sqlite`), { create: true });
    db.run('CREATE TABLE threads (id TEXT, title TEXT, model TEXT, archived INTEGER)');
    db.query('INSERT INTO threads VALUES (?, ?, ?, 0)').run(id, title, null);
    db.close();
  }
  assert.equal(call('codex', 'parse', join(codexHome, 'sessions'), env)[0].title, 'current generation title');

  // CLAUDE_CONFIG_DIR relocates the claude-code canonical root.
  const claudeDir = join(root, 'custom-claude');
  const project = join(claudeDir, 'projects', '-work');
  await mkdir(project, { recursive: true });
  await copyFile(join(repo, 'testdata/claude-code/test-session.jsonl'), join(project, 'session.jsonl'));
  const claudeEnv = { CLAUDE_CONFIG_DIR: claudeDir };
  assert.equal(call('claude-code', 'detect', join(claudeDir, 'projects'), claudeEnv), 0.9);
  assert.equal(call('claude-code', 'detect', join(claudeDir, 'projects'), {}), null);
  assert.ok(call('claude-code', 'parse', join(claudeDir, 'projects'), claudeEnv)[0].messages.length > 0);

  console.log('codex/claude-code storage parity tests passed');
} finally {
  await rm(root, { recursive: true, force: true });
}
