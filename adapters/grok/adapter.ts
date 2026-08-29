/* xAI Grok Build adapter — ~/.grok/sessions/<cwd>/<id>/. */
import { readdir, readFile, stat } from 'fs/promises';
import { basename, dirname, join } from 'path';
import { homedir } from 'os';
import type { Adapter, AdapterInfo, CanonPart, Conversation, Message, ParseOptions, ToolCall } from '../types/index.ts';
import { isUnderCanonicalRoot, runAdapter, textOnlyParts, thinkingPart, toolCallPart, toolResultPart } from '../types/index.ts';

const ROOT = join(homedir(), '.grok', 'sessions');
type Json = Record<string, any>;

const adapter: Adapter = {
  info(): AdapterInfo { return { name: 'grok', displayName: 'Grok Build', version: '1.0.0', defaultPaths: [ROOT] }; },
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
  const out: string[] = [];
  async function walk(dir: string, depth: number): Promise<void> {
    try {
      if ((await stat(join(dir, 'summary.json'))).isFile()) { out.push(dir); return; }
    } catch { /* continue */ }
    if (depth >= 2 || (shallow && out.length)) return;
    try {
      for (const entry of await readdir(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) await walk(join(dir, entry.name), depth + 1);
        if (shallow && out.length) return;
      }
    } catch { /* missing root */ }
  }
  await walk(path, 0);
  return out;
}

async function parseSession(dir: string, opts?: ParseOptions): Promise<Conversation | null> {
  let summary: Json;
  try { summary = JSON.parse(await readFile(join(dir, 'summary.json'), 'utf8')); } catch { return null; }
  const info = summary.info ?? {};
  const id = stringValue(info.id) ?? basename(dir);
  const createdAt = parseTime(summary.created_at) ?? 0;
  const updatedAt = parseTime(summary.last_active_at) ?? parseTime(summary.updated_at) ?? createdAt;
  if (opts?.since && updatedAt < opts.since) return null;

  let raw = '';
  try { raw = await readFile(join(dir, 'chat_history.jsonl'), 'utf8'); } catch { return null; }
  const messages: Message[] = [];
  if ((numberValue(summary.chat_format_version) ?? 0) >= 1) parseV1(raw, messages, opts);
  else parseLegacy(raw, messages, opts);
  if (!messages.length) return null;

  const title = stringValue(summary.generated_title)
    ?? stringValue(summary.session_summary)
    ?? messages.find(message => message.role === 'user')?.content
    ?? '(untitled)';
  const parentExternalId = stringValue(summary.parent_session_id);
  return {
    externalId: id,
    title: title.replace(/\s+/g, ' ').trim().slice(0, 120),
    createdAt: createdAt || updatedAt || Date.now(),
    updatedAt: updatedAt || createdAt,
    model: stringValue(summary.current_model_id),
    provider: 'xai',
    workspace: stringValue(info.cwd) ?? decodeCwd(dirname(dir)),
    messages,
    parentExternalId,
    forkType: parentExternalId ? 'fork' : undefined,
    metadata: { source: 'grok-build', chatFormatVersion: numberValue(summary.chat_format_version) ?? 0, agentName: summary.agent_name, sessionKind: summary.session_kind },
  };
}

