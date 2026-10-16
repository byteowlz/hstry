/**
 * Open WebUI adapter for hstry
 *
 * Parses Open WebUI SQLite database (webui.db)
 * Location: data/webui.db (in Docker: /app/backend/data/webui.db)
 *
 * Uses bun:sqlite under Bun, or better-sqlite3 under Node.
 */

import { readdir, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  CanonPart,
  Conversation,
  Message,
  ParseOptions,
} from '../types/index.ts';
import {
  runAdapter,
  textPart,
  thinkingPart,
  toolCallPart,
  toolResultPart,
} from '../types/index.ts';

// Dynamic SQLite support: bun:sqlite (Bun) or better-sqlite3 (Node). better-sqlite3 is not
// loaded under Bun because its native addon crashes the runtime.
interface SqliteDb {
  prepare(sql: string): { all(...params: unknown[]): unknown[] };
  close(): void;
}
let openDb: ((path: string) => SqliteDb) | null = null;
try {
  if (typeof Bun !== 'undefined') {
    // @ts-ignore - bun:sqlite is Bun-only
    const { Database: BunDb } = await import('bun:sqlite');
    openDb = (path: string) => new BunDb(path, { readonly: true }) as unknown as SqliteDb;
  } else {
    const BetterSqlite = (await import('better-sqlite3')).default;
    openDb = (path: string) => new BetterSqlite(path, { readonly: true }) as unknown as SqliteDb;
  }
} catch {
  // SQLite not available - adapter will return empty results
}

declare const Bun: unknown;

const DEFAULT_PATHS = [
  // Docker default
  '/app/backend/data',
  // Common local paths
  join(homedir(), '.open-webui', 'data'),
  join(homedir(), 'open-webui', 'data'),
  join(homedir(), '.config', 'open-webui', 'data'),
  // Development
  './data',
  './backend/data',
];

interface ChatRow {
  id: string;
  user_id: string;
  title: string;
  chat: string; // JSON string
  created_at: number;
  updated_at: number;
  share_id?: string;
  archived?: number;
  pinned?: number;
  meta?: string; // JSON string
}

interface ChatData {
  messages?: OpenWebUIMessage[];
  history?: { currentId?: string; messages?: Record<string, OpenWebUIMessage> };
  models?: string[];
}

interface OpenWebUIMessage {
  id?: string;
  role?: string;
  content?: string;
  timestamp?: number;
  model?: string;
  parentId?: string;
  childrenIds?: string[];
  done?: boolean;
  /** Assistant structured output (Responses API): reasoning, function_call, output, ... */
  output?: OpenWebUIOutputItem[];
  /** Legacy tool calls stored on the message itself. */
  tool_calls?: OpenWebUIToolCall[];
}

interface OpenWebUIOutputItem {
  type?: string;
  id?: string;
  call_id?: string;
  name?: string;
  status?: string;
  arguments?: unknown;
  content?: { type?: string; text?: unknown }[];
  summary?: { type?: string; text?: unknown }[];
  output?: { type?: string; text?: unknown }[];
  code?: string;
  lang?: string;
  queries?: unknown[];
  action?: Record<string, unknown>;
  error?: unknown;
}

interface OpenWebUIToolCall {
  id?: string;
  name?: string;
  arguments?: unknown;
  function?: { name?: string; arguments?: unknown };
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'openwebui',
      displayName: 'Open WebUI',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!openDb) return null;

    const dbPath = await findDatabase(path);
    if (!dbPath) return null;

    try {
      const db = openDb(dbPath);
      const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all() as { name: string }[];
      db.close();

      const hasChat = tables.some(t => t.name === 'chat');
      const hasAuth = tables.some(t => t.name === 'auth');

      if (hasChat && hasAuth) return 0.95;
      if (hasChat) return 0.7;
    } catch {
      return null;
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    if (!openDb) return [];

    const dbPath = await findDatabase(path);
    if (!dbPath) return [];

    const conversations: Conversation[] = [];

    try {
      const db = openDb(dbPath);

      let query = 'SELECT * FROM chat WHERE 1=1';
      const params: unknown[] = [];

      if (opts?.since) {
        query += ' AND (created_at >= ? OR updated_at >= ?)';
        // Open WebUI stores timestamps in seconds
        const sinceSec = Math.floor(opts.since / 1000);
        params.push(sinceSec, sinceSec);
      }

      query += ' ORDER BY updated_at DESC';

      if (opts?.limit) {
        query += ' LIMIT ?';
        params.push(opts.limit);
      }

      const rows = db.prepare(query).all(...params) as ChatRow[];

      for (const row of rows) {
        const conv = parseChat(row);
        if (conv && conv.messages.length > 0) {
          conversations.push(conv);
        }
      }

      db.close();
    } catch (err) {
      console.error('Error reading Open WebUI database:', err);
    }

    return conversations;
  },

  async export(conversations, opts) {
    if (opts.format === 'markdown') {
      return {
        format: 'markdown',
        content: conversationsToMarkdown(conversations),
        mimeType: 'text/markdown',
      };
    }

    if (opts.format === 'json') {
      return {
        format: 'json',
        content: JSON.stringify(conversations, null, opts.pretty ? 2 : 0),
        mimeType: 'application/json',
      };
    }

    if (opts.format !== 'openwebui') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    // Export as JSON (Open WebUI format)
    const data = conversations.map(conv => buildOpenWebUIFormat(conv));
    return {
      format: 'openwebui',
      content: JSON.stringify(data, null, opts.pretty ? 2 : 0),
      mimeType: 'application/json',
    };
  },
};

