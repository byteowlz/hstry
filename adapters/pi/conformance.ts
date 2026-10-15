// Format checked against earendil-works/pi (pi-mono mirror badlogic/pi-mono @ 4a5d372, coding-agent
// 1.2.0), packages/coding-agent/docs/session-format.md: session file version 3, header + id/parentId
// tree entries (message, model_change, thinking_level_change, compaction, session_info, usage, custom,
// label), nested message timestamps in unix ms, assistant usage/stopReason/thinkingLevel, toolResult
// {toolCallId, toolName, isError}, leading system message with prompt `sections`.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const lines = (items: unknown[]) => items.map(item => JSON.stringify(item)).join('\n') + '\n';
const T = Date.parse('2026-10-01T09:00:00.000Z');
const iso = (offsetMs: number) => new Date(T + offsetMs).toISOString();
const usage = (input: number, output: number, cacheRead: number, total: number) => ({
  input, output, cacheRead, cacheWrite: 0, totalTokens: input + output + cacheRead,
  cost: { input: total / 2, output: total / 2, cacheRead: 0, cacheWrite: 0, total },
});

const sessionsRoot = (home: string) => join(home, '.pi', 'agent', 'sessions');

export const cases: ConformanceCase[] = [
  {
    name: 'tree-session-tools-compaction',
    async setup(home) {
      const dir = join(sessionsRoot(home), '--work-demo--');
      await mkdir(dir, { recursive: true });
      const sid = '0199f0a1-1111-7222-8333-444455556666';
      const assistant = (offset: number, content: unknown[], stopReason: string, u: ReturnType<typeof usage>) => ({
        role: 'assistant', content, api: 'anthropic-messages', provider: 'anthropic', model: 'claude-sonnet-5-5', usage: u, stopReason, thinkingLevel: 'medium', timestamp: T + offset,
      });
      await writeFile(join(dir, `${iso(0).replace(/[:.]/g, '-')}_${sid}.jsonl`), lines([
        { type: 'session', version: 3, id: sid, timestamp: iso(0), cwd: '/work/demo' },
        { type: 'message', id: 'a0000001', parentId: null, timestamp: iso(0), message: { role: 'system', content: '', sections: { preamble: 'You are an expert coding assistant.', cwd: '/work/demo' }, toolsAdded: [{ name: 'bash', description: 'Run shell', parameters: {} }], timestamp: T } },
        { type: 'model_change', id: 'a0000002', parentId: 'a0000001', timestamp: iso(100), provider: 'anthropic', modelId: 'claude-sonnet-5-5' },
        { type: 'thinking_level_change', id: 'a0000003', parentId: 'a0000002', timestamp: iso(200), thinkingLevel: 'medium' },
        { type: 'message', id: 'a0000004', parentId: 'a0000003', timestamp: iso(1000), message: { role: 'user', content: 'Run the tests and fix failures', timestamp: T + 1000 } },
        { type: 'message', id: 'a0000005', parentId: 'a0000004', timestamp: iso(3000), message: assistant(3000, [
          { type: 'thinking', thinking: 'Run the test suite first.', thinkingSignature: 'sig-1' },
          { type: 'text', text: 'Running the tests.' },
          { type: 'toolCall', id: 'call_001|fc_001', name: 'bash', arguments: { command: 'npm test' } },
        ], 'toolUse', usage(1200, 80, 0, 0.012)) },
        { type: 'message', id: 'a0000006', parentId: 'a0000005', timestamp: iso(5000), message: { role: 'toolResult', toolCallId: 'call_001|fc_001', toolName: 'bash', content: [{ type: 'text', text: 'FAIL src/sum.test.ts\nExpected 3, received -1' }], isError: true, timestamp: T + 5000 } },
        { type: 'message', id: 'a0000007', parentId: 'a0000006', timestamp: iso(8000), message: assistant(8000, [
          { type: 'toolCall', id: 'call_002|fc_002', name: 'bash', arguments: { command: "sed -i 's/a - b/a + b/' src/sum.ts && npm test" } },
        ], 'toolUse', usage(1500, 60, 1200, 0.009)) },
        { type: 'message', id: 'a0000008', parentId: 'a0000007', timestamp: iso(11000), message: { role: 'toolResult', toolCallId: 'call_002|fc_002', toolName: 'bash', content: [{ type: 'text', text: 'PASS src/sum.test.ts' }], isError: false, timestamp: T + 11000 } },
        { type: 'message', id: 'a0000009', parentId: 'a0000008', timestamp: iso(13000), message: assistant(13000, [
          { type: 'text', text: 'Fixed: `sum` subtracted instead of adding. Tests pass.' },
        ], 'stop', usage(1700, 40, 2700, 0.004)) },
        { type: 'usage', id: 'a0000010', parentId: 'a0000009', timestamp: iso(14000), kind: 'cache_warm', provider: 'anthropic', model: 'claude-sonnet-5-5', usage: usage(0, 0, 5000, 0.0015) },
        { type: 'compaction', id: 'a0000011', parentId: 'a0000010', timestamp: iso(20000), summary: 'The user asked to fix failing tests; sum() was corrected.', firstKeptEntryId: 'a0000009', tokensBefore: 7000, details: { readFiles: [], modifiedFiles: ['src/sum.ts'] } },
        { type: 'message', id: 'a0000012', parentId: 'a0000011', timestamp: iso(30000), message: { role: 'user', content: [{ type: 'text', text: 'Thanks, commit it' }], timestamp: T + 30000 } },
        { type: 'session_info', id: 'a0000013', parentId: 'a0000012', timestamp: iso(31000), name: 'Fix sum tests' },
      ]));
      return sessionsRoot(home);
    },
  },
  {
    name: 'second-workspace-session',
    async setup(home) {
      const dir = join(sessionsRoot(home), '--work-other--');
      await mkdir(dir, { recursive: true });
      const sid = '0199f0a2-aaaa-7bbb-8ccc-ddddeeeeffff';
      await writeFile(join(dir, `${iso(600000).replace(/[:.]/g, '-')}_${sid}.jsonl`), lines([
        { type: 'session', version: 3, id: sid, timestamp: iso(600000), cwd: '/work/other' },
        { type: 'message', id: 'b0000001', parentId: null, timestamp: iso(601000), message: { role: 'user', content: 'What does this repo do?', timestamp: T + 601000 } },
        { type: 'message', id: 'b0000002', parentId: 'b0000001', timestamp: iso(604000), message: {
          role: 'assistant', content: [{ type: 'text', text: 'It is a small CLI for syncing notes.' }], api: 'openai-responses', provider: 'openai', model: 'gpt-5', usage: usage(300, 20, 0, 0.002), stopReason: 'stop', timestamp: T + 604000,
        } },
      ]));
      return sessionsRoot(home);
    },
  },
];
