import { randomUUID, createHash } from 'node:crypto';
import { BACKGROUND_CONTEXT as context } from '@earendil-works/chord/context';
import { Harness, createRegistry, MemoryStorage, defineDoc, InboxDoc, LiveDoc, AgentDoc, ProviderDoc, UsageDoc } from '@earendil-works/pi-durable';
import type { ConversationId, EntryId, Cursor, Storage, ConversationDocToken, JsonObject } from '@earendil-works/pi-durable';
import { createModels } from '@earendil-works/pi-ai/models';
import { SessionManager } from '@earendil-works/pi-coding-agent';
import {projectSnapshot} from './adapter.ts';
import {exportDurableConversation} from './export.ts';

import {SDK_VERSION, sourceVersion, validateSnapshot, type Snapshot} from './snapshot.ts';
export {validateSnapshot, type Snapshot} from './snapshot.ts';

async function scan<T>(read: (cursor?: Cursor) => Promise<{items: readonly T[]; next?: Cursor}>): Promise<T[]> {
  const result: T[] = [];
  let cursor: Cursor | undefined;
  do {
    const page = await read(cursor);
    result.push(...page.items);
    if (result.length > 100000) throw new Error('Snapshot record limit exceeded');
    cursor = page.next;
  } while (cursor);
  return result;
}

/** Called by the existing storage owner; never opens another process's database.
 * Oqto/shared-workspace hosts must pass their independently authorized ID set.
 * The set is a host projection, not an access grant: importing/indexing this
 * snapshot never authorizes later reads or runtime activation.
 */
export interface SnapshotOwner {harness: Harness; storage: Storage}

export async function captureSnapshot(owner: SnapshotOwner, storeId: string, authorizedConversationIds?: ReadonlySet<number>, publicSessionIds?: ReadonlyMap<number, string>): Promise<Snapshot> {
  const {harness, storage} = owner;
  const readDoc = async <T extends JsonObject>(token: ConversationDocToken<T>, conversationId: ConversationId): Promise<Readonly<T> | undefined> => {
    const record = await storage.findDocument({kind: token.definition.kind, scope: {kind: 'conversation', conversationId}}, 'current', context);
    if (!record) return undefined;
    const stored = await storage.document(record.id, 'current', context);
    if (!stored || stored.version !== token.definition.version || !stored.value || typeof stored.value !== 'object' || Array.isArray(stored.value)) throw new Error('Unsupported native document schema');
    return stored.value as Readonly<T>;
  };
  return harness.commit(async tx => {
    const tasks = await scan(cursor => tx.scanTasks({}, 256, cursor));
    if (tasks.some(task => (!authorizedConversationIds || authorizedConversationIds.has(task.conversationId)) && task.state.status !== 'terminal')) throw new Error('Durable store has pending or uncertain work; await quiescence before exporting');
    const allRecords = await scan(cursor => tx.scanConversations({order: 'ascending'}, 256, cursor));
    const records = allRecords.filter(record => !authorizedConversationIds || authorizedConversationIds.has(record.id));
    if (authorizedConversationIds && records.some(record => record.parent && !authorizedConversationIds.has(record.parent.conversationId))) throw new Error('Authorization denies inherited parent history; no snapshot exposed');
    const conversations: Snapshot['conversations'] = [];
    for (const record of records) {
      const inbox = await readDoc(InboxDoc, record.id);
      const live = await readDoc(LiveDoc, record.id);
      if (inbox && !Array.isArray(inbox.items)) throw new Error('Invalid native inbox schema');
      if (inbox?.items.length || live?.run || live?.generation || live?.compactions?.length || live?.tools?.some(tool => tool.status !== 'done')) throw new Error('Durable conversation has pending or uncertain work; await quiescence before exporting');
      const agent = await readDoc(AgentDoc, record.id);
      const provider = await readDoc(ProviderDoc, record.id);
      const usage = await readDoc(UsageDoc, record.id);
      const provenance = JSON.parse(JSON.stringify({publicSessionId: publicSessionIds?.get(record.id), providerSessionId: provider?.sessionId, model: agent?.model, thinkingLevel: agent?.thinkingLevel, usage}));
      conversations.push({record, provenance, entries: await scan(cursor => tx.scanEntries({conversationId: record.id, order: 'ascending'}, 256, cursor))});
    }
    return validateSnapshot({format: 'hstry.pi-durable.snapshot', version: 1, sdkVersion: SDK_VERSION, storeId, capturedAt: Date.now(), sourceVersion: sourceVersion(conversations), selection: authorizedConversationIds ? 'authorized-subtree' : 'full-tree', conversations});
  }, context);
}

