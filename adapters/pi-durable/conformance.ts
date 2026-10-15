// Format checked against the pi-durable SDK 1.1.0 golden fixtures in ./fixtures
// (manifest.json: synthetic-sdk-created-detached, pi/durable/ai/chord 1.1.0).
import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const fixtures = new URL('./fixtures/', import.meta.url).pathname;

function snapshotCase(name: string, file: string): ConformanceCase {
  return {
    name,
    async setup(home) {
      const root = join(home, '.local', 'share', 'hstry', 'pi-durable-snapshots');
      await mkdir(root, { recursive: true });
      await copyFile(join(fixtures, file), join(root, file));
      return root;
    },
  };
}

export const cases: ConformanceCase[] = [
  snapshotCase('full-tree', 'full.pi-durable.json'),
  snapshotCase('selected-branch', 'selected.pi-durable.json'),
];
