/**
 * Goose adapter for hstry
 *
 * Parses Goose session data from:
 * - SQLite database: ~/.local/share/goose/sessions/sessions.db (v1.10.0+)
 * - Legacy JSONL files: ~/.local/share/goose/sessions/*.jsonl
 *
 * Requires: better-sqlite3 for database support (npm install better-sqlite3)
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, extname, join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  Conversation,
  Message,
  ParseOptions,
  ToolCall,
} from '../types/index.ts';
import {
  runAdapter,
  textOnlyParts,
  toolCallPart,
  toolResultPart,
} from '../types/index.ts';

// Dynamic import for better-sqlite3 (optional dependency)
let Database: typeof import('better-sqlite3') | null = null;
try {
  Database = (await import('better-sqlite3')).default;
} catch {
  // SQLite not available - will fall back to JSONL only
}

const DEFAULT_PATHS_UNIX = [
  join(homedir(), '.local', 'share', 'goose', 'sessions'),
];

const DEFAULT_PATHS_WINDOWS = [
  join(process.env.APPDATA || '', 'Block', 'goose', 'data', 'sessions'),
];

const DEFAULT_PATHS = process.platform === 'win32' 
  ? DEFAULT_PATHS_WINDOWS 
  : DEFAULT_PATHS_UNIX;

interface SessionRow {
  id: string;
  session_id?: string;
  metadata?: string;
  messages?: string; // Legacy: JSON string of messages
  created_at?: string | number; // Can be ISO string or Unix timestamp
  updated_at?: string | number;
  working_directory?: string;
}

interface GooseMessage {
  role?: string;
  content?: string | GooseContent[];
  created_at?: string | number;
  tool_calls?: GooseToolCall[];
  tool_call_id?: string;
  name?: string;
}

interface GooseContent {
  type?: string;
  text?: string;
  tool_use_id?: string;
  tool_name?: string;
  input?: unknown;
  output?: unknown;
}

interface GooseToolCall {
  id?: string;
  type?: string;
  function?: {
    name?: string;
    arguments?: string;
  };
}

interface JsonlEntry {
  type?: string;
  role?: string;
  content?: string | GooseContent[];
  created?: number;
  timestamp?: string;
  tool_calls?: GooseToolCall[];
  metadata?: Record<string, unknown>;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'goose',
      displayName: 'Goose',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    // Check for SQLite database (if better-sqlite3 is available)
    if (Database) {
      const dbPath = join(path, 'sessions.db');
      const dbStats = await stat(dbPath).catch(() => null);
      if (dbStats?.isFile()) {
        try {
          const db = new Database(dbPath, { readonly: true });
          const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all() as { name: string }[];
          db.close();
          if (tables.some(t => t.name === 'sessions' || t.name === 'session')) {
            return 0.95;
          }
        } catch { /* continue */ }
      }
    }

    // Check for JSONL files (legacy)
    const jsonlFiles = await findJsonlFiles(path, { shallowOnly: true });
    if (jsonlFiles.length > 0) return 0.85;

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const conversations: Conversation[] = [];

    // Try SQLite database first (v1.10.0+)
    const dbPath = join(path, 'sessions.db');
    const dbStats = await stat(dbPath).catch(() => null);
    
    if (dbStats?.isFile()) {
      const dbConvs = await parseDatabase(dbPath, opts);
      conversations.push(...dbConvs);
    }

    // Also check legacy JSONL files
    const jsonlFiles = await findJsonlFiles(path, { shallowOnly: false });
    for (const filePath of jsonlFiles) {
      const conv = await parseJsonlFile(filePath, opts);
      if (conv) {
        // Avoid duplicates if also in DB
        if (!conversations.some(c => c.externalId === conv.externalId)) {
          conversations.push(conv);
        }
      }

      if (opts?.limit && conversations.length >= opts.limit) break;
    }

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return opts?.limit ? conversations.slice(0, opts.limit) : conversations;
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

    if (opts.format !== 'goose') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = conversations.map(conv => {
      const jsonl = buildGooseJsonl(conv);
      const name = `${conv.externalId ?? 'session'}.jsonl`;
      return { path: name, content: jsonl };
    });

    return {
      format: 'goose',
      files,
      mimeType: 'application/x-ndjson',
    };
  },
};

