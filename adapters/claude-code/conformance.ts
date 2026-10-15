import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const fixture = new URL('../../testdata/claude-code/test-session.jsonl', import.meta.url).pathname;
const recordsFixture = new URL('../../testdata/claude-code/records-session.jsonl', import.meta.url).pathname;

/** Assert one custom part survives with the given shape (order-insensitive lookup). */
function hasPart(conversations: unknown[], predicate: (part: Record<string, any>) => boolean): void {
  const parts = (conversations as { messages: { parts?: Record<string, any>[] }[] }[])
    .flatMap(conversation => conversation.messages)
    .flatMap(message => message.parts ?? []);
  if (!parts.some(predicate)) throw new Error('expected part not found in parsed output');
}

export const cases: ConformanceCase[] = [
  {
    name: 'project-session',
    async setup(home) {
      const project = join(home, '.claude', 'projects', '-work');
      await mkdir(project, { recursive: true });
      await copyFile(fixture, join(project, 'session.jsonl'));
      return join(home, '.claude', 'projects');
    },
  },
  {
    name: 'records-and-titles',
    async setup(home) {
      const project = join(home, '.claude', 'projects', '-work');
      await mkdir(project, { recursive: true });
      await copyFile(recordsFixture, join(project, 'session.jsonl'));
      return join(home, '.claude', 'projects');
    },
    check(conversations) {
      const [conversation] = conversations as { title?: string; messages: { role: string; content: string }[] }[];
      // /rename (custom-title) must outrank ai-title.
      if (conversation.title !== 'renamed: release audit') {
        throw new Error(`expected custom-title to win the title, got ${JSON.stringify(conversation.title)}`);
      }
      // redacted_thinking: opaque, retained, never searchable plaintext.
      hasPart(conversations, p => p.type === 'thinking' && p.visibility === 'hidden' && p.meta?.redacted === true && p.text === '');
      // fallback: recorded opaquely with both models.
      hasPart(conversations, p => p.type === 'x-claude-fallback' && p.payload?.from === 'claude-fable-5' && p.payload?.to === 'claude-opus-4-8');
      // attachment: imported as a system message, not as a user turn.
      if (!conversation.messages.some(m => m.role === 'system' && m.content.includes('deep-research'))) {
        throw new Error('expected skill_listing attachment as a searchable system message');
      }
    },
  },
];
