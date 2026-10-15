import type {Conversation, ExportOptions} from '../types/index.ts';
import {exportNativePi, type NativePiEntry, type FidelityReport} from './native.ts';

/** Canonical parts are authoritative; legacy tool names are never treated as IDs. */
export function exportGenericPi(conversation: Conversation, options: ExportOptions) {
  if (Buffer.byteLength(JSON.stringify(conversation)) > 16 * 1024 * 1024) throw new Error('Canonical source exceeds bounded 16 MiB conversion limit');
  const knownString = (value: unknown) => typeof value === 'string' && value.length ? value : undefined;
  const entries: NativePiEntry[] = [];
  const losses: FidelityReport['losses'] = [];
  let parentId: string | null = null;
  const append = (type: string, data: Record<string, unknown>, timestamp: number) => {
    const id = `input-${entries.length}`;
    entries.push({type, id, parentId, timestamp: new Date(timestamp).toISOString(), ...data});
    parentId = id;
  };
  if (conversation.model) append('model_change', {provider: conversation.provider ?? 'unknown', modelId: conversation.model}, conversation.createdAt);
  if (conversation.title) append('session_info', {name: conversation.title}, conversation.createdAt);
  for (const message of conversation.messages) {
    const timestamp = message.createdAt ?? conversation.createdAt;
    const parts = message.parts;
    const blocks: Record<string, unknown>[] = [];
    const resultParts = parts?.filter(part => part.type === 'tool_result') ?? [];
    for (const part of parts ?? []) {
      const native = part.meta?.native as Record<string, unknown> | undefined;
      if (part.type === 'text') blocks.push({...native, type: 'text', text: part.text});
      else if (part.type === 'thinking') blocks.push({...native, type: 'thinking', thinking: part.text});
      else if (part.type === 'tool_call') {
        if (options.includeTools !== false) blocks.push({...native, type: 'toolCall', id: part.toolCallId, name: part.name, arguments: part.input ?? {}});
        else losses.push({code: 'filtered-tool-call', detail: `Tool call ${part.toolCallId} explicitly excluded.`});
      } else if (part.type === 'x-pi-content' || part.type === 'x-pi-durable') {
        const block = part.payload as Record<string, unknown>;
        if (block?.type === 'image' && options.includeAttachments !== false) blocks.push(block);
        else losses.push({code: 'unsupported-part', detail: `Part ${part.id} retained in provenance, not active Pi content.`});
      } else if (part.type !== 'tool_result') losses.push({code: 'unsupported-part', detail: `Part ${part.id} retained in provenance, not active Pi content.`});
    }
    if (!parts?.length && message.content) blocks.push({type: 'text', text: message.content});
    if (message.toolCalls?.length && !parts?.some(part => part.type === 'tool_call') && options.includeTools !== false) throw new Error('Legacy tool calls lack native IDs; refusing to fabricate tool associations');
    if (message.role === 'system') {
      append('custom_message', {customType: 'hstry.system', content: blocks.length ? blocks : message.content, display: true, details: message.metadata}, timestamp);
      losses.push({code: 'system-role', detail: 'Generic system content is displayed as a Pi custom user message; provider system semantics are not reconstructed.'});
    } else if (message.role === 'tool') {
      if (options.includeTools === false) {losses.push({code: 'filtered-tool-result', detail: 'Tool result explicitly excluded; native data remains in provenance.'}); continue;}
      if (resultParts.length > 1) throw new Error('Multiple tool results in one message cannot be represented by Pi');
      const result = resultParts[0];
      const toolCallId = result?.toolCallId ?? message.metadata?.toolCallId;
      if (typeof toolCallId !== 'string' || !toolCallId) throw new Error('Tool result lacks native toolCallId');
      const output = result?.output;
      append('message', {message: {role: 'toolResult', toolCallId, toolName: result?.name ?? message.metadata?.toolName ?? 'unknown', content: Array.isArray(output) ? output : blocks.length ? blocks : [{type: 'text', text: typeof output === 'string' ? output : message.content}], isError: result?.isError ?? message.metadata?.isError ?? false, timestamp}}, timestamp);
    } else {
      if (resultParts.length) throw new Error('Tool result embedded in a non-tool message cannot be represented faithfully');
      const model = knownString(message.model) ?? knownString(conversation.model) ?? 'unknown';
      append('message', {message: {role: message.role, content: blocks, timestamp, ...(message.role === 'assistant' ? {model, provider: knownString(message.metadata?.provider) ?? knownString(conversation.provider) ?? 'unknown', api: knownString(message.metadata?.api) ?? 'unknown', stopReason: knownString(message.metadata?.stopReason) ?? (blocks.some(block => block.type === 'toolCall') ? 'toolUse' : 'stop'), usage: {input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: message.tokens ?? 0, cost: {input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: message.costUsd ?? 0}}} : {})}}, timestamp);
      if (message.role === 'assistant') losses.push({code: 'generic-assistant-metadata', detail: 'Unavailable native provider/usage detail uses explicit unknown/zero values; original canonical record retained in sidecar.'});
    }
  }
  const result = exportNativePi(conversation, {header: {type: 'session', version: 3, id: conversation.externalId ?? 'unidentified-canonical-source', timestamp: new Date(conversation.createdAt).toISOString(), cwd: conversation.workspace ?? '.'}, entries});
  const origin = conversation.metadata?.hstry_origin as {external_id?: string} | undefined;
  result.report.source = {namespace: 'canonical', conversationId: origin?.external_id ?? conversation.externalId};
  result.report.selection = 'selected-conversation';
  result.report.losses.push(...losses);
  result.files.find(file => file.path.endsWith('.provenance.json'))!.content = JSON.stringify({conversation, report: result.report});
  result.files.find(file => file.path.endsWith('.fidelity.json'))!.content = JSON.stringify(result.report);
  return result;
}
