import {randomUUID} from 'node:crypto';
import {validateModelMessage} from '../pi-durable/messages.ts';
import type {Conversation, ExportFile, ExportOptions} from '../types/index.ts';

export interface NativePiSession {
  header: {type: 'session'; version: 3; id: string; timestamp: string; cwd: string; parentSession?: string};
  entries: NativePiEntry[];
}
export interface NativePiEntry {
  type: string;
  id: string;
  parentId: string | null;
  timestamp: string;
  [key: string]: unknown;
}
export interface FidelityReport {
  protocol: 1;
  target: 'pi';
  disposition: 'detached-continuation';
  selection: 'full-tree' | 'selected-conversation';
  source: {namespace: string; conversationId?: string; storeId?: string};
  targetSessionId: string;
  identityMap: Record<string, string>;
  losses: {code: string; sourceId?: string; detail: string}[];
  sidecar: string;
  activationGates: {code: string; detail: string}[];
}

export function validateNativePiSession(value: unknown): NativePiSession {
  if (Buffer.byteLength(JSON.stringify(value)) > 16 * 1024 * 1024) throw new Error('Native Pi provenance exceeds 16 MiB');
  const session = value as NativePiSession;
  if (Buffer.byteLength(JSON.stringify(session)) > 16 * 1024 * 1024) throw new Error('Native Pi source exceeds bounded 16 MiB conversion limit');
  if (!session || session.header?.type !== 'session' || session.header.version !== 3 || typeof session.header.id !== 'string' || !session.header.id || typeof session.header.cwd !== 'string' || !Number.isFinite(Date.parse(session.header.timestamp)) || !Array.isArray(session.entries)) throw new Error('Unsupported native Pi session schema');
  const seen = new Set<string>();
  for (const entry of session.entries) {
    if (!entry || typeof entry.id !== 'string' || !entry.id || seen.has(entry.id) || typeof entry.type !== 'string' || !Number.isFinite(Date.parse(entry.timestamp)) || (entry.parentId !== null && !seen.has(entry.parentId))) throw new Error('Corrupt Pi entry identity/tree');
    for (const field of ['firstKeptEntryId', 'fromId', 'targetId']) {
      const reference = entry[field];
      if (reference !== undefined && reference !== null && (typeof reference !== 'string' || !seen.has(reference))) throw new Error(`Missing Pi ${field} anchor`);
    }
    if (!['message', 'model_change', 'thinking_level_change', 'usage', 'compaction', 'branch_summary', 'custom', 'custom_message', 'context_edit', 'label', 'session_info'].includes(entry.type)) throw new Error('Unsupported native Pi entry schema');
    if (entry.type === 'message') {
      const message = entry.message as Record<string, unknown>;
      if (['user', 'assistant', 'toolResult'].includes(String(message?.role))) validateModelMessage(message);
      else if (!['custom', 'bashExecution'].includes(String(message?.role))) throw new Error('Unsupported native Pi message role');
    }
    seen.add(entry.id);
  }
  return session;
}

/** Version-3 detached conversion artifact, verified by the pinned SDK tests.
 * Never writes or activates a managed Pi session. Unknown data remains bounded
 * in the accompanying sidecar; source identities never become target identities.
 */
export function exportNativePi(conv: Conversation, source: NativePiSession): {files: ExportFile[]; report: FidelityReport} {
  validateNativePiSession(source);
  const targetSessionId = randomUUID();
  const identityMap = Object.fromEntries(source.entries.map(entry => [entry.id, randomUUID().replaceAll('-', '').slice(0, 8)]));
  if (new Set(Object.values(identityMap)).size !== source.entries.length) throw new Error('Target identity collision; retry conversion');
  const entries = source.entries.map(entry => {
    const result: NativePiEntry = {...entry, id: identityMap[entry.id], parentId: entry.parentId === null ? null : identityMap[entry.parentId]};
    for (const field of ['firstKeptEntryId', 'fromId', 'targetId']) if (typeof entry[field] === 'string') result[field] = identityMap[entry[field] as string];
    return result;
  });
  const prefix = `detached/${targetSessionId}`;
  const report: FidelityReport = {
    protocol: 1, target: 'pi', disposition: 'detached-continuation', selection: 'full-tree',
    source: {namespace: 'pi', conversationId: source.header.id}, targetSessionId, identityMap,
    losses: [], sidecar: `${prefix}.provenance.json`,
    activationGates: [{code: 'host-authorization', detail: 'Detached conversion is not permission to activate a runtime or expose indexed history; the owning host must enforce account/source authorization.'}],
  };
  for (const entry of source.entries) {
    const message = entry.message as {content?: unknown[]} | undefined;
    for (const content of Array.isArray(message?.content) ? message.content : []) {
      const block = content as Record<string, unknown>;
      if (block.thinkingSignature || block.textSignature || block.signature || block.redactedThinking) report.activationGates.push({code: 'provider-signed-content', detail: `Entry ${entry.id}: signature/opaque reasoning retained byte-for-byte but must be validated by the selected provider before activation; synthetic tests do not prove provider acceptance.`});
      if (block.type === 'image') report.activationGates.push({code: 'provider-image-content', detail: `Entry ${entry.id}: image bytes/references are preserved, but the chosen provider must validate MIME/encoding/content before activation; synthetic tests do not prove provider acceptance.`});
    }
  }
  if (source.header.parentSession) report.losses.push({code: 'external-parent-session', detail: 'External session ancestry retained in provenance; no managed session path is activated.'});
  const header = {...source.header, id: targetSessionId};
  delete header.parentSession;
  return {report, files: [
    {path: `${prefix}.jsonl`, content: [header, ...entries].map(entry => JSON.stringify(entry)).join('\n') + '\n'},
    {path: report.sidecar, content: JSON.stringify({source, report})},
    {path: `${prefix}.fidelity.json`, content: JSON.stringify(report)},
  ]};
}
