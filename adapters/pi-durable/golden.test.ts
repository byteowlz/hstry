import {test, expect} from 'bun:test';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {SessionManager} from '@earendil-works/pi-coding-agent';
import {Harness, MemoryStorage, createRegistry, defineTool, defineExtension} from '@earendil-works/pi-durable';
import {Type} from '@earendil-works/pi-ai';
import {createModels} from '@earendil-works/pi-ai/models';
import {fauxProvider, fauxAssistantMessage} from '@earendil-works/pi-ai/providers/faux';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import {captureSnapshot, restoreSnapshot} from './interop.ts';
import {validateSnapshot} from './snapshot.ts';
import {exportPiSourceTree} from './tree.ts';

const fixture = (name: string) => fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url));

test('golden full-tree corpus roundtrips through both SDKs with explicit signature gates and zero historical execution', async () => {
  const manifest = JSON.parse(await readFile(fixture('manifest.json'), 'utf8'));
  expect(manifest.sdkVersions).toEqual({pi: '1.1.0', durable: '1.1.0', ai: '1.1.0', chord: '1.1.0'});
  for (const [name, digest] of Object.entries(manifest.fixtures)) expect(createHash('sha256').update(await readFile(fixture(name))).digest('hex')).toBe(String(digest));
  const sourceBytes = await readFile(fixture('ordinary-pi.jsonl'));
  const source = SessionManager.inMemory('/synthetic/hstry', undefined, sourceBytes.toString().trim().split('\n').map(line => JSON.parse(line)));
  const snapshot = validateSnapshot(JSON.parse(await readFile(fixture('full.pi-durable.json'), 'utf8')));
  const registry = createRegistry();
  let executed = 0;
  registry.install(defineExtension({name: 'golden-guard', tools: [defineTool({name: 'historical_guard', description: 'Must not replay history', parameters: Type.Object({fixture: Type.Number()}), execute: async () => {executed++; return {};}})]}));
  const faux = fauxProvider();
  const models = createModels(); models.setProvider(faux.provider);
  const storage = new MemoryStorage();
  const harness = await Harness.open(storage, {models, registry}, context);
  try {
    const mapping = await restoreSnapshot(snapshot, harness);
    expect(mapping.sidecar).toEqual(snapshot);
    expect(mapping.activationGates.map(gate => gate.code)).toEqual(['host-authorization', 'provider-content-validation']);
    expect(executed).toBe(0); expect(faux.state.callCount).toBe(0);
    const captured = await captureSnapshot({harness, storage}, 'golden-target');
    const output = exportPiSourceTree(captured);
    const artifact = output.files.find(file => file.path.endsWith('.jsonl'))!;
    const target = SessionManager.inMemory('/synthetic/hstry', undefined, artifact.content.trim().split('\n').map(line => JSON.parse(line)));
    expect(JSON.parse(JSON.stringify(target.getEntries()))).toEqual(source.getEntries().map(entry => ({...entry, id: output.report.identityMap[entry.id], parentId: entry.parentId === null ? null : output.report.identityMap[entry.parentId], ...('targetId' in entry ? {targetId: output.report.identityMap[entry.targetId]} : {}), ...('fromId' in entry ? {fromId: entry.fromId ? output.report.identityMap[entry.fromId] : null} : {}), ...('firstKeptEntryId' in entry ? {firstKeptEntryId: entry.firstKeptEntryId ? output.report.identityMap[entry.firstKeptEntryId] : null} : {})})));
    expect(output.report.losses).toEqual([]);
    expect(output.report.activationGates.filter(gate => gate.code === 'provider-signed-content').length).toBe(4);
    expect(output.report.activationGates.some(gate => gate.code === 'provider-image-content')).toBe(true);
    expect(target.getSessionId()).not.toBe(source.getSessionId());
    expect(target.getSessionName()).toBe('Golden interoperability session');
    const leafSource = snapshot.conversations.find(conversation => conversation.entries.some(entry => (entry.data as {hstryPiEntry?: {id?: string}} | undefined)?.hstryPiEntry?.id === source.getLeafId()))!;
    const continuation = (await harness.conversation(mapping.conversations[leafSource.record.id], context))!;
    await continuation.configure({model: {provider: 'faux', modelId: 'faux-1'}}, context);
    faux.setResponses([fauxAssistantMessage('golden native new turn')]);
    expect((await (await continuation.submit({type: 'input', content: 'golden-new-input'}, context)).wait(context)).status).toBe('done');
    expect(executed).toBe(0); expect(faux.state.callCount).toBe(1);
    expect(await readFile(fixture('ordinary-pi.jsonl'))).toEqual(sourceBytes);
    const selected = validateSnapshot(JSON.parse(await readFile(fixture('selected.pi-durable.json'), 'utf8')));
    expect(selected.selection).toBe('selected-branch');
    expect(() => exportPiSourceTree(selected)).toThrow('full-tree');
  } finally {await harness.close(context);}
});
