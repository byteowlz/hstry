import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const fixture = new URL('../../testdata/codex/test-session-rollout.jsonl', import.meta.url).pathname;

export const cases: ConformanceCase[] = [{
  name: 'date-nested-rollout',
  async setup(home) {
    const day = join(home, '.codex', 'sessions', '2026', '01', '01');
    await mkdir(day, { recursive: true });
    await copyFile(fixture, join(day, 'rollout-2026-01-01T00-00-00-fixture.jsonl'));
    return join(home, '.codex', 'sessions');
  },
}];
