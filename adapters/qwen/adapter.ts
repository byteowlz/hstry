/* Qwen Code adapter.
 * Legacy: ~/.qwen/tmp/<projectHash>/chats/session-*.json (Gemini CLI style).
 * Current (QwenLM/qwen-code @main, v0.25.x, 2026-10,
 * packages/core/src/services/chatRecordingService.ts + config/storage.ts):
 * ~/.qwen/projects/<sanitized-cwd>/chats/<sessionId>.jsonl, one ChatRecord per
 * line ({uuid,parentUuid,sessionId,timestamp,type:user|assistant|tool_result|system,
 * message:Content{role,parts[text|thought|functionCall|functionResponse]},model,...}).
 */
import { readdir, readFile, stat } from 'fs/promises';
import { basename, extname, join } from 'path';
import { homedir } from 'os';
import type { Adapter, AdapterInfo, CanonPart, Conversation, Message, ParseOptions, ToolCall } from '../types/index.ts';
import {
  isUnderAnyCanonicalRoot,
  runAdapter,
  textPart,
  textOnlyParts,
  thinkingPart,
  toolCallPart,
  toolResultPart,
} from '../types/index.ts';

const QWEN_DIR = join(homedir(), '.qwen');
const LEGACY_ROOT = join(QWEN_DIR, 'tmp');
const PROJECTS_ROOT = join(QWEN_DIR, 'projects');
const ROOTS = [LEGACY_ROOT, PROJECTS_ROOT];
type J = Record<string, any>;

const adapter: Adapter = {
  info(): AdapterInfo {
    return { name: 'qwen', displayName: 'Qwen Code', version: '1.1.0', defaultPaths: ROOTS };
  },
  async detect(path) {
    if (!isUnderAnyCanonicalRoot(path, ROOTS)) return null;
    return (await files(path, true)).length ? 0.9 : null;
  },
  async parse(path, opts) {
    const out: Conversation[] = [];
    for (const f of await files(path, false)) {
      const c = f.endsWith('.jsonl') ? await parseJsonl(f, opts) : await parseLegacy(f, opts);
      if (c) out.push(c);
    }
    out.sort((a, b) => b.createdAt - a.createdAt);
    return opts?.limit ? out.slice(0, opts.limit) : out;
  },
};

async function files(path: string, shallow: boolean): Promise<string[]> {
  const out: string[] = [];
  const isSession = (name: string) =>
    (name.startsWith('session-') && name.endsWith('.json')) || (name.endsWith('.jsonl') && !name.startsWith('.'));
  async function walk(d: string, n: number) {
    try {
      const s = await stat(d);
      if (s.isFile()) {
        if (isSession(basename(d))) out.push(d);
        return;
      }
    } catch {
      return;
    }
    if (n > 3) return;
    try {
      const entries = await readdir(d, { withFileTypes: true });
      for (const e of entries) {
        if (e.isDirectory()) await walk(join(d, e.name), n + 1);
        else if (isSession(e.name)) out.push(join(d, e.name));
        if (shallow && out.length) return;
      }
    } catch {
      // unreadable directory
    }
  }
  await walk(path, 0);
  return out;
}