function parseChat(row: ChatRow): Conversation | null {
  let chatData: ChatData;
  try {
    chatData = JSON.parse(row.chat);
  } catch {
    return null;
  }

  const messages = extractMessages(chatData);
  if (messages.length === 0) return null;

  // Timestamps are in seconds
  const createdAt = row.created_at * 1000;
  const updatedAt = row.updated_at ? row.updated_at * 1000 : undefined;

  let meta: Record<string, unknown> = {};
  if (row.meta) {
    try {
      meta = JSON.parse(row.meta);
    } catch { /* ignore */ }
  }

  const model = chatData.models?.[0] ?? deriveModel(messages);

  return {
    externalId: row.id,
    title: row.title || undefined,
    createdAt,
    updatedAt,
    model,
    messages,
    metadata: {
      userId: row.user_id,
      shareId: row.share_id,
      archived: row.archived === 1,
      pinned: row.pinned === 1,
      ...meta,
    },
  };
}

function extractMessages(chatData: ChatData): Message[] {
  const messages: Message[] = [];

  // Try direct messages array first
  if (chatData.messages && Array.isArray(chatData.messages)) {
    for (const msg of chatData.messages) {
      const parsed = parseMessage(msg);
      if (parsed) messages.push(parsed);
    }
    return messages;
  }

  // history.messages is a tree; history.currentId names the leaf of the branch
  // the user actually sees. Walking its parentId chain avoids importing
  // regenerated/edited siblings that sorting by timestamp would interleave.
  if (chatData.history?.messages) {
    const msgMap = chatData.history.messages;
    const active = walkActiveBranch(msgMap, chatData.history.currentId);
    const entries = active.length ? active : sortByTimestamp(Object.values(msgMap));
    for (const msg of entries) {
      const parsed = parseMessage(msg);
      if (parsed) messages.push(parsed);
    }
  }

  return messages;
}

/** Walk currentId -> parents back to the root and return root-first order. */
function walkActiveBranch(
  msgMap: Record<string, OpenWebUIMessage>,
  currentId: string | undefined
): OpenWebUIMessage[] {
  if (!currentId || !msgMap[currentId]) return [];
  const chain: OpenWebUIMessage[] = [];
  const seen = new Set<string>();
  let id: string | undefined = currentId;
  while (id && msgMap[id] && !seen.has(id)) {
    seen.add(id);
    chain.push(msgMap[id]);
    id = msgMap[id].parentId;
  }
  return chain.reverse();
}

function sortByTimestamp(messages: OpenWebUIMessage[]): OpenWebUIMessage[] {
  return [...messages].sort((a, b) => {
    if (a.timestamp && b.timestamp) return a.timestamp - b.timestamp;
    return 0;
  });
}

