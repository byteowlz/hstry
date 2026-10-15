import {readFile, readdir, stat} from 'node:fs/promises';
import {join} from 'node:path';
import {homedir} from 'node:os';
import type {Adapter, Conversation, CanonPart, Message} from '../types/index.ts';
import {runAdapter, isUnderCanonicalRoot} from '../types/index.ts';
import {validateSnapshot, type Snapshot} from './snapshot.ts';

export async function parseSnapshot(path: string): Promise<Conversation[]> {
  const bytes = await readFile(path);
  if (bytes.byteLength > 16 * 1024 * 1024) throw new Error('Snapshot exceeds 16 MiB limit');
  const snapshot = validateSnapshot(JSON.parse(bytes.toString('utf8')));
  return projectSnapshot(snapshot);
}

export function projectSnapshot(snapshot: Snapshot): Conversation[] {
  validateSnapshot(snapshot);
  const external = (id: number) => `pi-durable:${encodeURIComponent(snapshot.storeId)}:${id}`;
  return snapshot.conversations.map(({record, entries, provenance}) => {
    const messages: Message[] = [];
    for (const entry of entries) {
      const native = entry.model ?? [];
      if (!native.length) messages.push({role: 'system', content: JSON.stringify(entry.data ?? {kind: entry.kind}), metadata: {sourceMessageId: `${external(record.id)}:${entry.id}:event`, sourceEntryId: entry.id, nativeEntry: entry}});
      for (const [messageIndex, message] of native.entries()) {
        const anchor = `${external(record.id)}:${entry.id}:${messageIndex}`;
        const content = typeof message.content === 'string' ? [{type: 'text' as const, text: message.content}] : message.content;
        const parts: CanonPart[] = (content ?? []).map((block, i) => {
          const id = `${anchor}:${i}`;
          if (block.type === 'text') return {id, type: 'text', text: block.text, meta: {native: block}};
          if (block.type === 'thinking') return {id, type: 'thinking', text: block.thinking, meta: {native: block}};
          if (block.type === 'toolCall') return {id, type: 'tool_call', toolCallId: block.id, name: block.name, input: block.arguments, meta: {native: block}};
          return {id, type: 'x-pi-durable', payload: block};
        });
        if (message.role === 'toolResult') parts.unshift({id: `${anchor}:result`, type: 'tool_result', toolCallId: message.toolCallId, name: message.toolName, isError: message.isError, output: message.content});
        messages.push({role: message.role === 'toolResult' ? 'tool' : message.role, content: parts.filter(p => p.type === 'text' || p.type === 'thinking').map(p => p.text).join('\n'), parts, createdAt: Number.isSafeInteger(message.timestamp) ? message.timestamp : undefined, ...(message.role === 'assistant' ? {model: message.model, tokens: message.usage?.totalTokens, costUsd: message.usage?.cost?.total} : {}), metadata: {sourceMessageId: anchor, sourceEntryId: entry.id, sourceConversationId: entry.conversationId, sourceMessageIndex: messageIndex, nativeEntry: entry}});
      }
    }
    const dates = messages.map(m => m.createdAt).filter((t): t is number => t !== undefined);
    const namedEntry = [...entries].reverse().find(entry => (entry.data as {hstryPiEntry?: {type?: string}} | undefined)?.hstryPiEntry?.type === 'session_info');
    const title = (namedEntry?.data as {hstryPiEntry?: {name?: string}} | undefined)?.hstryPiEntry?.name ?? (entries.find(entry => (entry.data as {sourcePiTitle?: string} | undefined)?.sourcePiTitle)?.data as {sourcePiTitle?: string} | undefined)?.sourcePiTitle;
    return {externalId: external(record.id), title, createdAt: dates.length ? Math.min(...dates) : snapshot.capturedAt, updatedAt: dates.length ? Math.max(...dates) : snapshot.capturedAt, messages, ...(record.parent ? {parentExternalId: external(record.parent.conversationId), forkType: 'fork' as const} : {}), metadata: {storeId: snapshot.storeId, nativeConversationId: record.id, sourceCursor: {sourceVersion: snapshot.sourceVersion, capturedAt: snapshot.capturedAt, lastEntryId: entries.at(-1)?.id}, selection: snapshot.selection, nativeConversation: record, sourceProvenance: provenance, sdkVersion: snapshot.sdkVersion}};
  });
}

async function snapshotPaths(path: string): Promise<string[]> {
  if (!(await stat(path)).isDirectory()) return path.endsWith('.pi-durable.json') ? [path] : [];
  return (await readdir(path, {withFileTypes: true})).filter(entry => entry.isFile() && entry.name.endsWith('.pi-durable.json')).map(entry => join(path, entry.name)).sort();
}

const SNAPSHOT_ROOT = join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local', 'share'), 'hstry', 'pi-durable-snapshots');
const adapter: Adapter = {
  info: () => ({name: 'pi-durable', displayName: 'Pi Durable committed snapshot', version: '1.0.0', defaultPaths: [SNAPSHOT_ROOT]}),
  async detect(path) {
    if (!isUnderCanonicalRoot(path, SNAPSHOT_ROOT)) return null;
    try {
      const paths = await snapshotPaths(path);
      if (!paths.length) return null;
      for (const file of paths) await parseSnapshot(file);
      return 1;
    } catch { return null; }
  },
  async parse(path, options = {}) {
    let conversations: Conversation[] = [];
    for (const file of await snapshotPaths(path)) conversations.push(...await parseSnapshot(file));
    const ids = conversations.map(conversation => conversation.externalId);
    if (new Set(ids).size !== ids.length) throw new Error('Duplicate source conversations across snapshots; select one committed snapshot per store');
    if (options.since !== undefined) conversations = conversations.filter(c => (c.updatedAt ?? c.createdAt) >= options.since!);
    return options.limit !== undefined ? conversations.slice(0, options.limit) : conversations;
  },
};
if (import.meta.main) runAdapter(adapter);
export default adapter;
