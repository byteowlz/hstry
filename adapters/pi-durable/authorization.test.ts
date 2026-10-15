import {test, expect} from 'bun:test';
import {Harness, MemoryStorage, createRegistry} from '@earendil-works/pi-durable';
import {createModels} from '@earendil-works/pi-ai/models';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import {captureSnapshot} from './interop.ts';
import {projectSnapshot} from './adapter.ts';

test('host-authorized snapshot excludes another account and fails closed on denied fork inheritance', async () => {
  const storage = new MemoryStorage();
  const harness = await Harness.open(storage, {models: createModels(), registry: createRegistry()}, context);
  try {
    const visible = await harness.root(context);
    const hidden = await harness.createConversation({ownership: {kind: 'ownerless'}}, context);
    await visible.commit(tx => tx.appendEntry(visible.id, {kind: 'pi.user', model: [{role: 'user', content: 'authorized history', timestamp: 1}]}), context);
    const secret = await hidden.commit(tx => tx.appendEntry(hidden.id, {kind: 'pi.user', model: [{role: 'user', content: 'OTHER_ACCOUNT_SECRET', timestamp: 1}]}), context);
    const permitted = await captureSnapshot({harness, storage}, 'shared-store', new Set([visible.id]));
    expect(permitted.selection).toBe('authorized-subtree');
    expect(projectSnapshot(permitted).map(conversation => conversation.metadata?.nativeConversationId)).toEqual([visible.id]);
    expect(JSON.stringify(permitted)).not.toContain('OTHER_ACCOUNT_SECRET');
    expect((await captureSnapshot({harness, storage}, 'shared-store', new Set())).conversations).toEqual([]);
    const fork = await harness.commit(tx => tx.forkConversation(hidden.id, secret.id, {ownership: {kind: 'ownerless'}}), context);
    await expect(captureSnapshot({harness, storage}, 'shared-store', new Set([fork.id]))).rejects.toThrow('Authorization denies inherited parent history');
  } finally {await harness.close(context);}
});
