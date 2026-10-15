// Adapter drift check against current public Hugging Face agent traces (trx-bx02).
//
// Samples recently updated public datasets tagged `format:agent-traces`,
// classifies each sampled file by its native shape, places it at the
// matching adapter's canonical location inside an isolated HOME and parses
// it through the real adapter protocol. Fetched traces are never written
// outside a temporary directory and never committed.
//
// Failures (exit 1): a classified trace that the adapter rejects or parses
// into zero messages. Informational: native event/content types not listed
// in scripts/trace-drift-known.json (new upstream schema to review).
//
// Usage:
//   bun scripts/trace-drift.ts [--datasets 40] [--files 2] [--report drift.json]
//   bun scripts/trace-drift.ts --local trace.jsonl   # offline check of one file

import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

type Harness = 'claude-code' | 'codex' | 'pi' | 'hermes' | 'droid';
type Line = Record<string, any>;

const HUB = 'https://huggingface.co';
const MAX_BYTES = 2 * 1024 * 1024;
const repo = new URL('../', import.meta.url).pathname;
const flag = (name: string, fallback: string) => {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : fallback;
};
const datasetLimit = Number(flag('datasets', '40'));
const filesPerDataset = Number(flag('files', '2'));
const reportPath = flag('report', '');
const localFile = flag('local', '');

async function json(url: string): Promise<any> {
  const response = await fetch(url, { headers: process.env.HF_TOKEN ? { authorization: `Bearer ${process.env.HF_TOKEN}` } : {} });
  if (!response.ok) throw new Error(`${response.status} ${url}`);
  return response.json();
}

/** Bounded download: whole small files, otherwise a prefix cut at the last complete line. */
async function fetchPrefix(dataset: string, revision: string, path: string, size: number): Promise<string> {
  const response = await fetch(`${HUB}/datasets/${dataset}/resolve/${revision}/${path.split('/').map(encodeURIComponent).join('/')}`, {
    headers: size > MAX_BYTES ? { range: `bytes=0-${MAX_BYTES - 1}` } : {},
  });
  if (!response.ok) throw new Error(`${response.status} fetching ${path}`);
  const text = await response.text();
  return size > MAX_BYTES ? text.slice(0, text.lastIndexOf('\n') + 1) : text;
}

function lines(text: string): Line[] {
  const out: Line[] = [];
  for (const raw of text.split('\n')) {
    if (!raw.trim()) continue;
    try {
      const value = JSON.parse(raw);
      if (value && typeof value === 'object' && !Array.isArray(value)) out.push(value);
    } catch {
      // Ignore malformed lines; a fully malformed file classifies as unknown.
    }
  }
  return out;
}

function classify(entries: Line[]): Harness | null {
  if (!entries.length) return null;
  if (entries[0].type === 'session' && typeof entries[0].version === 'number' && typeof entries[0].id === 'string') return 'pi';
  if (entries.some(e => ['session_meta', 'response_item', 'turn_context', 'event_msg'].includes(e.type) && e.payload)) return 'codex';
  if (entries.some(e => typeof e.sessionId === 'string' && typeof e.uuid === 'string')) return 'claude-code';
  if (entries.some(e => typeof e.system_prompt === 'string' || (Array.isArray(e.messages) && typeof e.id === 'string' && 'source' in e))) return 'hermes';
  if (entries.some(e => e.type === 'session_start' || (typeof e.sessionId === 'string' && e.message && !e.uuid))) return 'droid';
  return null;
}

/** Whether the native trace contains conversational messages at all. */
function hasMessages(harness: Harness, entries: Line[]): boolean {
  switch (harness) {
    case 'claude-code': return entries.some(e => (e.type === 'user' || e.type === 'assistant') && e.message);
    case 'codex': return entries.some(e => e.type === 'response_item' && e.payload?.type === 'message');
    case 'pi': return entries.some(e => e.type === 'message');
    case 'hermes': return entries.some(e => Array.isArray(e.messages) && e.messages.length > 0);
    case 'droid': return entries.some(e => e.message);
  }
}

/** Native type vocabulary used for drift reporting. */
function nativeTypes(harness: Harness, entries: Line[]): string[] {
  const types = new Set<string>();
  for (const entry of entries) {
    if (typeof entry.type === 'string') types.add(entry.type);
    if (harness === 'codex' && typeof entry.payload?.type === 'string') types.add(`${entry.type}.${entry.payload.type}`);
    const content = entry.message?.content ?? entry.payload?.content;
    if (Array.isArray(content)) for (const block of content) if (typeof block?.type === 'string') types.add(`content.${block.type}`);
  }
  return [...types].sort();
}

async function place(harness: Harness, home: string, name: string, text: string, entries: Line[]): Promise<string> {
  const write = async (dir: string, file: string, content: string) => {
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, file), content);
  };
  switch (harness) {
    case 'claude-code':
      await write(join(home, '.claude', 'projects', '-drift'), `${name}.jsonl`, text);
      return join(home, '.claude', 'projects');
    case 'codex':
      await write(join(home, '.codex', 'sessions', '2026', '01', '01'), `rollout-${name}.jsonl`, text);
      return join(home, '.codex', 'sessions');
    case 'pi':
      await write(join(home, '.pi', 'agent', 'sessions', '--drift--'), `${name}.jsonl`, text);
      return join(home, '.pi', 'agent', 'sessions');
    case 'droid':
      await write(join(home, '.factory', 'sessions', '-drift'), `${name}.jsonl`, text);
      return join(home, '.factory', 'sessions');
    case 'hermes': {
      // `hermes sessions export` emits one session per line; the adapter reads
      // the on-disk session_<id>.json layout.
      const dir = join(home, '.hermes', 'sessions');
      for (const [index, entry] of entries.entries()) await write(dir, `session_${name}-${index}.json`, JSON.stringify({ session_id: entry.session_id ?? entry.id, ...entry }));
      return dir;
    }
  }
}

