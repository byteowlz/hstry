import {test, expect} from 'bun:test';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {SessionManager} from '@earendil-works/pi-coding-agent';
import {Harness, MemoryStorage, createRegistry} from '@earendil-works/pi-durable';
import {createModels} from '@earendil-works/pi-ai/models';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import pi from '../pi/adapter.ts';
import {captureSnapshot, snapshotFromPi, restoreSnapshot} from './interop.ts';
import {exportPiSourceTree} from './tree.ts';
import {parseSnapshot} from './adapter.ts';

function loadArtifact(files: {path: string; content: string}[]) {
  const file = files.find(file => file.path.endsWith('.jsonl'))!;
  const entries = file.content.trim().split('\n').map(line => JSON.parse(line));
  return SessionManager.inMemory('/synthetic', undefined, entries);
}

test('existing Pi exporter preserves SDK-created full tree and remaps every reference', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'hstry-pi-tree-'));
  try {
    const source = SessionManager.inMemory('/synthetic');
    const root = source.appendMessage({role: 'user', content: 'root user', timestamp: 1});
    const branchA = source.appendMessage({role: 'user', content: 'branch A', timestamp: 2});
    source.appendLabelChange(branchA, 'A');
    source.branch(root);
    source.appendMessage({role: 'user', content: [{type: 'image', mimeType: 'image/png', data: 'aGVsbG8='}], timestamp: 3});
    source.appendSessionInfo('native tree');
    const path = join(directory, 'fixture.jsonl');
    const original = [source.getHeader(), ...source.getEntries()].map(entry => JSON.stringify(entry)).join('\n') + '\n';
    await writeFile(path, original);
    const conversations = await pi.parse(path, {includeTools: true});
    expect(conversations[0].messages[0].content).toBe('root user');
    expect(conversations[0].messages.some(message => message.parts?.some(part => part.type === 'x-pi-content'))).toBe(true);
    const exported = await pi.export!(conversations, {format: 'pi', includeTools: true});
    const target = loadArtifact(exported.files!);
    const report = (exported.metadata!.fidelityReports as {identityMap: Record<string, string>; losses: unknown[]}[])[0];
    expect(report.losses).toEqual([]);
    expect(target.getSessionId()).not.toBe(source.getSessionId());
    expect(target.getEntries()).toEqual(source.getEntries().map(entry => ({...entry, id: report.identityMap[entry.id], parentId: entry.parentId === null ? null : report.identityMap[entry.parentId], ...('targetId' in entry ? {targetId: report.identityMap[entry.targetId]} : {})})));
    expect(await Bun.file(path).text()).toBe(original);
    expect(await pi.parse(path, {includeTools: true})).toEqual(conversations);
    const native = await snapshotFromPi([source.getHeader()!, ...source.getEntries()], '/synthetic', 'tree-store', 'full-tree');
    expect(native.conversations.length).toBe(2);
    const restoredStorage = new MemoryStorage();
    const restoredHarness = await Harness.open(restoredStorage, {models: createModels(), registry: createRegistry()}, context);
    try {
      await restoreSnapshot(native, restoredHarness);
      const committed = await captureSnapshot({harness: restoredHarness, storage: restoredStorage}, 'restored-tree');
      const tree = exportPiSourceTree(committed);
      const roundTrip = loadArtifact(tree.files);
      expect(roundTrip.getEntries()).toEqual(source.getEntries().map(entry => ({...entry, id: tree.report.identityMap[entry.id], parentId: entry.parentId === null ? null : tree.report.identityMap[entry.parentId], ...('targetId' in entry ? {targetId: tree.report.identityMap[entry.targetId]} : {})})));
      expect(roundTrip.getSessionName()).toBe('native tree');
      expect(tree.report.losses).toEqual([]);
    } finally {await restoredHarness.close(context);}
  } finally {await rm(directory, {recursive: true, force: true});}
});

test('existing Pi exporter preserves Durable reset and context omission without reactivating archived text', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'hstry-native-reset-'));
  const storage = new MemoryStorage();
  const harness = await Harness.open(storage, {models: createModels(), registry: createRegistry()}, context);
  try {
    const root = await harness.root(context);
    await root.commit(async tx => {
      await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: 'ARCHIVED', timestamp: 1}]});
      await tx.appendEntry(root.id, {kind: 'pi.reset', head: 'self'});
      const omitted = await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: 'OMITTED', timestamp: 2}]});
      await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: 'ACTIVE', timestamp: 3}], edits: [{target: omitted.id, action: 'omit'}]});
    }, context);
    const snapshot = await captureSnapshot({harness, storage}, 'reset-store');
    const file = join(directory, 'reset.pi-durable.json');
    await writeFile(file, JSON.stringify(snapshot));
    const conversations = await parseSnapshot(file);
    const result = await pi.export!(conversations, {format: 'pi', includeTools: true});
    const target = loadArtifact(result.files!);
    expect(target.getEntries().flatMap(entry => entry.type === 'message' && 'content' in entry.message ? [entry.message.content] : [])).toEqual(['ARCHIVED', 'OMITTED', 'ACTIVE']);
    expect(target.buildSessionContext().messages.flatMap(message => 'content' in message ? [message.content] : [])).toEqual(['ACTIVE']);
    const sidecar = JSON.parse(result.files!.find(file => file.path.endsWith('.provenance.json'))!.content);
    expect(sidecar.conversation).toEqual(conversations[0]);
  } finally {await harness.close(context); await rm(directory, {recursive: true, force: true});}
});