const ImportProvenance = defineDoc({
  kind: 'hstry.interop.source', version: 1, scope: 'session',
  initial: () => ({fingerprint: '', conversations: {} as Record<string, number>, entries: {} as Record<string, number>}),
});

/** Passive history only: Session commits append entries, never submissions or tasks. */
export async function restoreSnapshot(snapshot: Snapshot, harness: Harness): Promise<{conversations: Record<string, ConversationId>; entries: Record<string, EntryId>; sidecar: Snapshot; activationGates: {code: string; detail: string}[]; warnings: string[]}> {
  validateSnapshot(snapshot);
  const activationGates = [{code: 'host-authorization', detail: 'Passive restore is not runtime admission or indexed-history authorization; await successful import, discard failed partial targets, and require owning-host authorization before activation.'}, {code: 'provider-content-validation', detail: 'Historical reasoning/signatures/image bytes and references are preserved as data, not validated provider content; selected providers must validate compatibility before model-facing activation.'}];
  const fingerprint = createHash('sha256').update(JSON.stringify({storeId: snapshot.storeId, selection: snapshot.selection, sdkVersion: snapshot.sdkVersion, sourceVersion: snapshot.sourceVersion})).digest('hex');
  const prior = await harness.snapshot(ImportProvenance, context);
  if (prior?.fingerprint === fingerprint) {
    for (const id of [...Object.values(prior.conversations), ...Object.values(prior.entries)]) if (!Number.isSafeInteger(id) || id <= 0) throw new Error('Corrupt persisted import mapping');
    return {conversations: prior.conversations as Record<string, ConversationId>, entries: prior.entries as Record<string, EntryId>, sidecar: structuredClone(snapshot), activationGates, warnings: ['Previously imported source; no records appended.']};
  }
  await harness.commit(async tx => {
    if ((await tx.scanConversations({}, 1)).items.length) throw new Error('Import requires fresh native storage');
  }, context);
  const conversations = new Map<number, ConversationId>();
  const entries = new Map<number, EntryId>();
  for (const source of snapshot.conversations) {
    await harness.commit(async tx => {
      const parent = source.record.parent;
      const record = parent
        ? await tx.forkConversation(conversations.get(parent.conversationId)!, entries.get(parent.at)!, {ownership: {kind: 'ownerless'}})
        : await tx.createConversation({ownership: {kind: 'ownerless'}});
      conversations.set(source.record.id, record.id);
      for (const entry of source.entries) {
        if (entries.has(entry.id)) continue;
        const {id, conversationId, byTaskId, head, edits, ...draft} = entry;
        const created = await tx.appendEntry(record.id, {...draft, ...(head !== undefined ? {head: head === id ? 'self' : entries.get(head)} : {}), ...(edits ? {edits: edits.map(edit => ({...edit, target: entries.get(edit.target)!}))} : {})});
        entries.set(id, created.id);
      }
    }, context);
  }
  const mapping = {conversations: Object.fromEntries(conversations), entries: Object.fromEntries(entries)};
  await harness.commit(async tx => {
    const provenance = await tx.doc(ImportProvenance);
    provenance.fingerprint = fingerprint;
    provenance.conversations = mapping.conversations;
    provenance.entries = mapping.entries;
  }, context);
  return {...mapping, sidecar: structuredClone(snapshot), activationGates, warnings: ['Runtime tasks, documents, provider session identity and execution state are not migrated; new linked continuation only.']};
}