async function parseDatabase(dbPath: string, opts?: ParseOptions): Promise<Conversation[]> {
  if (!Database) return [];

  const conversations: Conversation[] = [];

  try {
    const db = new Database(dbPath, { readonly: true });

    // Check if messages table exists (new schema)
    const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all() as { name: string }[];
    const hasMessagesTable = tables.some(t => t.name === 'messages');

    if (hasMessagesTable) {
      // Use new schema with separate messages table
      let query = `
        SELECT
          s.id,
          s.name as title,
          s.working_dir as working_directory,
          s.created_at,
          s.updated_at,
          s.description as metadata,
          m.id as msg_id,
          m.role,
          m.content_json,
          m.created_timestamp,
          m.metadata_json as msg_metadata
        FROM sessions s
        LEFT JOIN messages m ON s.id = m.session_id
        WHERE 1=1
      `;
      const params: unknown[] = [];

      if (opts?.since) {
        query += ' AND (s.created_at >= ? OR s.updated_at >= ?)';
        params.push(opts.since, opts.since);
      }

      query += ' ORDER BY COALESCE(s.updated_at, s.created_at) DESC, m.id ASC';

      if (opts?.limit) {
        // Note: LIMIT applies to sessions, not messages
        query = `
          SELECT * FROM (
            ${query}
          ) WHERE msg_id IS NOT NULL
        `;
      }

      const rows = db.prepare(query).all(...params) as any[];

      // Group messages by session
      const sessionMap = new Map<string, {
        title: string;
        working_directory?: string;
        created_at: string;
        updated_at?: string;
        metadata?: string;
        messages: any[];
      }>();

      for (const row of rows) {
        const sessionId = row.id;
        if (!sessionMap.has(sessionId)) {
          sessionMap.set(sessionId, {
            title: row.title,
            working_directory: row.working_directory,
            created_at: row.created_at,
            updated_at: row.updated_at,
            metadata: row.metadata,
            messages: [],
          });
        }

        if (row.role) {
          sessionMap.get(sessionId)!.messages.push({
            role: row.role,
            content_json: row.content_json,
            created_timestamp: row.created_timestamp,
            msg_metadata: row.msg_metadata,
          });
        }
      }

      // Convert to conversations
      for (const [id, session] of sessionMap.entries()) {
        const conv = parseSessionFromDb({
          id,
          title: session.title,
          working_directory: session.working_directory,
          created_at: session.created_at,
          updated_at: session.updated_at,
          metadata: session.metadata,
          messages: session.messages,
        });
        if (conv && conv.messages.length > 0) {
          conversations.push(conv);
        }
      }
    } else {
      // Fallback to old schema (messages embedded in session row)
      let query = `SELECT * FROM sessions WHERE 1=1`;
      const params: unknown[] = [];

      if (opts?.since) {
        query += ' AND (created_at >= ? OR updated_at >= ?)';
        params.push(opts.since, opts.since);
      }

      query += ' ORDER BY COALESCE(updated_at, created_at) DESC';

      if (opts?.limit) {
        query += ' LIMIT ?';
        params.push(opts.limit);
      }

      const rows = db.prepare(query).all(...params) as SessionRow[];

      for (const row of rows) {
        const conv = parseSessionRow(row);
        if (conv && conv.messages.length > 0) {
          conversations.push(conv);
        }
      }
    }

    db.close();
  } catch (err) {
    console.error('Error reading Goose database:', err);
  }

  return conversations;
}

function parseSessionRow(row: SessionRow): Conversation | null {
  let messages: Message[] = [];

  if (row.messages) {
    try {
      const msgData = JSON.parse(row.messages) as GooseMessage[];
      messages = msgData.map(m => parseGooseMessage(m)).filter((m): m is Message => m !== null);
    } catch { /* ignore */ }
  }

  if (messages.length === 0) return null;

  let metadata: Record<string, unknown> = {};
  if (row.metadata) {
    try {
      metadata = JSON.parse(row.metadata);
    } catch { /* ignore */ }
  }

  const createdAt = parseTimestamp(row.created_at) ?? Date.now();
  const updatedAt = row.updated_at ? parseTimestamp(row.updated_at) : undefined;

  return {
    externalId: row.session_id ?? row.id,
    title: (metadata.title as string) ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    workspace: row.working_directory ?? (metadata.working_directory as string),
    messages,
    metadata: {
      source: 'goose-db',
      ...metadata,
    },
  };
}

function parseSessionFromDb(row: {
  id: string;
  title?: string;
  working_directory?: string;
  created_at: string;
  updated_at?: string;
  metadata?: string;
  messages: Array<{
    role: string;
    content_json: string;
    created_timestamp: number;
    msg_metadata?: string;
  }>;
}): Conversation | null {
  const messages: Message[] = row.messages.map(m => {
    let content = '';
    try {
      const contentArray = JSON.parse(m.content_json) as GooseContent[];
      content = extractContent(contentArray);
    } catch { /* ignore */ }

    return {
      role: mapRole(m.role),
      content,
      parts: textOnlyParts(content),
      createdAt: m.created_timestamp * 1000, // Convert seconds to milliseconds
    };
  }).filter((m): m is Message => m.content || m.tool_calls?.length);

  if (messages.length === 0) return null;

  let metadata: Record<string, unknown> = {};
  if (row.metadata) {
    try {
      metadata = JSON.parse(row.metadata);
    } catch { /* ignore */ }
  }

  const createdAt = parseTimestamp(row.created_at) ?? Date.now();
  const updatedAt = row.updated_at ? parseTimestamp(row.updated_at) : undefined;

  return {
    externalId: row.id,
    title: row.title ?? (metadata.title as string) ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    workspace: row.working_directory ?? (metadata.working_directory as string),
    messages,
    metadata: {
      source: 'goose-db',
      ...metadata,
    },
  };
}

