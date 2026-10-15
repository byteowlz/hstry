// Format checked against can1357/oh-my-pi @ c2cb79e (2026-10-14), docs/session.md and
// packages/coding-agent/src/session/{session-entries,session-title-slot}.ts: session version 3,
// ~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<sessionId>.jsonl, file physically starts with a
// fixed-width 256-byte `type:"title"` slot line, then the session header (id, title, titleSource,
// cwd) and id/parentId entries; model_change carries `model: "provider/modelId"`; titles change via
// title_change entries; camelCase message roles (user, assistant, toolResult, ...).
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const T = Date.parse('2026-10-02T14:00:00.000Z');
const iso = (offsetMs: number) => new Date(T + offsetMs).toISOString();
const usage = (input: number, output: number, cacheRead: number, total: number) => ({
  input, output, cacheRead, cacheWrite: 0, totalTokens: input + output + cacheRead,
  cost: { input: total / 2, output: total / 2, cacheRead: 0, cacheWrite: 0, total },
});

/** The title slot is exactly 256 bytes including its newline. */
function titleSlot(title: string, source: string, updatedAt: string): string {
  const base = { type: 'title', v: 1, title, source, updatedAt, pad: '' };
  const padding = 256 - Buffer.byteLength(JSON.stringify(base) + '\n');
  return JSON.stringify({ ...base, pad: ' '.repeat(padding) }) + '\n';
}

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';
const sessionsRoot = (home: string) => join(home, '.omp', 'agent', 'sessions');

export const cases: ConformanceCase[] = [
  {
    name: 'title-slot-session',
    async setup(home) {
      const dir = join(sessionsRoot(home), '--work-demo--');
      await mkdir(dir, { recursive: true });
      const sid = '019c625b-b900-7000-8000-000000000001';
      const slot = titleSlot('Fix flaky retry test', 'auto', iso(5000));
      if (Buffer.byteLength(slot) !== 256) throw new Error('title slot must be 256 bytes');
      const assistant = (offset: number, content: unknown[], stopReason: string, u: ReturnType<typeof usage>) => ({
        role: 'assistant', content, api: 'anthropic-messages', provider: 'anthropic', model: 'claude-sonnet-4-5', usage: u, stopReason, timestamp: T + offset,
      });
      await writeFile(join(dir, `${iso(0).replace(/[:.]/g, '-')}_${sid}.jsonl`), slot + lines([
        { type: 'session', version: 3, id: sid, title: 'Fix flaky retry test', titleSource: 'auto', timestamp: iso(0), cwd: '/work/demo' },
        { type: 'model_change', id: 'b1c2d3e4', parentId: null, timestamp: iso(100), model: 'anthropic/claude-sonnet-4-5', role: 'default' },
        { type: 'thinking_level_change', id: 'c1d2e3f4', parentId: 'b1c2d3e4', timestamp: iso(200), thinkingLevel: 'high', configured: 'auto' },
        { type: 'message', id: 'a1000001', parentId: 'c1d2e3f4', timestamp: iso(1000), message: { role: 'user', content: [{ type: 'text', text: 'The retry test is flaky, please fix it' }], timestamp: T + 1000 } },
        { type: 'message', id: 'a1000002', parentId: 'a1000001', timestamp: iso(3000), message: assistant(3000, [
          { type: 'thinking', thinking: 'Look at the test first.' },
          { type: 'text', text: 'Reading the test.' },
          { type: 'toolCall', id: 'toolu_01A', name: 'read', arguments: { path: 'src/retry.test.ts' } },
        ], 'toolUse', usage(900, 50, 0, 0.01)) },
        { type: 'custom', id: 'a1000003', parentId: 'a1000002', timestamp: iso(3100), customType: 'tool_execution_start', data: { toolCallId: 'toolu_01A', toolName: 'read', startedAt: iso(3100) } },
        { type: 'message', id: 'a1000004', parentId: 'a1000003', timestamp: iso(4000), message: { role: 'toolResult', toolCallId: 'toolu_01A', toolName: 'read', content: [{ type: 'text', text: 'ENOENT: no such file or directory, open src/retry.test.ts' }], isError: true, timestamp: T + 4000 } },
        { type: 'message', id: 'a1000005', parentId: 'a1000004', timestamp: iso(6000), message: assistant(6000, [
          { type: 'toolCall', id: 'toolu_02B', name: 'bash', arguments: { command: 'ls src' } },
        ], 'toolUse', usage(1100, 30, 900, 0.006)) },
        { type: 'message', id: 'a1000006', parentId: 'a1000005', timestamp: iso(7000), message: { role: 'toolResult', toolCallId: 'toolu_02B', toolName: 'bash', content: [{ type: 'text', text: 'retry.ts\nretry.spec.ts' }], isError: false, timestamp: T + 7000 } },
        { type: 'message', id: 'a1000007', parentId: 'a1000006', timestamp: iso(9000), message: assistant(9000, [
          { type: 'text', text: 'The test file is retry.spec.ts; I seeded the jitter to remove the flakiness.' },
        ], 'stop', usage(1300, 40, 2000, 0.005)) },
        { type: 'model_usage', id: 'a1000008', parentId: 'a1000007', timestamp: iso(9500), purpose: 'title', role: 'tiny', api: 'openai-responses', provider: 'openai', model: 'gpt-5-mini', usage: usage(80, 10, 0, 0.0001), stopReason: 'stop' },
        { type: 'title_change', id: 'a1000009', parentId: 'a1000008', timestamp: iso(10000), title: 'Fix flaky retry test', source: 'auto', trigger: 'auto' },
        { type: 'compaction', id: 'a1000010', parentId: 'a1000009', timestamp: iso(20000), summary: 'Fixed the flaky retry test by seeding jitter.', firstKeptEntryId: 'a1000007', tokensBefore: 9000, method: 'summary' },
        { type: 'message', id: 'a1000011', parentId: 'a1000010', timestamp: iso(30000), message: { role: 'user', content: [{ type: 'text', text: 'Great, run the full suite' }], timestamp: T + 30000 } },
      ]));
      return sessionsRoot(home);
    },
  },
  {
    name: 'legacy-header-first-session',
    async setup(home) {
      const dir = join(sessionsRoot(home), '--work-legacy--');
      await mkdir(dir, { recursive: true });
      const sid = '019b0000-aaaa-7bbb-8ccc-ddddeeeeffff';
      await writeFile(join(dir, `${iso(900000).replace(/[:.]/g, '-')}_${sid}.jsonl`), lines([
        { type: 'session', version: 3, id: sid, timestamp: iso(900000), cwd: '/work/legacy' },
        { type: 'message', id: 'c0000001', parentId: null, timestamp: iso(901000), message: { role: 'user', content: [{ type: 'text', text: 'Explain this repo' }], timestamp: T + 901000 } },
        { type: 'message', id: 'c0000002', parentId: 'c0000001', timestamp: iso(903000), message: {
          role: 'assistant', content: [{ type: 'text', text: 'A small Rust CLI.' }], api: 'openai-responses', provider: 'openai', model: 'gpt-5', usage: usage(200, 10, 0, 0.001), stopReason: 'stop', timestamp: T + 903000,
        } },
      ]));
      return sessionsRoot(home);
    },
  },
];
