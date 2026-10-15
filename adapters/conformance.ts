// Adapter conformance runner (trx-w4sp).
//
// Every adapters/<name>/adapter.ts must ship adapters/<name>/conformance.ts
// exporting `cases`. Each case builds a fixture inside an isolated HOME and
// the runner drives the adapter through the real JSON protocol:
//   - detect() claims the fixture path, and refuses a sibling outside its
//     canonical roots (unless the case is content-detected, e.g. exports)
//   - parse() returns conversations with integer timestamps and known roles
//   - the parsed output equals the committed expected snapshot (full value)
//
// Usage:
//   bun adapters/conformance.ts             # run all adapters
//   bun adapters/conformance.ts codex pi    # run selected adapters
//   bun adapters/conformance.ts --update    # rewrite expected snapshots

import { mkdtemp, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export interface ConformanceCase {
  /** Stable case name; used for the expected snapshot file name. */
  name: string;
  /** Create the fixture under `home` and return the path to detect/parse. */
  setup(home: string): Promise<string>;
  /**
   * `canonical` (default): detect must claim the path and refuse a path
   * outside the adapter's canonical roots. `content`: detect must claim the
   * path, location-independent (export files). `none`: import-only, detect
   * is not exercised.
   */
  detection?: 'canonical' | 'content' | 'none';
  /** Optional extra assertions on parsed conversations. */
  check?(conversations: unknown[]): void;
}

const adaptersDir = new URL('.', import.meta.url).pathname;
const ROLES = new Set(['user', 'assistant', 'system', 'tool']);

function adapterEnv(home: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) {
    // Never let the developer's real tool locations leak into fixtures.
    if (value === undefined || /^(XDG_|CODEX_HOME$|CLAUDE_CONFIG_DIR$|APPDATA$|LOCALAPPDATA$)/.test(key)) continue;
    env[key] = value;
  }
  return {
    ...env,
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: join(home, '.config'),
    XDG_DATA_HOME: join(home, '.local', 'share'),
    XDG_STATE_HOME: join(home, '.local', 'state'),
    XDG_CACHE_HOME: join(home, '.cache'),
    TZ: 'UTC',
  };
}

function call(adapter: string, home: string, method: string, path: string): unknown {
  const result = Bun.spawnSync(['bun', 'run', join(adaptersDir, adapter, 'adapter.ts')], {
    cwd: home,
    env: { ...adapterEnv(home), HSTRY_REQUEST: JSON.stringify({ method, params: { path, opts: {} } }) },
  });
  if (!result.success) throw new Error(`${method} exited ${result.exitCode}: ${result.stderr.toString().trim()}`);
  return JSON.parse(result.stdout.toString());
}

/** Replace the temporary HOME so snapshots are machine-independent. */
function normalize(value: unknown, home: string): unknown {
  return JSON.parse(JSON.stringify(value).split(home).join('$HOME'));
}

function validate(conversations: unknown): asserts conversations is Record<string, any>[] {
  if (!Array.isArray(conversations) || conversations.length === 0) throw new Error('parse returned no conversations');
  for (const conversation of conversations) {
    if (!Number.isInteger(conversation.createdAt)) throw new Error(`createdAt not an integer: ${conversation.createdAt}`);
    if (conversation.updatedAt !== undefined && !Number.isInteger(conversation.updatedAt)) throw new Error('updatedAt not an integer');
    if (!Array.isArray(conversation.messages) || conversation.messages.length === 0) throw new Error(`conversation ${conversation.externalId} has no messages`);
    for (const message of conversation.messages) {
      if (!ROLES.has(message.role)) throw new Error(`unknown role ${message.role}`);
      if (typeof message.content !== 'string') throw new Error('message content is not a string');
      if (message.createdAt !== undefined && message.createdAt !== null && !Number.isInteger(message.createdAt)) throw new Error('message createdAt not an integer');
    }
  }
}

async function runAdapter(adapter: string, update: boolean): Promise<string[]> {
  const module = join(adaptersDir, adapter, 'conformance.ts');
  if (!existsSync(module)) return [`${adapter}: missing conformance.ts (every adapter needs fixtures)`];
  const { cases } = (await import(module)) as { cases: ConformanceCase[] };
  if (!cases?.length) return [`${adapter}: conformance.ts exports no cases`];
  const failures: string[] = [];
  for (const testCase of cases) {
    const home = await mkdtemp(join(tmpdir(), `hstry-conformance-${adapter}-`));
    const label = `${adapter}/${testCase.name}`;
    try {
      const path = await testCase.setup(home);
      const detection = testCase.detection ?? 'canonical';
      if (detection !== 'none') {
        const confidence = call(adapter, home, 'detect', path);
        if (typeof confidence !== 'number' || confidence <= 0) throw new Error(`detect did not claim fixture (got ${JSON.stringify(confidence)})`);
      }
      if (detection === 'canonical') {
        const outside = join(home, 'not-a-canonical-root', adapter);
        await mkdir(outside, { recursive: true });
        const claimed = call(adapter, home, 'detect', outside);
        if (claimed !== null) throw new Error(`detect claimed a non-canonical path (got ${JSON.stringify(claimed)})`);
      }
      const conversations = normalize(call(adapter, home, 'parse', path), home);
      validate(conversations);
      testCase.check?.(conversations);
      const expectedPath = join(adaptersDir, adapter, 'conformance', `${testCase.name}.expected.json`);
      const actual = JSON.stringify(conversations, null, 2) + '\n';
      if (update) {
        await mkdir(join(adaptersDir, adapter, 'conformance'), { recursive: true });
        await writeFile(expectedPath, actual);
      } else if (!existsSync(expectedPath)) {
        throw new Error(`missing expected snapshot ${expectedPath}; review output and run with --update`);
      } else if ((await readFile(expectedPath, 'utf8')) !== actual) {
        throw new Error(`parse output differs from ${expectedPath}; inspect, then --update if intended`);
      }
      console.log(`ok   ${label}`);
    } catch (error) {
      failures.push(`${label}: ${(error as Error).message}`);
      console.log(`FAIL ${label}`);
    } finally {
      await rm(home, { recursive: true, force: true });
    }
  }
  return failures;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const update = args.includes('--update');
  const selected = args.filter(arg => !arg.startsWith('--'));
  const all = (await readdir(adaptersDir, { withFileTypes: true }))
    .filter(entry => entry.isDirectory() && existsSync(join(adaptersDir, entry.name, 'adapter.ts')))
    .map(entry => entry.name)
    .sort();
  const unknown = selected.filter(name => !all.includes(name));
  if (unknown.length) throw new Error(`unknown adapters: ${unknown.join(', ')}`);
  const failures: string[] = [];
  for (const adapter of selected.length ? selected : all) failures.push(...(await runAdapter(adapter, update)));
  if (failures.length) {
    console.error(`\n${failures.length} conformance failure(s):\n  ${failures.join('\n  ')}`);
    process.exit(1);
  }
  console.log(`\nall ${selected.length || all.length} adapters conform`);
}
