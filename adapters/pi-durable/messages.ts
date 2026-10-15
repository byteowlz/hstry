/** Validate the pinned 1.1.0 model-facing schema before any target writes.
 * Opaque application data/signatures remain untouched; provider verification
 * is a separate activation gate, never inferred from these shape checks.
 */
export function validateModelMessage(value: unknown): void {
  const message = value as Record<string, unknown>;
  if (!message || !['user', 'assistant', 'toolResult'].includes(String(message.role)) || !Number.isSafeInteger(message.timestamp) || Number(message.timestamp) < 0) throw new Error('Invalid native message identity/timestamp');
  if (message.role === 'user' && typeof message.content === 'string') return;
  if (!Array.isArray(message.content)) throw new Error('Native model content must be an array');
  for (const value of message.content) {
    const block = value as Record<string, unknown>;
    if (!block || typeof block.type !== 'string') throw new Error('Invalid native content block');
    if (block.type === 'text') {if (typeof block.text !== 'string') throw new Error('Invalid text block');}
    else if (block.type === 'image' && message.role !== 'assistant') {
      if (typeof block.data !== 'string' || typeof block.mimeType !== 'string' || !block.mimeType.startsWith('image/')) throw new Error('Invalid native image block');
    } else if (block.type === 'thinking' && message.role === 'assistant') {
      if (typeof block.thinking !== 'string') throw new Error('Invalid reasoning block');
    } else if (block.type === 'toolCall' && message.role === 'assistant') {
      if (typeof block.id !== 'string' || !block.id || typeof block.name !== 'string' || !block.name || !block.arguments || typeof block.arguments !== 'object' || Array.isArray(block.arguments)) throw new Error('Invalid native tool-call identity/arguments');
    } else throw new Error('Unsupported native model content schema; preserve it as opaque application provenance, not active model content');
  }
  if (message.role === 'toolResult' && (typeof message.toolCallId !== 'string' || !message.toolCallId || typeof message.toolName !== 'string' || typeof message.isError !== 'boolean')) throw new Error('Invalid native tool-result association');
  if (message.role === 'assistant') {
    for (const field of ['provider', 'model', 'api', 'stopReason']) if (typeof message[field] !== 'string' || !message[field]) throw new Error(`Missing native assistant ${field}`);
    const usage = message.usage as Record<string, unknown>;
    const cost = usage?.cost as Record<string, unknown>;
    for (const field of ['input', 'output', 'cacheRead', 'cacheWrite', 'totalTokens']) if (!Number.isSafeInteger(usage?.[field]) || Number(usage[field]) < 0) throw new Error('Invalid native assistant usage');
    for (const field of ['input', 'output', 'cacheRead', 'cacheWrite', 'total']) if (typeof cost?.[field] !== 'number' || !Number.isFinite(cost[field]) || Number(cost[field]) < 0) throw new Error('Invalid native assistant cost');
  }
}
