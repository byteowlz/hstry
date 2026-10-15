import assert from 'node:assert/strict';
import {mkdtemp, rm, readFile} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import {Harness, createRegistry, createSession} from '@earendil-works/pi-durable';
import {openNodeSqliteStorage} from '@earendil-works/pi-durable/storage/sqlite/node';
import {createModels} from '@earendil-works/pi-ai/models';
import {captureSnapshot, restoreSnapshot} from './interop.ts';
import {validateSnapshot, type Snapshot} from './snapshot.ts';

const directory = await mkdtemp(join(tmpdir(), 'hstry-sqlite-proof-'));
const options = () => ({models: createModels(), registry: createRegistry()});
try {
  const snapshots: Snapshot[] = [];
  for (const storeId of ['store-a', 'store-b']) {
    const path = join(directory, `${storeId}.sqlite`);
    const storage = await openNodeSqliteStorage(path);
    let writeCount = 0;
    const commit = storage.commit.bind(storage);
    storage.commit = async (...args) => {writeCount++; return commit(...args);};
    const harness = await Harness.open(storage, options(), context);
    try {
      const root = await harness.root(context);
      assert.equal(root.id, 1);
      // Native owner serializes this two-entry commit and the snapshot capture.
      const writing = root.commit(async tx => {
        await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: 'atomic-a', timestamp: 1}]});
        await Promise.resolve();
        await tx.appendEntry(root.id, {kind: 'pi.user', model: [{role: 'user', content: 'atomic-b', timestamp: 2}]});
      }, context);
      const capturing = captureSnapshot({harness, storage}, storeId, undefined, new Map([[root.id, 'shared-public-session-id']]));
      await writing;
      const snapshot = await capturing;
      assert.deepEqual(snapshot.conversations[0].entries.flatMap(entry => entry.model ?? []).map(message => message.content), ['atomic-a', 'atomic-b']);
      const before = writeCount;
      snapshots.push(await captureSnapshot({harness, storage}, storeId, undefined, new Map([[root.id, 'shared-public-session-id']])));
      assert.equal(writeCount, before, 'capturing history must not write to the source store');
    } finally {await harness.close(context);}
    // Reopen through its owner SDK only after the first owner has closed.
    const reopenedStorage = await openNodeSqliteStorage(path);
    const reopened = await Harness.open(reopenedStorage, options(), context);
    try {
      assert.deepEqual((await captureSnapshot({harness: reopened, storage: reopenedStorage}, storeId, undefined, new Map([[1, 'shared-public-session-id']]))).conversations, snapshots.at(-1)!.conversations);
    } finally {await reopened.close(context);}
  }
  // A supported lower-level Session store has no Harness documents. Reading
  // it must not synthesize inbox/live/provider/agent documents into the source.
  const barePath = join(directory, 'bare-session.sqlite');
  const bareSession = createSession(await openNodeSqliteStorage(barePath));
  await bareSession.commit(async tx => {
    const conversation = await tx.createConversation({ownership: {kind: 'ownerless'}});
    await tx.appendEntry(conversation.id, {kind: 'pi.user', model: [{role: 'user', content: 'bare committed source', timestamp: 1}]});
  }, context);
  await bareSession.close(context);
  const bareStorage = await openNodeSqliteStorage(barePath);
  let bareWrites = 0;
  const bareCommit = bareStorage.commit.bind(bareStorage);
  bareStorage.commit = async (...args) => {bareWrites++; return bareCommit(...args);};
  const bareOwner = await Harness.open(bareStorage, options(), context);
  try {
    const before = bareWrites;
    const captured = await captureSnapshot({harness: bareOwner, storage: bareStorage}, 'bare-source');
    assert.equal(captured.conversations.length, 1);
    assert.deepEqual(captured.conversations[0].provenance, {});
    assert.equal(bareWrites, before, 'missing Harness documents must not be created during capture');
  } finally {await bareOwner.close(context);}

  assert.equal(snapshots[0].conversations[0].record.id, snapshots[1].conversations[0].record.id);
  assert.notEqual(snapshots[0].storeId, snapshots[1].storeId);
  assert.equal(snapshots[0].conversations[0].provenance?.publicSessionId, snapshots[1].conversations[0].provenance?.publicSessionId, 'public ID collisions are independent of native store identity');
  const sourceFile = join(directory, 'store-a.sqlite');
  const original = await readFile(sourceFile);
  const targetPath = join(directory, 'target.sqlite');
  const targetStorage = await openNodeSqliteStorage(targetPath);
  const target = await Harness.open(targetStorage, options(), context);
  const targetOwner = {harness: target, storage: targetStorage};
  try {
    const imported = await restoreSnapshot(snapshots[0], target);
    assert.deepEqual(imported.sidecar, snapshots[0], 'all source provenance must survive in the returned bounded sidecar');
    const fresh = await captureSnapshot(targetOwner, 'target');
    assert.notEqual(fresh.conversations[0].provenance?.providerSessionId, snapshots[0].conversations[0].provenance?.providerSessionId, 'source provider identity must not become active target identity');
    const first = (await captureSnapshot(targetOwner, 'target')).conversations;
    await restoreSnapshot({...snapshots[0], capturedAt: snapshots[0].capturedAt + 1}, target);
    assert.deepEqual((await captureSnapshot(targetOwner, 'target')).conversations, first);
    assert.throws(() => validateSnapshot({...snapshots[0], sourceVersion: 'sha256:tampered'}));
  } finally {await target.close(context);}
  assert.deepEqual(await readFile(sourceFile), original, 'conversion modified source database');
  console.log('PASS: SQLite owner snapshots, atomic concurrent writes, read-only capture including bare Session stores, reopen, root1/public-ID collisions, source immutability and idempotent import');
} finally {await rm(directory, {recursive: true, force: true});}