/** Current `.jsonl` tree records, one ChatRecord per line. */
async function parseJsonl(file: string, opts?: ParseOptions): Promise<Conversation | null> {
  const lines = (await readFile(file, 'utf8')).split(/\r?\n/);
  const records: J[] = [];
  for (const line of lines) {
    if (!line.trim()) continue;
    try {
      const value = JSON.parse(line);
      if (value && typeof value === 'object') records.push(value);
    } catch {
      // A truncated tail is normal for a live session; skip the line.
    }
  }
  if (!records.length) return null;

  const messages: Message[] = [];
  let title: string | undefined;
  let model: string | undefined;
  let parentExternalId: string | undefined;
  let cwd: string | undefined;
  let version: string | undefined;

  for (const record of records) {
    cwd ??= typeof record.cwd === 'string' ? record.cwd : undefined;
    version ??= typeof record.version === 'string' ? record.version : undefined;
    if (typeof record.model === 'string') model = record.model;

    if (record.type === 'system') {
      if (record.subtype === 'custom_title') title = record.systemPayload?.customTitle ?? title;
      else if (record.subtype === 'parent_session') parentExternalId = record.systemPayload?.parentSessionId ?? parentExternalId;
      else if (record.subtype === 'session_model' && typeof record.systemPayload?.modelId === 'string') model = record.systemPayload.modelId;
      continue;
    }

    const role = record.type === 'user' ? 'user' : record.type === 'assistant' ? 'assistant' : record.type === 'tool_result' ? 'tool' : null;
    if (!role) continue;

    const built = buildParts(record, role);
    if (!built.content && !built.parts?.length) continue;
    messages.push({
      role,
      content: built.content,
      parts: built.parts,
      createdAt: time(record.timestamp),
      model: typeof record.model === 'string' ? record.model : undefined,
      toolCalls: built.toolCalls,
      metadata: {
        uuid: record.uuid,
        ...(record.subtype ? { subtype: record.subtype } : {}),
        ...(record.isSidechain ? { isSidechain: true } : {}),
        ...(record.agentId ? { agentId: record.agentId } : {}),
      },
    });
  }

  if (!messages.length) return null;
  const createdAt = messages[0].createdAt ?? time(records[0]?.timestamp) ?? Date.now();
  const updatedAt = messages.at(-1)?.createdAt ?? createdAt;
  if (opts?.since && updatedAt < opts.since) return null;
  return {
    externalId: String(records.find(r => r.sessionId)?.sessionId ?? basename(file, extname(file))),
    title: title ?? messages.find(m => m.role === 'user')?.content.slice(0, 100) ?? '(untitled)',
    createdAt,
    updatedAt,
    model,
    workspace: cwd,
    messages,
    parentExternalId,
    ...(parentExternalId ? { forkType: 'thread' as const } : {}),
    metadata: { source: 'qwen-code', format: 'jsonl', version },
  };
}

/** Gemini `Content.parts` -> canonical parts + searchable text. */
function buildParts(record: J, role: Message['role']): { content: string; parts?: CanonPart[]; toolCalls?: ToolCall[] } {
  const parts: CanonPart[] = [];
  const textChunks: string[] = [];
  const toolCalls: ToolCall[] = [];
  const raw = Array.isArray(record.message?.parts) ? record.message.parts : [];

  for (const part of raw) {
    if (!part || typeof part !== 'object') continue;
    if (typeof part.text === 'string' && part.text.trim()) {
      const text = part.text.trim();
      textChunks.push(text);
      parts.push(part.thought === true ? thinkingPart(text) : textPart(text));
    } else if (part.functionCall) {
      const call = part.functionCall;
      const callId = String(call.id ?? call.name ?? 'unknown');
      parts.push(toolCallPart(callId, String(call.name ?? 'tool'), call.args));
      toolCalls.push({ toolName: String(call.name ?? 'tool'), input: call.args, status: 'pending' });
    } else if (part.functionResponse) {
      const call = part.functionResponse;
      const callId = String(call.id ?? call.name ?? 'unknown');
      const output = responseText(call.response);
      parts.push(toolResultPart(callId, output, { name: typeof call.name === 'string' ? call.name : undefined }));
      textChunks.push(output);
    }
  }

  // The tool_result record carries the outcome: attach status/error to the call.
  const result = record.toolCallResult;
  if (role === 'tool' && result) {
    const status = result.status === 'error' || result.error ? 'error' : 'success';
    const last = parts.findLast(p => p.type === 'tool_result') as { isError?: boolean } | undefined;
    if (last && status === 'error') last.isError = true;
    if (!parts.some(p => p.type === 'tool_result')) {
      const output = responseText(result) || String(result.error ?? '');
      parts.push(toolResultPart(String(result.callId ?? 'unknown'), output, { isError: status === 'error' }));
      textChunks.push(output);
    }
  }

  const content = textChunks.join('\n').trim();
  return { content, parts: parts.length ? parts : textOnlyParts(content), toolCalls: toolCalls.length ? toolCalls : undefined };
}