/** Import an explicitly selected ordinary Pi branch using its actual SDK parser. */
export async function snapshotFromPi(entries: Parameters<typeof SessionManager.inMemory>[2], cwd = '.', storeId: string = randomUUID(), selection: 'full-tree' | 'selected-branch' = 'selected-branch'): Promise<Snapshot> {
  const session = SessionManager.inMemory(cwd, undefined, entries);
  const storage = new MemoryStorage();
  const harness = await Harness.open(storage, {registry: createRegistry(), models: createModels()}, context);
  try {
    const root = await harness.root(context);
    await root.commit(tx => tx.appendEntry(root.id, {kind: 'hstry.pi.source', data: JSON.parse(JSON.stringify({sourcePiHeader: session.getHeader(), sourcePiTitle: session.getSessionName()}))}), context);
    const entryIds = new Map<string, EntryId>();
    const entryConversations = new Map<string, ConversationId>();
    const leaves = new Map<ConversationId, string>();
    const sourceEntries = session.getEntries();
    const sourceIndexes = new Map(sourceEntries.map((entry, index) => [entry.id, index]));
    const selected = selection === 'full-tree' ? sourceEntries : session.getBranch();
    for (const entry of selected) {
      let conversationId = root.id;
      if (entry.parentId !== null) {
        const parentConversation = entryConversations.get(entry.parentId);
        const parentEntry = entryIds.get(entry.parentId);
        if (!parentConversation || !parentEntry) throw new Error('Pi branch references an entry outside the selected tree');
        conversationId = leaves.get(parentConversation) === entry.parentId
          ? parentConversation
          : (await harness.commit(tx => tx.forkConversation(parentConversation, parentEntry, {ownership: {kind: 'ownerless'}}), context)).id;
      } else if (leaves.size) {
        conversationId = (await harness.createConversation({ownership: {kind: 'ownerless'}}, context)).id;
      }
      await harness.commit(async tx => {
        const firstKept = entry.type === 'compaction' && entry.firstKeptEntryId ? entryIds.get(entry.firstKeptEntryId) : undefined;
        if (entry.type === 'compaction' && entry.firstKeptEntryId && !firstKept) throw new Error('Pi compaction references an entry outside the selected branch');
        const nativeEdits = entry.type === 'context_edit' ? [{target: entryIds.get(entry.targetId)!, ...(entry.replacement === null ? {action: 'omit' as const} : {action: 'replace' as const, messages: [{...(session.getEntry(entry.targetId) as {message: object}).message, content: entry.replacement.content}]})}] : undefined;
        if (nativeEdits?.some(edit => !edit.target)) throw new Error('Pi context edit references an entry outside the selected branch');
        const created = await tx.appendEntry(conversationId, {
          kind: entry.type === 'message' ? `pi.${entry.message.role === 'toolResult' ? 'tool-result' : entry.message.role}` : `hstry.pi.${entry.type}`,
          ...(entry.type === 'message' && (entry.message.role === 'user' || entry.message.role === 'assistant' || entry.message.role === 'toolResult') ? {model: [entry.message]} : {}),
          ...(entry.type === 'compaction' ? {head: firstKept ?? 'self', model: [{role: 'user', content: [{type: 'text', text: entry.summary}], timestamp: Date.parse(entry.timestamp)}]} : {}),
          ...(nativeEdits ? {edits: JSON.parse(JSON.stringify(nativeEdits))} : {}),
          ...(entry.type === 'custom_message' ? {model: [{role: 'user', content: entry.content, timestamp: Date.parse(entry.timestamp)}]} : {}),
          ...(entry.type === 'branch_summary' ? {model: [{role: 'user', content: [{type: 'text', text: entry.summary}], timestamp: Date.parse(entry.timestamp)}]} : {}),
          data: JSON.parse(JSON.stringify({hstryPiEntry: entry, publicSessionId: session.getSessionId(), sourcePiHeader: session.getHeader(), sourcePiTitle: session.getSessionName(), sourcePiEntryIndex: sourceIndexes.get(entry.id)})),
        });
        entryIds.set(entry.id, created.id);
        entryConversations.set(entry.id, conversationId);
        leaves.set(conversationId, entry.id);
      }, context);
    }
    const snapshot = await captureSnapshot({harness, storage}, storeId);
    // Provider identities of this temporary conversion harness are not source
    // Pi identities. Preserve the actual independent Pi public ID instead.
    snapshot.conversations = snapshot.conversations.map(conversation => ({...conversation, provenance: {publicSessionId: session.getSessionId()}}));
    snapshot.sourceVersion = sourceVersion(snapshot.conversations);
    return {...snapshot, selection};
  } finally { await harness.close(context); }
}

/** Detached SDK-owned target; does not activate or write a managed Pi session. */
export function piFromSnapshot(snapshot: Snapshot, conversationId: number, cwd = '.') {
  validateSnapshot(snapshot);
  const source = projectSnapshot(snapshot).find(conversation => conversation.metadata?.nativeConversationId === conversationId);
  if (!source) throw new Error('Unknown conversation');
  source.workspace = cwd;
  const result = exportDurableConversation(source);
  const artifact = result.files.find(file => file.path.endsWith('.jsonl'))!;
  const session = SessionManager.inMemory(cwd, undefined, artifact.content.trim().split('\n').map(line => JSON.parse(line)));
  const header = session.getHeader();
  if (!header) throw new Error('Pi SDK did not create a session header');
  return {entries: [header, ...session.getEntries()], source: {storeId: snapshot.storeId, conversationId}, sidecar: snapshot, report: result.report, warnings: result.report.losses.map(loss => loss.detail)};
}
