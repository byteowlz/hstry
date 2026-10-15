import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const fixture = new URL('../../testdata/claude-code/test-session.jsonl', import.meta.url).pathname;

export const cases: ConformanceCase[] = [{
  name: 'project-session',
  async setup(home) {
    const project = join(home, '.claude', 'projects', '-work');
    await mkdir(project, { recursive: true });
    await copyFile(fixture, join(project, 'session.jsonl'));
    return join(home, '.claude', 'projects');
  },
}];