function parseMessage(msg: OpenWebUIMessage): Message | null {
  const parts: CanonPart[] = [];
  const chunks: string[] = [];
  let outputText = '';

  for (const item of Array.isArray(msg.output) ? msg.output : []) {
    if (!item || typeof item !== 'object') continue;
    if (item.type === 'reasoning') {
      const text = outputPartsText(item.summary) || outputPartsText(item.content);
      if (text) {
        parts.push(thinkingPart(text));
        chunks.push(text);
      }
    } else if (item.type === 'function_call') {
      const callId = String(item.call_id ?? item.id ?? item.name ?? 'tool');
      parts.push(toolCallPart(callId, String(item.name ?? 'tool'), parseArguments(item.arguments)));
    } else if (item.type === 'function_call_output') {
      const text = outputPartsText(item.output);
      parts.push(toolResultPart(String(item.call_id ?? item.id ?? 'unknown'), text, { isError: item.status === 'failed' }));
      if (text) chunks.push(text);
    } else if (item.type === 'open_webui:code_interpreter') {
      const callId = String(item.call_id ?? item.id ?? 'code_interpreter');
      parts.push(toolCallPart(callId, 'code_interpreter', { code: item.code, lang: item.lang }));
      const text = outputPartsText(item.output);
      if (text) {
        parts.push(toolResultPart(callId, text, { name: 'code_interpreter' }));
        chunks.push(text);
      }
    } else if (item.type === 'web_search_call' || item.type === 'file_search_call' || item.type === 'computer_call') {
      const callId = String(item.call_id ?? item.id ?? item.type);
      parts.push(toolCallPart(callId, item.type.replace(/_call$/, ''), item.action ?? item.queries ?? undefined));
    } else if (item.type === 'message' || item.type === 'error' || item.content) {
      const text = outputPartsText(item.content) || outputPartsText(item.output);
      if (text) {
        if (!outputText) outputText = text;
        parts.push(textPart(text));
        chunks.push(text);
      }
    } else if (typeof (item as { text?: unknown }).text === 'string') {
      const text = (item as { text: string }).text.trim();
      if (text) {
        parts.push(textPart(text));
        chunks.push(text);
      }
    }
  }

  for (const call of Array.isArray(msg.tool_calls) ? msg.tool_calls : []) {
    if (!call || typeof call !== 'object') continue;
    const name = call.function?.name ?? call.name ?? 'tool';
    const callId = String(call.id ?? name);
    parts.push(toolCallPart(callId, String(name), parseArguments(call.function?.arguments ?? call.arguments)));
  }

  const content = typeof msg.content === 'string' ? msg.content.trim() : '';
  if (content && content !== outputText) {
    parts.unshift(textPart(content));
    chunks.unshift(content);
  }

  if (!parts.length) return null;

  return {
    role: mapRole(msg.role ?? 'assistant'),
    content: chunks.join('\n').trim(),
    parts,
    createdAt: msg.timestamp ? msg.timestamp * 1000 : undefined,
    model: msg.model,
    metadata: {
      id: msg.id,
      parentId: msg.parentId,
      done: msg.done,
    },
  };
}

function outputPartsText(parts?: { type?: string; text?: unknown }[]): string {
  if (!Array.isArray(parts)) return '';
  return parts
    .filter(part => part?.type !== 'input_image')
    .map(part => (part?.text === undefined || part?.text === null ? '' : typeof part.text === 'string' ? part.text : String(part.text)))
    .join('')
    .trim();
}

function parseArguments(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  let parsed: unknown = value.trim();
  while (typeof parsed === 'string') {
    try {
      parsed = JSON.parse(parsed);
    } catch {
      break;
    }
  }
  return parsed;
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
    case 'human':
      return 'user';
    case 'assistant':
    case 'ai':
    case 'bot':
      return 'assistant';
    case 'system':
      return 'system';
    case 'tool':
    case 'function':
      return 'tool';
    default:
      return 'assistant';
  }
}

function deriveModel(messages: Message[]): string | undefined {
  for (const msg of messages) {
    if (msg.model) return msg.model;
  }
  return undefined;
}

function conversationsToMarkdown(conversations: Conversation[]): string {
  const blocks: string[] = [];
  for (const conv of conversations) {
    const title = conv.title ?? 'Conversation';
    blocks.push(`# ${title}`);
    blocks.push('');
    blocks.push(`- Created: ${new Date(conv.createdAt).toISOString()}`);
    if (conv.updatedAt) {
      blocks.push(`- Updated: ${new Date(conv.updatedAt).toISOString()}`);
    }
    if (conv.model) {
      blocks.push(`- Model: ${conv.model}`);
    }
    blocks.push('');

    for (const msg of conv.messages) {
      blocks.push(`## ${msg.role}`);
      if (msg.createdAt) {
        blocks.push(`_at ${new Date(msg.createdAt).toISOString()}_`);
      }
      blocks.push('');
      blocks.push(msg.content || '');
      blocks.push('');
    }
  }
  return blocks.join('\n').trim() + '\n';
}

function buildOpenWebUIFormat(conv: Conversation): Record<string, unknown> {
  return {
    id: conv.externalId,
    title: conv.title,
    created_at: Math.floor(conv.createdAt / 1000),
    updated_at: conv.updatedAt ? Math.floor(conv.updatedAt / 1000) : undefined,
    models: conv.model ? [conv.model] : [],
    messages: conv.messages.map(msg => ({
      role: msg.role,
      content: msg.content,
      timestamp: msg.createdAt ? Math.floor(msg.createdAt / 1000) : undefined,
      model: msg.model,
    })),
  };
}

async function findDatabase(path: string): Promise<string | null> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return null;

  // Direct file
  if (stats.isFile() && path.endsWith('.db')) {
    return path;
  }

  if (!stats.isDirectory()) return null;

  // Look for webui.db in directory
  const dbPath = join(path, 'webui.db');
  const dbStats = await stat(dbPath).catch(() => null);
  if (dbStats?.isFile()) {
    return dbPath;
  }

  // Check backend/data subdirectory
  const backendDbPath = join(path, 'backend', 'data', 'webui.db');
  const backendStats = await stat(backendDbPath).catch(() => null);
  if (backendStats?.isFile()) {
    return backendDbPath;
  }

  return null;
}

runAdapter(adapter);
