import {createHash} from 'node:crypto';
import {validateModelMessage} from './messages.ts';
import type {ConversationRecord, EntryRecord} from '@earendil-works/pi-durable';
export const SDK_VERSION = '1.1.0';
export interface Snapshot {
  format: 'hstry.pi-durable.snapshot';
  version: 1;
  sdkVersion: '1.1.0';
  storeId: string;
  capturedAt: number;
  /** Content version of committed source records, not a fabricated native commit sequence. */
  sourceVersion: string;
  selection: 'full-tree' | 'selected-branch' | 'authorized-subtree';
  conversations: {
    record: ConversationRecord;
    entries: EntryRecord[];
    /** Historical hints only; never native runtime identity/permission restoration. */
    provenance?: {publicSessionId?: string; providerSessionId?: string; model?: unknown; thinkingLevel?: unknown; usage?: unknown};
  }[];
}
export function sourceVersion(conversations: Snapshot['conversations']): string {
  return `sha256:${createHash('sha256').update(JSON.stringify(conversations)).digest('hex')}`;
}
const MAX_BYTES = 16 * 1024 * 1024;
const identity = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) > 0;

export function validateSnapshot(value: unknown): Snapshot {
  if (Buffer.byteLength(JSON.stringify(value)) > MAX_BYTES) throw new Error('Snapshot exceeds 16 MiB limit');
  const s = value as Snapshot;
  if (!s || s.format !== 'hstry.pi-durable.snapshot' || s.version !== 1 || s.sdkVersion !== SDK_VERSION ||
      typeof s.storeId !== 'string' || !s.storeId || !Number.isSafeInteger(s.capturedAt) ||
      !['full-tree', 'selected-branch', 'authorized-subtree'].includes(s.selection) || !Array.isArray(s.conversations)) throw new Error('Unsupported Durable snapshot');
  const conversations = new Map<number, typeof s.conversations[number]>();
  const entries = new Map<number, EntryRecord>();
  for (const conversation of s.conversations) {
    if (!identity(conversation.record?.id) || conversations.has(conversation.record.id) || !Array.isArray(conversation.entries)) throw new Error('Invalid conversation identity');
    if (conversation.provenance) for (const key of ['publicSessionId', 'providerSessionId'] as const) {
      const value = conversation.provenance[key];
      if (value !== undefined && (typeof value !== 'string' || !value)) throw new Error('Invalid independent source session provenance');
    }
    conversations.set(conversation.record.id, conversation);
    let last = 0;
    for (const entry of conversation.entries) {
      if (!identity(entry.id) || entry.id <= last || !identity(entry.conversationId) || typeof entry.kind !== 'string' ||
          (entry.model !== undefined && !Array.isArray(entry.model))) throw new Error('Invalid entry');
      if (entries.has(entry.id) && JSON.stringify(entries.get(entry.id)) !== JSON.stringify(entry)) throw new Error('Conflicting entry identity');
      entries.set(entry.id, entry);
      last = entry.id;
    }
  }
  const preceding = new Set<number>();
  for (const conversation of s.conversations) {
    if (conversation.record.parent && (!preceding.has(conversation.record.parent.conversationId) || !entries.has(conversation.record.parent.at))) throw new Error('Missing or out-of-order fork source');
    preceding.add(conversation.record.id);
    const parent = conversation.record.parent;
    const parentEntries = parent ? conversations.get(parent.conversationId)!.entries : [];
    if (parent && !parentEntries.some(entry => entry.id === parent.at)) throw new Error('Fork anchor is not visible in its parent');
    const inherited = parent ? parentEntries.filter(entry => entry.id <= parent.at) : [];
    if (inherited.length > conversation.entries.length || inherited.some((entry, index) => JSON.stringify(entry) !== JSON.stringify(conversation.entries[index]))) throw new Error('Incomplete or conflicting fork inheritance');
    if (conversation.entries.slice(inherited.length).some(entry => entry.conversationId !== conversation.record.id)) throw new Error('Foreign entries outside fork inheritance');
    const visible = new Set(conversation.entries.map(entry => entry.id));
    for (const entry of conversation.entries) {
      if (!conversations.has(entry.conversationId) || (entry.head !== undefined && !visible.has(entry.head))) throw new Error('Missing entry source/head');
      if (entry.head !== undefined && entry.head > entry.id) throw new Error('Forward head reference');
      for (const edit of entry.edits ?? []) {
        if (!visible.has(edit.target) || edit.target >= entry.id || !['omit', 'replace'].includes(edit.action)) throw new Error('Invalid edit target/action');
        if (edit.action === 'replace') {
          if (!Array.isArray(edit.messages)) throw new Error('Invalid context replacement');
          for (const message of edit.messages) validateModelMessage(message);
        }
      }
      for (const message of entry.model ?? []) {
        validateModelMessage(message);
      }
    }
  }
  if (s.sourceVersion !== sourceVersion(s.conversations)) throw new Error('Snapshot source version does not match committed records');
  return s;
}