function parse(harness: Harness, home: string, path: string): { ok: boolean; conversations: number; messages: number; error?: string } {
  const result = Bun.spawnSync(['bun', 'run', join(repo, 'adapters', harness, 'adapter.ts')], {
    cwd: home,
    env: {
      PATH: process.env.PATH ?? '',
      HOME: home,
      XDG_CONFIG_HOME: join(home, '.config'),
      XDG_DATA_HOME: join(home, '.local', 'share'),
      XDG_STATE_HOME: join(home, '.local', 'state'),
      TZ: 'UTC',
      HSTRY_REQUEST: JSON.stringify({ method: 'parse', params: { path, opts: {} } }),
    },
  });
  if (!result.success) return { ok: false, conversations: 0, messages: 0, error: result.stderr.toString().trim().slice(0, 500) };
  const conversations = JSON.parse(result.stdout.toString()) as { messages?: unknown[] }[];
  const messages = conversations.reduce((sum, conversation) => sum + (conversation.messages?.length ?? 0), 0);
  return { ok: conversations.length > 0 && messages > 0, conversations: conversations.length, messages, error: messages ? undefined : 'no messages parsed' };
}

const known: Record<string, string[]> = JSON.parse(await readFile(join(repo, 'scripts', 'trace-drift-known.json'), 'utf8').catch(() => '{}'));
const datasets = localFile ? [] : await json(`${HUB}/api/datasets?filter=format:agent-traces&sort=lastModified&direction=-1&limit=${datasetLimit}`);
const samples: any[] = [];

async function examine(sample: any, path: string, text: string): Promise<void> {
  const entries = path.endsWith('.json') ? lines(text.replace(/\n/g, ' ')) : lines(text);
  const harness = classify(entries);
  sample.harness = harness;
  if (!harness) return;
  // Traces without any conversational records (e.g. metadata-only
  // uploads) are not adapter failures.
  sample.messageBearing = hasMessages(harness, entries);
  sample.unseenTypes = nativeTypes(harness, entries).filter(type => !(known[harness] ?? []).includes(type));
  sample.nativeVersion = entries.find(e => e.version && typeof e.version === 'string')?.version ?? entries.find(e => e.payload?.cli_version)?.payload.cli_version;
  const home = await mkdtemp(join(tmpdir(), 'hstry-drift-'));
  try {
    Object.assign(sample, parse(harness, home, await place(harness, home, 'sample', text, entries)));
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}

if (localFile) {
  const sample: any = { dataset: 'local', file: localFile };
  samples.push(sample);
  await examine(sample, localFile, await readFile(localFile, 'utf8'));
}

for (const { id } of datasets) {
  try {
    const info = await json(`${HUB}/api/datasets/${id}`);
    if (info.gated || info.private) continue;
    const tree: { path: string; size: number; type: string }[] = await json(`${HUB}/api/datasets/${id}/tree/${info.sha}?recursive=true`);
    const files = tree.filter(file => file.type === 'file' && /\.(jsonl|json)$/.test(file.path)).slice(0, filesPerDataset);
    for (const file of files) {
      const sample: any = { dataset: id, revision: info.sha, file: file.path, bytes: file.size, truncated: file.size > MAX_BYTES };
      samples.push(sample);
      try {
        await examine(sample, file.path, await fetchPrefix(id, info.sha, file.path, file.size));
      } catch (error) {
        sample.ok = false;
        sample.error = (error as Error).message;
      }
    }
  } catch (error) {
    samples.push({ dataset: id, ok: false, error: (error as Error).message });
  }
}

const classified = samples.filter(sample => sample.harness);
const failures = classified.filter(sample => !sample.ok && (sample.messageBearing || sample.error !== 'no messages parsed'));
const unseen: Record<string, string[]> = {};
for (const sample of classified) for (const type of sample.unseenTypes ?? []) (unseen[sample.harness] ??= []).includes(type) || unseen[sample.harness].push(type);
const report = {
  generatedAt: new Date().toISOString(),
  datasetsSampled: datasets.length,
  files: samples.length,
  classified: classified.length,
  failures: failures.length,
  byHarness: Object.fromEntries(['claude-code', 'codex', 'pi', 'hermes', 'droid'].map(h => [h, {
    samples: classified.filter(s => s.harness === h).length,
    failures: failures.filter(s => s.harness === h).length,
    versions: [...new Set(classified.filter(s => s.harness === h && s.nativeVersion).map(s => s.nativeVersion))].sort(),
  }])),
  unseenTypes: unseen,
  samples,
};
if (reportPath) await writeFile(reportPath, JSON.stringify(report, null, 2));
for (const [harness, summary] of Object.entries(report.byHarness)) console.log(`${harness.padEnd(12)} samples=${summary.samples} failures=${summary.failures} versions=${summary.versions.join(',') || '-'}`);
for (const [harness, types] of Object.entries(unseen)) console.log(`unseen ${harness}: ${types.sort().join(', ')}`);
for (const failure of failures) console.log(`FAIL ${failure.harness} ${failure.dataset}@${failure.revision?.slice(0, 8)} ${failure.file}: ${failure.error}`);
console.log(`unclassified files: ${samples.length - classified.length}`);
if (failures.length) process.exit(1);