function parseGooseMessage(msg: GooseMessage): Message | null {
  if (!msg.role) return null;

  const content = extractContent(msg.content);
  if (!content && !msg.tool_calls?.length) return null;

  const toolCalls = msg.tool_calls?.map(tc => ({
    toolName: tc.function?.name ?? 'unknown',
    input: tc.function?.arguments ? safeJsonParse(tc.function.arguments) : undefined,
  }));

  // Build parts: text + any tool calls
  const canonParts = textOnlyParts(content) ?? [];
  if (toolCalls?.length) {
    for (const tc of msg.tool_calls ?? []) {
      const callId = tc.id ?? tc.function?.name ?? 'unknown';
      canonParts.push(toolCallPart(callId, tc.function?.name ?? 'unknown', tc.function?.arguments ? safeJsonParse(tc.function.arguments) : undefined));
    }
  }
  // Tool result messages (role=tool with tool_call_id)
  if (msg.tool_call_id) {
    canonParts.push(toolResultPart(msg.tool_call_id, content, { name: msg.name ?? undefined }));
  }

  return {
    role: mapRole(msg.role),
    content,
    parts: canonParts.length > 0 ? canonParts : undefined,
    createdAt: parseTimestamp(msg.created_at),
    toolCalls: toolCalls?.length ? toolCalls : undefined,
    metadata: {
      toolCallId: msg.tool_call_id,
      name: msg.name,
    },
  };
}

async function parseJsonlFile(filePath: string, opts?: ParseOptions): Promise<Conversation | null> {
  const raw = await readFile(filePath, 'utf-8').catch(() => null);
  if (!raw) return null;

  const lines = raw.split(/\r?\n/).filter(line => line.trim());
  if (lines.length === 0) return null;

  const messages: Message[] = [];
  let metadata: Record<string, unknown> = {};
  let firstTimestamp: number | undefined;
  let lastTimestamp: number | undefined;

  for (const line of lines) {
    try {
      const entry = JSON.parse(line) as JsonlEntry;

      if (entry.type === 'metadata' || entry.metadata) {
        metadata = { ...metadata, ...entry.metadata, ...entry };
        continue;
      }

      if (entry.role && entry.content) {
        const timestamp = parseTimestamp(entry.created ?? entry.timestamp);
        if (timestamp) {
          if (!firstTimestamp || timestamp < firstTimestamp) firstTimestamp = timestamp;
          if (!lastTimestamp || timestamp > lastTimestamp) lastTimestamp = timestamp;
        }

        const entryContent = extractContent(entry.content);
        messages.push({
          role: mapRole(entry.role),
          content: entryContent,
          parts: textOnlyParts(entryContent),
          createdAt: timestamp,
        });
      }
    } catch { /* skip invalid lines */ }
  }

  if (messages.length === 0) return null;

  const createdAt = firstTimestamp ?? Date.now();
  const updatedAt = lastTimestamp;

  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  return {
    externalId: basename(filePath, extname(filePath)),
    title: (metadata.title as string) ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    workspace: metadata.working_directory as string,
    messages,
    metadata: {
      file: filePath,
      source: 'goose-jsonl',
      ...metadata,
    },
  };
}

function extractContent(content?: string | GooseContent[]): string {
  if (!content) return '';
  if (typeof content === 'string') return content.trim();

  const parts: string[] = [];
  for (const part of content) {
    if (part.type === 'text' && part.text) {
      parts.push(part.text);
    }
  }
  return parts.join('\n').trim();
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
    case 'human':
      return 'user';
    case 'assistant':
    case 'ai':
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

function parseTimestamp(value?: string | number): number | undefined {
  if (!value) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
}

function safeJsonParse(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function deriveTitle(messages: Message[]): string | undefined {
  const firstUser = messages.find(m => m.role === 'user');
  if (!firstUser?.content) return undefined;
  const text = firstUser.content.slice(0, 80);
  return text.length < firstUser.content.length ? `${text}...` : text;
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
    if (conv.workspace) {
      blocks.push(`- Workspace: ${conv.workspace}`);
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

function buildGooseJsonl(conv: Conversation): string {
  const lines: string[] = [];

  // Metadata line
  lines.push(JSON.stringify({
    type: 'metadata',
    title: conv.title,
    working_directory: conv.workspace,
    created_at: new Date(conv.createdAt).toISOString(),
  }));

  // Message lines
  for (const msg of conv.messages) {
    lines.push(JSON.stringify({
      role: msg.role,
      content: msg.content,
      timestamp: msg.createdAt ? new Date(msg.createdAt).toISOString() : undefined,
    }));
  }

  return lines.join('\n') + '\n';
}

async function findJsonlFiles(path: string, opts: { shallowOnly: boolean }): Promise<string[]> {
  const files: string[] = [];
  const stats = await stat(path).catch(() => null);
  if (!stats?.isDirectory()) return files;

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(path, entry.name);
    if (entry.isFile() && extname(entry.name) === '.jsonl') {
      files.push(entryPath);
    } else if (entry.isDirectory() && !opts.shallowOnly) {
      const nested = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const child of nested) {
        if (child.isFile() && extname(child.name) === '.jsonl') {
          files.push(join(entryPath, child.name));
        }
      }
    }
  }
  return files;
}

runAdapter(adapter);
