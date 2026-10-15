import type {Conversation} from '../types/index.ts';
import type {NativePiEntry, NativePiSession, FidelityReport} from '../pi/native.ts';
import {exportNativePi} from '../pi/native.ts';
import type {EntryRecord} from '@earendil-works/pi-durable';

/** Pure detached-artifact builder used by the installed Pi exporter, not a runtime owner. */
export function exportDurableConversation(conversation: Conversation) {
  const records = new Map<number, EntryRecord>();
  for (const message of conversation.messages) {
    const entry = message.metadata?.nativeEntry as EntryRecord | undefined;
    if (!entry || !Number.isSafeInteger(entry.id)) throw new Error('Missing native Durable entry provenance');
    if (records.has(entry.id) && JSON.stringify(records.get(entry.id)) !== JSON.stringify(entry)) throw new Error('Conflicting native Durable entry provenance');
    records.set(entry.id, entry);
  }
  const source: NativePiSession = {header: {type: 'session', version: 3, id: conversation.externalId!, cwd: conversation.workspace ?? '.', timestamp: new Date(conversation.createdAt).toISOString()}, entries: []};
  let parentId: string | null = null;
  const anchors = new Map<number, string[]>();
  const originalIds = new Map<string, string>();
  const losses: FidelityReport['losses'] = [];
  const append = (type: string, data: Record<string, unknown>, sourceId: number): string => {
    const id = `native-${sourceId}-${source.entries.length}`;
    source.entries.push({type, id, parentId, timestamp: new Date(conversation.createdAt).toISOString(), ...data});
    parentId = id;
    return id;
  };
  for (const entry of records.values()) {
    const original = (entry.data as {hstryPiEntry?: NativePiEntry} | undefined)?.hstryPiEntry;
    if (original) {
      const {id: sourceId, parentId: sourceParent, ...data} = original;
      for (const field of ['firstKeptEntryId', 'fromId', 'targetId']) {
        if (typeof data[field] === 'string') {
          const mapped = originalIds.get(data[field] as string);
          if (!mapped) throw new Error(`Missing selected Pi ${field} anchor`);
          data[field] = mapped;
        }
      }
      const id = append(original.type, data, entry.id);
      originalIds.set(original.id, id);
      anchors.set(entry.id, [id]);
      continue;
    }
    if (entry.head === entry.id) {
      // New root isolates old branches from the active model context, without deleting archives.
      parentId = null;
      append('custom', {customType: 'hstry.durable.reset', data: {sourceEntryId: entry.id}}, entry.id);
    } else if (entry.head !== undefined) {
      const kept = anchors.get(entry.head)?.[0];
      if (!kept) throw new Error('Missing Durable context-head anchor');
      const summary = (entry.model ?? []).flatMap(message => typeof message.content === 'string' ? [message.content] : message.content.filter(block => block.type === 'text').map(block => block.text)).join('\n');
      const id = append('compaction', {summary, firstKeptEntryId: kept, tokensBefore: 0}, entry.id);
      anchors.set(entry.id, [id]);
      losses.push({code: 'compaction-accounting', sourceId: String(entry.id), detail: 'Native model content retained as Pi summary; unavailable Pi tokensBefore accounting is zero, original entry is in provenance.'});
      continue;
    }
    const ids: string[] = [];
    for (const message of entry.model ?? []) ids.push(append('message', {message}, entry.id));
    if (!ids.length) ids.push(append('custom', {customType: 'hstry.durable.entry', data: entry}, entry.id));
    anchors.set(entry.id, ids);
    for (const edit of entry.edits ?? []) {
      const targets = anchors.get(edit.target);
      if (!targets) throw new Error('Missing Durable context-edit anchor');
      if (edit.action === 'omit') for (const targetId of targets) append('context_edit', {targetId, replacement: null}, entry.id);
      else {
        if (targets.length !== 1 || edit.messages.length !== 1) throw new Error('Durable multi-message context replacement cannot be represented faithfully by Pi');
        const targetEntry = records.get(edit.target);
        const prior = targetEntry?.model?.[0];
        const replacement = edit.messages[0];
        if (!prior || prior.role !== replacement.role) throw new Error('Durable role-changing context edit cannot be represented faithfully by Pi');
        const {content: priorContent, ...priorMetadata} = prior;
        const {content: replacementContent, ...replacementMetadata} = replacement;
        if (JSON.stringify(priorMetadata) !== JSON.stringify(replacementMetadata)) throw new Error('Durable context edit changes metadata Pi cannot replace');
        append('context_edit', {targetId: targets[0], replacement: {content: replacementContent}}, entry.id);
      }
    }
    if (entry.data !== undefined) losses.push({code: 'application-data', sourceId: String(entry.id), detail: 'Application data retained in bounded sidecar, not executable Pi state.'});
    if (entry.byTaskId !== undefined) losses.push({code: 'task-attribution', sourceId: String(entry.id), detail: 'Historical task attribution retained only in provenance; tasks are not reconstructed.'});
  }
  if (conversation.title) append('session_info', {name: conversation.title}, -1);
  const exported = exportNativePi(conversation, source);
  exported.report.selection = 'selected-conversation';
  exported.report.source = {namespace: 'pi-durable', storeId: conversation.metadata?.storeId as string, conversationId: conversation.externalId};
  exported.report.losses.push(...losses, {code: 'runtime-state', detail: 'New continuation: no tasks, queues, permissions, provider-session identity or process/workspace state imported.'});
  if (conversation.parentExternalId || (conversation.metadata?.nativeConversation as {parent?: unknown} | undefined)?.parent) exported.report.losses.push({code: 'external-parent-conversation', detail: 'Fork ancestry remains in source provenance; this artifact selects one conversation explicitly.'});
  exported.files.find(file => file.path.endsWith('.fidelity.json'))!.content = JSON.stringify(exported.report);
  exported.files.find(file => file.path.endsWith('.provenance.json'))!.content = JSON.stringify({conversation, report: exported.report});
  return exported;
}
