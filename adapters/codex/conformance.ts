import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const fixture = new URL('../../testdata/codex/test-session-rollout.jsonl', import.meta.url).pathname;
const reasoningFixture = new URL('../../testdata/codex/reasoning-and-events-rollout.jsonl', import.meta.url).pathname;

export const cases: ConformanceCase[] = [
  {
    name: 'date-nested-rollout',
    async setup(home) {
      const day = join(home, '.codex', 'sessions', '2026', '01', '01');
      await mkdir(day, { recursive: true });
      await copyFile(fixture, join(day, 'rollout-2026-01-01T00-00-00-fixture.jsonl'));
      return join(home, '.codex', 'sessions');
    },
  },
  {
    name: 'reasoning-and-events',
    async setup(home) {
      const day = join(home, '.codex', 'sessions', '2026', '09', '22');
      await mkdir(day, { recursive: true });
      await copyFile(reasoningFixture, join(day, 'rollout-2026-09-22T19-17-31-fixture.jsonl'));
      return join(home, '.codex', 'sessions');
    },
    check(conversations) {
      const [conversation] = conversations as { messages: { role: string; content: string; parts?: Record<string, any>[]; metadata?: Record<string, any> }[] }[];
      const thinking = conversation.messages.flatMap(m => m.parts ?? []).filter(p => p.type === 'thinking');
      // reasoning summaries must be retained AND searchable (FTS indexes content).
      if (!thinking.some(p => p.text.includes('git status'))) throw new Error('reasoning summary part missing');
      if (!conversation.messages.some(m => m.content.includes('git status'))) throw new Error('reasoning summary not in searchable content');
      // agent_message that a response_item.message mirrored must not be duplicated.
      const mirrored = conversation.messages.filter(m => m.content === 'I will inspect the working tree first.');
      if (mirrored.length !== 1) throw new Error(`expected 1 copy of mirrored agent_message, got ${mirrored.length}`);
      // agent_message with no response_item.message must be kept.
      if (!conversation.messages.some(m => m.metadata?.source === 'event_msg.agent_message' && m.content.includes('database layer'))) {
        throw new Error('unmirrored agent_message was dropped');
      }
    },
  },
];
