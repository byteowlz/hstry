/* Vercel FX adapter — ~/.fx/sessions/<id>/{events.jsonl,session.json}. */
import { readdir, readFile, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import type { Adapter, AdapterInfo, Conversation, Message, ParseOptions, ToolCall } from '../types/index.ts';
import { isUnderCanonicalRoot, runAdapter, textOnlyParts, toolCallPart, toolResultPart } from '../types/index.ts';

const ROOT = join(homedir(), '.fx', 'sessions');
type Json = Record<string, any>;

const adapter: Adapter = {
  info(): AdapterInfo {
    return { name: 'fx', displayName: 'Vercel FX', version: '1.0.0', defaultPaths: [ROOT] };
  },
  async detect(path: string): Promise<number | null> {
    if (!isUnderCanonicalRoot(path, ROOT)) return null;
    return (await findSessionDirs(path, true)).length ? 0.95 : null;
  },
  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const out: Conversation[] = [];
    for (const dir of await findSessionDirs(path, false)) {
      const conv = await parseSession(dir, opts);
      if (conv) out.push(conv);
    }
    out.sort((a, b) => b.createdAt - a.createdAt);
    return opts?.limit ? out.slice(0, opts.limit) : out;
  },
};

async function findSessionDirs(path: string, shallow: boolean): Promise<string[]> {
  try {
    if ((await stat(join(path, 'session.json'))).isFile()) return [path];
  } catch { /* root, not a session */ }
  const out: string[] = [];
  try {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      if (!entry.isDirectory() || entry.name === 'latest') continue;
      const dir = join(path, entry.name);
      try {
        if ((await stat(join(dir, 'session.json'))).isFile()) out.push(dir);
      } catch { /* incomplete session */ }
      if (shallow && out.length) break;
    }
  } catch { /* absent root */ }
  return out;
}

async function parseSession(dir: string, opts?: ParseOptions): Promise<Conversation | null> {
  let manifest: Json = {};
  try { manifest = JSON.parse(await readFile(join(dir, 'session.json'), 'utf8')); } catch { /* events remain authoritative */ }
  let raw = '';
  try { raw = await readFile(join(dir, 'events.jsonl'), 'utf8'); } catch { return null; }

  let id = stringValue(manifest.id) ?? basename(dir);
  let createdAt = integer(manifest.created_at_ms) ?? 0;
  let updatedAt = integer(manifest.updated_at_ms) ?? createdAt;
  let workspace = stringValue(manifest.workspace_root);
  let model = stringValue(manifest.preferences?.model);
  let provider = stringValue(manifest.preferences?.provider);
  let tokensIn = integer(manifest.total_input_tokens) ?? 0;
  let tokensOut = integer(manifest.total_output_tokens) ?? 0;
  const messages: Message[] = [];

  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    let event: Json;
    try { event = JSON.parse(line); } catch { continue; }
    const ts = integer(event.timestamp_ms);
    if (ts !== undefined) updatedAt = Math.max(updatedAt, ts);
    const payload = event.payload ?? event.event?.[event.kind];
    if (!payload || typeof payload !== 'object') continue;
    if (event.kind === 'session_started') {
      id = stringValue(payload.id) ?? id;
      createdAt = integer(payload.created_at_ms) ?? createdAt;
      workspace = stringValue(payload.workspace_root) ?? workspace;
      model = stringValue(payload.preferences?.model) ?? model;
      provider = stringValue(payload.preferences?.provider) ?? provider;
    } else if (event.kind === 'preferences_changed') {
      model = stringValue(payload.model) ?? model;
      provider = stringValue(payload.provider) ?? provider;
    } else if (event.kind === 'workspace_rebound') {
      workspace = stringValue(payload.workspace_root) ?? workspace;
    } else if (event.kind === 'history_turn_committed') {
      tokensIn = integer(payload.total_input_tokens) ?? tokensIn;
      tokensOut = integer(payload.total_output_tokens) ?? tokensOut;
      appendTurn(messages, payload.turn, ts ?? updatedAt, opts);
    }
  }

  if (!createdAt) createdAt = updatedAt || Date.now();
  if (opts?.since && updatedAt < opts.since) return null;
  if (!messages.length) return null;
  const firstUser = messages.find(message => message.role === 'user')?.content ?? '(untitled)';
  return {
    externalId: id,
    title: firstUser.replace(/\s+/g, ' ').trim().slice(0, 100),
    createdAt,
    updatedAt,
    model,
    provider,
    workspace,
    tokensIn,
    tokensOut,
    messages,
    metadata: { storageFormat: manifest.storage_format ?? 'event_log_v1', source: 'fx' },
  };
}

function appendTurn(messages: Message[], turn: Json, timestamp: number, opts?: ParseOptions): void {
  if (!turn || typeof turn !== 'object') return;
  const kind = Object.keys(turn)[0];
  const value = turn[kind];
  if (!value || typeof value !== 'object') return;
  if (kind === 'compacted_summary') {
    const text = stringValue(value.summary);
    if (text) messages.push({ role: 'system', content: text, createdAt: timestamp, parts: textOnlyParts(text), metadata: { compacted: true } });
    return;
  }
  const user = stringValue(value.user?.text);
  if (user) messages.push({ role: 'user', content: user, createdAt: timestamp, parts: textOnlyParts(user) });

  const toolCalls: ToolCall[] = [];
  const assistantParts: any[] = [];
  for (const step of value.execution?.tool_steps ?? []) {
    const stepText = stringValue(step.assistant);
    if (stepText) assistantParts.push(...(textOnlyParts(stepText) ?? []));
    for (const call of step.tool_calls ?? []) {
      const callId = stringValue(call.id) ?? `call-${toolCalls.length + 1}`;
      const name = stringValue(call.name) ?? 'tool';
      const input = parseMaybeJson(call.arguments_json);
      toolCalls.push({ toolName: name, input, status: 'pending' });
      assistantParts.push(toolCallPart(callId, name, input));
    }
    if (opts?.includeTools !== false) {
      for (const result of step.tool_results ?? []) {
        const callId = stringValue(result.tool_call_id) ?? 'unknown';
        const output = stringValue(result.output) ?? '';
        messages.push({
          role: 'tool', content: output, createdAt: integer(result.created_at_ms) ?? timestamp,
          parts: [toolResultPart(callId, output, { name: stringValue(result.tool_name), isError: result.status === 'failure' })],
          metadata: { status: result.status },
        });
      }
    }
  }
  const assistant = stringValue(value.assistant) ?? '';
  if (assistant) assistantParts.push(...(textOnlyParts(assistant) ?? []));
  if (assistant || toolCalls.length) {
    messages.push({ role: 'assistant', content: assistant, createdAt: timestamp, parts: assistantParts.length ? assistantParts : undefined, toolCalls: toolCalls.length ? toolCalls : undefined, metadata: kind === 'interrupted' ? { interrupted: true } : undefined });
  }
}

function stringValue(value: unknown): string | undefined { return typeof value === 'string' ? value : undefined; }
function integer(value: unknown): number | undefined { return typeof value === 'number' && Number.isFinite(value) ? Math.floor(value) : undefined; }
function parseMaybeJson(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  try { return JSON.parse(value); } catch { return value; }
}

runAdapter(adapter);