function responseText(response: any): string {
  if (response == null) return '';
  if (typeof response === 'string') return response;
  if (typeof response.output === 'string') return response.output;
  if (typeof response.error === 'string') return response.error;
  try {
    return JSON.stringify(response);
  } catch {
    return '';
  }
}

/** Legacy `session-*.json` documents. */
async function parseLegacy(file: string, opts?: ParseOptions): Promise<Conversation | null> {
  let j: J;
  try {
    j = JSON.parse(await readFile(file, 'utf8'));
  } catch {
    return null;
  }
  const messages: Message[] = [];
  for (const m of j.messages ?? []) {
    const role = m.type === 'user' ? 'user' : m.type === 'qwen' || m.type === 'assistant' ? 'assistant' : null;
    if (!role) continue;
    const parts: CanonPart[] = [];
    const chunks: string[] = [];
    for (const t of m.thoughts ?? []) {
      const text = [t.subject, t.description].filter(Boolean).join(': ').trim();
      if (text) {
        parts.push(thinkingPart(text));
        chunks.push(text);
      }
    }
    const toolCalls: ToolCall[] = [];
    for (const call of m.toolCalls ?? []) {
      const callId = String(call.id ?? call.name ?? 'unknown');
      parts.push(toolCallPart(callId, String(call.name ?? 'tool'), call.args));
      const response = call.result?.[0]?.functionResponse?.response;
      const output = responseText(response);
      if (response !== undefined) parts.push(toolResultPart(callId, output, { isError: call.status === 'error' }));
      toolCalls.push({ toolName: String(call.name ?? 'tool'), input: call.args, output, status: call.status === 'error' ? 'error' : 'success' });
    }
    const content = flat(m.content);
    if (content) chunks.push(content);
    parts.push(...(textOnlyParts(content) ?? []));
    const allContent = chunks.join('\n').trim();
    if (!allContent && !parts.length) continue;
    messages.push({ role, content: allContent, parts, createdAt: time(m.timestamp), model: typeof m.model === 'string' ? m.model : undefined, toolCalls: toolCalls.length ? toolCalls : undefined });
  }
  if (!messages.length) return null;
  const createdAt = time(j.startTime) ?? messages[0].createdAt ?? Date.now();
  const updatedAt = time(j.lastUpdated) ?? messages.at(-1)?.createdAt ?? createdAt;
  if (opts?.since && updatedAt < opts.since) return null;
  return {
    externalId: String(j.sessionId ?? basename(file, '.json')),
    title: messages.find(m => m.role === 'user')?.content.slice(0, 100) ?? '(untitled)',
    createdAt,
    updatedAt,
    model: messages.findLast(m => m.model)?.model,
    workspace: typeof j.cwd === 'string' ? j.cwd : undefined,
    messages,
    metadata: { source: 'qwen-code', format: 'legacy', projectHash: j.projectHash },
  };
}

function flat(v: any): string {
  if (typeof v === 'string') return v;
  if (!Array.isArray(v)) return '';
  return v.map(x => (typeof x === 'string' ? x : (x?.text ?? x?.content ?? ''))).join('\n').trim();
}

function time(v: any): number | undefined {
  if (typeof v === 'number') return Math.floor(v < 1e11 ? v * 1000 : v);
  if (typeof v === 'string') {
    const n = Date.parse(v);
    return Number.isFinite(n) ? Math.floor(n) : undefined;
  }
  return undefined;
}

runAdapter(adapter);
