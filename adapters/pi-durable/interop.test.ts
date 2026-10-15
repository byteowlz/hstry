import {test, expect} from 'bun:test';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import {Harness, MemoryStorage, createRegistry, defineExtension, defineTool} from '@earendil-works/pi-durable';
import {createModels} from '@earendil-works/pi-ai/models';
import type {EntryId} from '@earendil-works/pi-durable';
import {Type} from '@earendil-works/pi-ai';
import {fauxProvider, fauxAssistantMessage} from '@earendil-works/pi-ai/providers/faux';
import {SessionManager} from '@earendil-works/pi-coding-agent';
import {captureSnapshot, restoreSnapshot, piFromSnapshot, snapshotFromPi, validateSnapshot} from './interop.ts';
import {parseSnapshot} from './adapter.ts';
const options = () => ({registry: createRegistry(), models: createModels()});

test('actual SDK passive import preserves duplicate-text tool associations, forks, heads and reasoning', async () => {
  const sourceStorage = new MemoryStorage();
  const harness = await Harness.open(sourceStorage, options(), context);
  const faux = fauxProvider();
  const targetOptions = options();
  let toolExecutions = 0;
  targetOptions.registry.install(defineExtension({name: 'fixture', tools: [defineTool({name: 'must-not-run', description: 'Historical tool execution guard', parameters: Type.Object({}), execute: async () => {toolExecutions++; return {};}})]}));
  targetOptions.models.setProvider(faux.provider);
  const targetStorage = new MemoryStorage();
  const target = await Harness.open(targetStorage, targetOptions, context);
  const directory = await mkdtemp(join(tmpdir(), 'hstry-durable-test-'));
  try {
    const root = await harness.root(context);
    let first: EntryId;
    await root.commit(async tx => {
      first = (await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: [{type: 'text', text: 'hello'}, {type: 'image', mimeType: 'image/png', data: 'aGVsbG8='}], timestamp: 1000}]})).id;
      for (const id of ['call-a', 'call-b']) {
        await tx.appendEntry(root.id, {kind: 'pi.assistant', model: [{role: 'assistant', api: 'openai-responses', provider: 'synthetic', model: 'synthetic', content: [{type: 'thinking', thinking: 'reasoning'}, {type: 'toolCall', id, name: 'must-not-run', arguments: {}}], usage: {input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: {input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0}}, stopReason: 'toolUse', timestamp: 1001}]});
        await tx.appendEntry(root.id, {kind: 'pi.tool-result', model: [{role: 'toolResult', toolCallId: id, toolName: 'must-not-run', content: [{type: 'text', text: 'same result'}], isError: id === 'call-b', timestamp: 1002}]});
      }
      await tx.appendEntry(root.id, {kind: 'pi.reset', head: 'self', data: {note: 'reset'}});
    }, context);
    await root.commit(tx => tx.forkConversation(root.id, first!, {ownership: {kind: 'ownerless'}}), context);
    const snapshot = await captureSnapshot({harness, storage: sourceStorage}, 'store-a');
    const mapping = await restoreSnapshot(snapshot, target);
    const restored = await captureSnapshot({harness: target, storage: targetStorage}, 'target');
    expect(restored.conversations.map(c => c.entries)).toEqual(snapshot.conversations.map(c => c.entries.map(e => ({...e, id: mapping.entries[e.id], conversationId: mapping.conversations[e.conversationId], ...(e.head !== undefined ? {head: mapping.entries[e.head]} : {})}))));
    expect((await target.inspect(context)).tasks).toEqual([]);
    expect(faux.state.callCount).toBe(0);
    const repeated = await restoreSnapshot({...snapshot, capturedAt: snapshot.capturedAt + 1000}, target);
    expect({conversations: repeated.conversations, entries: repeated.entries}).toEqual({conversations: mapping.conversations, entries: mapping.entries});
    expect((await captureSnapshot({harness: target, storage: targetStorage}, 'target')).conversations).toEqual(restored.conversations);
    await expect(restoreSnapshot({...snapshot, storeId: 'other'}, target)).rejects.toThrow('fresh');
    const file = join(directory, 'history.pi-durable.json');
    await writeFile(file, JSON.stringify(snapshot));
    const firstParse = await parseSnapshot(file);
    expect(await parseSnapshot(file)).toEqual(firstParse);
    await writeFile(file, JSON.stringify({...snapshot, storeId: 'store-b'}));
    expect((await parseSnapshot(file))[0].externalId).not.toBe(firstParse[0].externalId);
    const exported = piFromSnapshot(snapshot, root.id);
    const pi = SessionManager.inMemory('.', undefined, exported.entries);
    expect(pi.getEntries().filter(e => e.type === 'message').length).toBe(5);
    const inverse = await snapshotFromPi(exported.entries, '.', 'inverse-store', 'full-tree');
    expect(inverse.conversations.flatMap(c => c.entries).flatMap(e => e.model ?? []).length).toBe(5);
    expect(pi.buildSessionContext().messages).toEqual([]);
    expect(() => validateSnapshot({...snapshot, sdkVersion: '9'})).toThrow();
    expect(() => validateSnapshot({...snapshot, conversations: [...snapshot.conversations, snapshot.conversations[0]]})).toThrow();
    const continuation = (await target.conversation(mapping.conversations[root.id], context))!;
    await continuation.configure({model: {provider: 'faux', modelId: 'faux-1'}}, context);
    faux.setResponses([fauxAssistantMessage('new turn')]);
    const settled = await (await continuation.submit({type: 'input', content: 'continue'}, context)).wait(context);
    expect(settled.status).toBe('done');
    expect(faux.state.callCount).toBe(1);
    expect(toolExecutions).toBe(0);
  } finally {
    await harness.close(context); await target.close(context); await rm(directory, {recursive: true, force: true});
  }
});
