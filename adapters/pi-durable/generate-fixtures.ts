import {mkdir, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {goldenPiSource, FIXTURE_TIME} from './golden-source.ts';
import {snapshotFromPi} from './interop.ts';

const directory = fileURLToPath(new URL('./fixtures/', import.meta.url));
await mkdir(directory, {recursive: true});
const {artifact} = goldenPiSource();
const entries = JSON.parse(JSON.stringify([artifact.header, ...artifact.entries]));
const full = await snapshotFromPi(entries, artifact.header.cwd, 'golden-pi-store', 'full-tree');
full.capturedAt = FIXTURE_TIME;
const selected = await snapshotFromPi(entries, artifact.header.cwd, 'golden-pi-store', 'selected-branch');
selected.capturedAt = FIXTURE_TIME;
const files = {'ordinary-pi.jsonl': entries.map((entry: unknown) => JSON.stringify(entry)).join('\n') + '\n', 'full.pi-durable.json': JSON.stringify(full, null, 2) + '\n', 'selected.pi-durable.json': JSON.stringify(selected, null, 2) + '\n'};
const hashes: Record<string, string> = {};
for (const [name, contents] of Object.entries(files)) {
  await writeFile(join(directory, name), contents);
  hashes[name] = createHash('sha256').update(contents).digest('hex');
}
await writeFile(join(directory, 'manifest.json'), JSON.stringify({protocol: 1, sdkVersions: {pi: '1.1.0', durable: '1.1.0', ai: '1.1.0', chord: '1.1.0'}, swiftWrapper: {repo: 'https://github.com/finnvoor/PiDurableKit', revision: 'e9a3bca038017053a57fe88d09ee47f711c4174d', qualification: 'reference-pin-only; Apple/Swift runtime not qualified'}, sourceKind: 'synthetic-sdk-created-detached', fixtures: hashes}, null, 2) + '\n');
console.log('Generated deterministic SDK-created Pi and Durable golden fixtures.');