function parseV1(raw: string, messages: Message[], opts?: ParseOptions): void {
  const pendingReasoning: string[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    let item: Json;
    try { item = JSON.parse(line); } catch { continue; }
    if (item.type === 'system') {
      const content = stringValue(item.content);
      if (content) messages.push({ role: 'system', content, parts: textOnlyParts(content) });
    } else if (item.type === 'user') {
      if (item.synthetic_reason) continue;
      const content = contentText(item.content);
      if (content) messages.push({ role: 'user', content, parts: textOnlyParts(content), attachments: imageAttachments(item.content, opts) });
    } else if (item.type === 'reasoning') {
      const text = reasoningText(item);
      if (text) pendingReasoning.push(text);
    } else if (item.type === 'assistant') {
      const content = stringValue(item.content) ?? '';
      const calls: ToolCall[] = [];
      const parts: CanonPart[] = pendingReasoning.splice(0).map(text => thinkingPart(text));
      for (const call of item.tool_calls ?? []) {
        const id = stringValue(call.id) ?? `call-${calls.length + 1}`;
        const name = stringValue(call.name) ?? 'tool';
        const input = parseMaybeJson(call.arguments);
        calls.push({ toolName: name, input, status: 'pending' });
        parts.push(toolCallPart(id, name, input));
      }
      parts.push(...(textOnlyParts(content) ?? []));
      if (content || calls.length || parts.length) messages.push({ role: 'assistant', content, model: stringValue(item.model_id), parts, toolCalls: calls.length ? calls : undefined });
    } else if (item.type === 'tool_result' && opts?.includeTools !== false) {
      const output = stringValue(item.content) ?? '';
      const id = stringValue(item.tool_call_id) ?? 'unknown';
      messages.push({ role: 'tool', content: output, parts: [toolResultPart(id, output)], attachments: imageAttachments(item.images, opts) });
    } else if (item.type === 'backend_tool_call' && opts?.includeTools !== false) {
      const kind = item.kind ?? {};
      const name = stringValue(kind.tool_type) ?? 'backend_tool';
      const id = stringValue(kind.id) ?? `backend-${messages.length}`;
      messages.push({ role: 'assistant', content: '', parts: [toolCallPart(id, name, kind)], toolCalls: [{ toolName: name, input: kind, status: 'success' }] });
    }
  }
}

function parseLegacy(raw: string, messages: Message[], opts?: ParseOptions): void {
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    let item: Json;
    try { item = JSON.parse(line); } catch { continue; }
    const role = item.role;
    if (!['system', 'user', 'assistant', 'tool'].includes(role)) continue;
    const content = contentText(item.content);
    const calls: ToolCall[] = [];
    const parts: CanonPart[] = [...(textOnlyParts(content) ?? [])];
    for (const call of item.tool_calls ?? []) {
      const id = stringValue(call.id) ?? stringValue(call.tool_call_id) ?? `call-${calls.length + 1}`;
      const fn = call.function ?? call;
      const name = stringValue(fn.name) ?? 'tool';
      const input = parseMaybeJson(fn.arguments);
      calls.push({ toolName: name, input, status: 'pending' });
      parts.push(toolCallPart(id, name, input));
    }
    if (role === 'tool' && opts?.includeTools === false) continue;
    if (!content && !calls.length) continue;
    messages.push({ role, content, parts: role === 'tool' ? [toolResultPart(stringValue(item.tool_call_id) ?? 'unknown', content, { name: stringValue(item.name) })] : parts, toolCalls: calls.length ? calls : undefined });
  }
}

function contentText(value: unknown): string {
  if (typeof value === 'string') return value;
  if (!Array.isArray(value)) return '';
  return value.map(part => typeof part === 'string' ? part : part?.type === 'text' ? stringValue(part.text) ?? '' : '').filter(Boolean).join('\n');
}
function imageAttachments(value: unknown, opts?: ParseOptions): any[] | undefined {
  if (opts?.includeAttachments === false || !Array.isArray(value)) return undefined;
  const images = value.filter(part => part?.type === 'image' && typeof part.url === 'string').map(part => ({ type: 'image', name: 'image', path: part.url, metadata: { url: part.url } }));
  return images.length ? images : undefined;
}
function reasoningText(item: Json): string {
  if (typeof item.summary === 'string') return item.summary;
  if (Array.isArray(item.summary)) return item.summary.map((v: any) => v?.text ?? '').filter(Boolean).join('\n');
  if (typeof item.content === 'string') return item.content;
  return '';
}
function decodeCwd(cwdDir: string): string | undefined {
  const name = basename(cwdDir);
  try { return decodeURIComponent(name); } catch { return undefined; }
}
function parseTime(value: unknown): number | undefined { if (typeof value !== 'string') return undefined; const n = Date.parse(value); return Number.isFinite(n) ? Math.floor(n) : undefined; }
function stringValue(value: unknown): string | undefined { return typeof value === 'string' ? value : undefined; }
function numberValue(value: unknown): number | undefined { return typeof value === 'number' && Number.isFinite(value) ? value : undefined; }
function parseMaybeJson(value: unknown): unknown { if (typeof value !== 'string') return value; try { return JSON.parse(value); } catch { return value; } }

runAdapter(adapter);
