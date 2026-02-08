/**
 * Open WebUI adapter for hstry
 *
 * Parses Open WebUI SQLite database (webui.db)
 * Location: data/webui.db (in Docker: /app/backend/data/webui.db)
 *
 * Requires: better-sqlite3 (npm install better-sqlite3)
 */

import { readdir, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  Conversation,
  Message,
  ParseOptions,
} from '../types/index.ts';
import { runAdapter, textOnlyParts } from '../types/index.ts';

// Dynamic import for better-sqlite3 (optional dependency)
let Database: typeof import('better-sqlite3') | null = null;
try {
  Database = (await import('better-sqlite3')).default;
} catch {
  // SQLite not available - adapter will return empty results
}

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
  history?: { messages?: Record<string, OpenWebUIMessage> };
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
    if (!Database) return null;

    const dbPath = await findDatabase(path);
    if (!dbPath) return null;

    try {
      const db = new Database(dbPath, { readonly: true });
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
    if (!Database) return [];

    const dbPath = await findDatabase(path);
    if (!dbPath) return [];

    const conversations: Conversation[] = [];

    try {
      const db = new Database(dbPath, { readonly: true });

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

  // Try history.messages (tree structure)
  if (chatData.history?.messages) {
    const msgMap = chatData.history.messages;
    // Find root messages and traverse
    const allMsgs = Object.values(msgMap);
    
    // Sort by timestamp if available, otherwise by tree order
    const sorted = allMsgs.sort((a, b) => {
      if (a.timestamp && b.timestamp) return a.timestamp - b.timestamp;
      return 0;
    });

    for (const msg of sorted) {
      const parsed = parseMessage(msg);
      if (parsed) messages.push(parsed);
    }
  }

  return messages;
}

function parseMessage(msg: OpenWebUIMessage): Message | null {
  if (!msg.role || !msg.content) return null;

  return {
    role: mapRole(msg.role),
    content: msg.content,
    parts: textOnlyParts(msg.content),
    createdAt: msg.timestamp ? msg.timestamp * 1000 : undefined,
    model: msg.model,
    metadata: {
      id: msg.id,
      parentId: msg.parentId,
      done: msg.done,
    },
  };
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
