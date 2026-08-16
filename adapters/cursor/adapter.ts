/**
 * Cursor adapter for hstry
 *
 * Parses Cursor chat history from VSCode SQLite state files
 * Location: ~/Library/Application Support/Cursor/User/workspaceStorage/<hash>/state.vscdb
 */

import { readFile, readdir, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import { gunzipSync } from 'zlib';
import type {
  Adapter,
  AdapterInfo,
  Conversation,
  Message,
  ParseOptions,
  ToolCall,
} from '../types/index.ts';
import { runAdapter, textOnlyParts } from '../types/index.ts';

interface SqliteStatement {
  get(...params: unknown[]): unknown;
  all(...params: unknown[]): unknown[];
}

interface SqliteDatabase {
  query?(sql: string): SqliteStatement;
  prepare?(sql: string): SqliteStatement;
  close(): void;
}

type OpenDatabase = (path: string) => SqliteDatabase;

let openDatabase: OpenDatabase | null = null;
try {
  const isBun = typeof (globalThis as typeof globalThis & { Bun?: unknown }).Bun !== 'undefined';
  if (isBun) {
    const { Database } = await import('bun:sqlite');
    openDatabase = path => new Database(path, { readonly: true });
  } else {
    const { default: Database } = await import('better-sqlite3');
    openDatabase = path => new Database(path, { readonly: true });
  }
} catch {
  // SQLite is unavailable in this runtime; detection returns no match.
}

function statement(db: SqliteDatabase, sql: string): SqliteStatement {
  const prepared = db.query?.(sql) ?? db.prepare?.(sql);
  if (!prepared) throw new Error('SQLite runtime does not support prepared queries');
  return prepared;
}

function getRow(db: SqliteDatabase, sql: string, parameter: string): StateRow | undefined {
  return statement(db, sql).get(parameter) as StateRow | undefined;
}

function getJson<T>(db: SqliteDatabase, key: string): T | undefined {
  for (const table of ['ItemTable', 'cursorDiskKV']) {
    try {
      const row = getRow(db, `SELECT value FROM ${table} WHERE key = ?`, key);
      if (row?.value) return JSON.parse(row.value) as T;
    } catch { /* table/key absent */ }
  }
  return undefined;
}

function listKeys(db: SqliteDatabase, prefix: string): string[] {
  const keys = new Set<string>();
  for (const table of ['ItemTable', 'cursorDiskKV']) {
    try {
      const rows = statement(db, `SELECT key FROM ${table} WHERE key LIKE ?`).all(`${prefix}%`) as StateRow[];
      for (const row of rows) keys.add(row.key);
    } catch { /* table absent */ }
  }
  return [...keys];
}

// Platform-specific paths
const DEFAULT_PATHS = (() => {
  const home = homedir();
  const snapshots = join(home, '.cursaves', 'snapshots');
  switch (process.platform) {
    case 'darwin': {
      const user = join(home, 'Library', 'Application Support', 'Cursor', 'User');
      return [join(user, 'globalStorage'), join(user, 'workspaceStorage'), snapshots];
    }
    case 'win32': {
      const user = join(process.env.APPDATA || join(home, 'AppData', 'Roaming'), 'Cursor', 'User');
      return [join(user, 'globalStorage'), join(user, 'workspaceStorage'), snapshots];
    }
    default: { // Linux
      const user = join(home, '.config', 'Cursor', 'User');
      return [join(user, 'globalStorage'), join(user, 'workspaceStorage'), snapshots];
    }
  }
})();

interface StateRow {
  key: string;
  value: string;
}

interface CursorChatData {
  tabs?: CursorTab[];
  currentTabId?: string;
}

interface CursorTab {
  id?: string;
  title?: string;
  createdAt?: number;
  lastUpdatedAt?: number;
  bubbles?: CursorBubble[];
}

interface CursorBubble {
  type?: string;
  text?: string;
  rawText?: string;
  state?: string;
  modelType?: string;
  timingInfo?: { startTime?: number; endTime?: number };
}

interface CursorPrompt {
  id?: string;
  prompt?: string;
  response?: string;
  createdAt?: number;
  conversationId?: string;
  model?: string;
}

interface ComposerHeader {
  composerId?: string;
  name?: string;
  createdAt?: number | string;
  lastUpdatedAt?: number | string;
  workspaceIdentifier?: { uri?: { fsPath?: string; path?: string } };
}

interface ComposerBubbleHeader {
  bubbleId?: string;
  type?: number;
}

interface ComposerBubble {
  type?: number;
  text?: string;
  richText?: string;
  createdAt?: number | string;
  toolResults?: unknown;
}

interface ComposerData {
  name?: string;
  createdAt?: number | string;
  lastUpdatedAt?: number | string;
  fullConversationHeadersOnly?: ComposerBubbleHeader[];
  conversationMap?: Record<string, ComposerBubble>;
}

interface ComposerSnapshot {
  composerId?: string;
  sourceProjectPath?: string;
  composerData?: ComposerData;
  bubbleEntries?: Record<string, ComposerBubble>;
}

const CHAT_DATA_KEY = 'workbench.panel.aichat.view.aichat.chatdata';
const PROMPTS_KEY = 'aiService.prompts';
const COMPOSER_HEADERS_KEY = 'composer.composerHeaders';
const MAX_COMPOSER_TEXT = 20_000;

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'cursor',
      displayName: 'Cursor',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!openDatabase) return null;

    const snapshotFiles = await findSnapshotFiles(path);
    if (snapshotFiles.length > 0) return 0.9;

    const dbFiles = await findStateFiles(path);
    if (dbFiles.length === 0) return null;

    // Check if any database has cursor chat data
    for (const dbPath of dbFiles.slice(0, 3)) {
      try {
        const db = openDatabase(dbPath);
        const hasLegacyChat = getRow(
          db,
          'SELECT value FROM ItemTable WHERE key = ?',
          CHAT_DATA_KEY,
        );
        const hasComposer = getJson<{ allComposers?: ComposerHeader[] }>(
          db,
          COMPOSER_HEADERS_KEY,
        ) || listKeys(db, 'composerData:').length > 0;
        db.close();
        if (hasComposer) return 0.95;
        if (hasLegacyChat) return 0.9;
      } catch { /* continue */ }
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    if (!openDatabase) return [];

    const dbFiles = await findStateFiles(path);
    const snapshotFiles = await findSnapshotFiles(path);
    if (dbFiles.length === 0 && snapshotFiles.length === 0) return [];

    const conversations: Conversation[] = [];
    const seenIds = new Set<string>();

    for (const snapshotPath of snapshotFiles) {
      const conversation = await parseComposerSnapshot(snapshotPath, opts);
      if (!conversation || !conversation.externalId || seenIds.has(conversation.externalId)) continue;
      seenIds.add(conversation.externalId);
      conversations.push(conversation);
      if (opts?.limit && conversations.length >= opts.limit) break;
    }

    for (const dbPath of dbFiles) {
      if (opts?.limit && conversations.length >= opts.limit) break;
      const workspaceId = basename(dbPath.replace(/\/state\.vscdb$/, ''));
      const convs = await parseStateDb(dbPath, workspaceId, opts);
      
      for (const conv of convs) {
        // Avoid duplicates
        const key = conv.externalId ?? `${conv.createdAt}-${conv.title}`;
        if (seenIds.has(key)) continue;
        seenIds.add(key);
        
        conversations.push(conv);

        if (opts?.limit && conversations.length >= opts.limit) break;
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

    // Cursor doesn't have a good import format, export as generic JSON
    if (opts.format === 'cursor') {
      return {
        format: 'cursor',
        content: JSON.stringify(conversations, null, opts.pretty ? 2 : 0),
        mimeType: 'application/json',
      };
    }

    throw new Error(`Unsupported export format: ${opts.format}`);
  },
};

async function parseStateDb(dbPath: string, workspaceId: string, opts?: ParseOptions): Promise<Conversation[]> {
  const conversations: Conversation[] = [];

  try {
    if (!openDatabase) return conversations;
    const db = openDatabase(dbPath);

    conversations.push(...parseComposerData(db, dbPath, opts));

    // Try to get legacy chat data.
    const chatRow = getRow(db, 'SELECT value FROM ItemTable WHERE key = ?', CHAT_DATA_KEY);
    if (chatRow) {
      const chatConvs = parseChatData(chatRow.value, workspaceId, opts);
      conversations.push(...chatConvs);
    }

    // Also try prompts (may have additional data)
    const promptsRow = getRow(db, 'SELECT value FROM ItemTable WHERE key = ?', PROMPTS_KEY);
    if (promptsRow) {
      const promptConvs = parsePrompts(promptsRow.value, workspaceId, opts);
      conversations.push(...promptConvs);
    }

    db.close();
  } catch (err) {
    console.error('Error reading Cursor database:', err);
  }

  return conversations;
}

function toMilliseconds(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
    return Math.floor(value < 1e12 ? value * 1000 : value);
  }
  if (typeof value === 'string') {
    const numeric = Number(value);
    if (Number.isFinite(numeric) && numeric > 0) {
      return Math.floor(numeric < 1e12 ? numeric * 1000 : numeric);
    }
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return Math.floor(parsed);
  }
  return undefined;
}

function composerText(bubble?: ComposerBubble): string {
  const text = bubble?.text?.trim() || bubble?.richText?.trim() || '';
  return text.length > MAX_COMPOSER_TEXT
    ? `${text.slice(0, MAX_COMPOSER_TEXT - 1)}…`
    : text;
}

function composerToolCalls(bubble?: ComposerBubble): ToolCall[] {
  if (!bubble?.toolResults) return [];
  const rawTools = Array.isArray(bubble.toolResults) ? bubble.toolResults : [bubble.toolResults];
  return rawTools.flatMap(raw => {
    if (!raw || typeof raw !== 'object') return [];
    const tool = raw as Record<string, unknown>;
    const toolName = typeof tool.name === 'string'
      ? tool.name
      : typeof tool.toolName === 'string' ? tool.toolName : 'tool';
    const output = typeof tool.result === 'string'
      ? tool.result
      : JSON.stringify(tool.result ?? tool.output ?? '');
    return [{
      toolName,
      input: tool.params ?? tool.input,
      output: output.slice(0, 8_000),
      status: 'success' as const,
    }];
  });
}

function composerConversation(
  composerId: string,
  data: ComposerData,
  bubbles: Record<string, ComposerBubble>,
  header: ComposerHeader | undefined,
  dbPath: string,
  opts?: ParseOptions,
): Conversation | null {
  const messages: Message[] = [];
  let firstTimestamp: number | undefined;
  let lastTimestamp: number | undefined;

  const append = (bubbleHeader: ComposerBubbleHeader, bubble?: ComposerBubble) => {
    const content = composerText(bubble);
    if (!content) return;
    const createdAt = toMilliseconds(bubble?.createdAt) ?? toMilliseconds(data.createdAt);
    if (createdAt !== undefined) {
      firstTimestamp = firstTimestamp === undefined ? createdAt : Math.min(firstTimestamp, createdAt);
      lastTimestamp = lastTimestamp === undefined ? createdAt : Math.max(lastTimestamp, createdAt);
    }
    const type = bubbleHeader.type ?? bubble?.type;
    if (type === 1) {
      messages.push({ role: 'user', content, parts: textOnlyParts(content), createdAt });
    } else if (type === 2) {
      const toolCalls = composerToolCalls(bubble);
      messages.push({
        role: 'assistant',
        content,
        parts: textOnlyParts(content),
        createdAt,
        toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
      });
    }
  };

  const headers = data.fullConversationHeadersOnly ?? [];
  if (headers.length > 0) {
    for (const bubbleHeader of headers) {
      if (!bubbleHeader.bubbleId) continue;
      append(bubbleHeader, bubbles[bubbleHeader.bubbleId] ?? data.conversationMap?.[bubbleHeader.bubbleId]);
    }
  } else {
    for (const [bubbleId, bubble] of Object.entries(data.conversationMap ?? bubbles)) {
      append({ bubbleId, type: bubble.type }, bubble);
    }
  }
  if (messages.length === 0) return null;

  const createdAt = firstTimestamp
    ?? toMilliseconds(data.createdAt)
    ?? toMilliseconds(header?.createdAt)
    ?? Date.now();
  const updatedAt = lastTimestamp
    ?? toMilliseconds(data.lastUpdatedAt)
    ?? toMilliseconds(header?.lastUpdatedAt)
    ?? createdAt;
  if (opts?.since && createdAt < opts.since && updatedAt < opts.since) return null;

  return {
    externalId: composerId,
    title: data.name ?? header?.name ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    provider: 'cursor',
    workspace: header?.workspaceIdentifier?.uri?.fsPath
      ?? header?.workspaceIdentifier?.uri?.path
      ?? basename(dbPath.replace(/[/\\]state\.vscdb$/, '')),
    messages,
    metadata: { source: 'cursor-composer', composerId, file: dbPath },
  };
}

function parseComposerData(
  db: SqliteDatabase,
  dbPath: string,
  opts?: ParseOptions,
): Conversation[] {
  const headers = getJson<{ allComposers?: ComposerHeader[] }>(db, COMPOSER_HEADERS_KEY)
    ?.allComposers ?? [];
  const composerIds = new Set(headers.flatMap(header => header.composerId ? [header.composerId] : []));
  for (const key of listKeys(db, 'composerData:')) {
    composerIds.add(key.slice('composerData:'.length));
  }

  const conversations: Conversation[] = [];
  for (const composerId of composerIds) {
    const data = getJson<ComposerData>(db, `composerData:${composerId}`);
    if (!data) continue;
    const bubbles = { ...(data.conversationMap ?? {}) };
    for (const key of listKeys(db, `bubbleId:${composerId}:`)) {
      const bubbleId = key.slice(`bubbleId:${composerId}:`.length);
      const bubble = getJson<ComposerBubble>(db, key);
      if (bubble) bubbles[bubbleId] = bubble;
    }
    const conversation = composerConversation(
      composerId,
      data,
      bubbles,
      headers.find(header => header.composerId === composerId),
      dbPath,
      opts,
    );
    if (conversation) conversations.push(conversation);
  }
  return conversations;
}

async function parseComposerSnapshot(
  snapshotPath: string,
  opts?: ParseOptions,
): Promise<Conversation | null> {
  try {
    const bytes = await readFile(snapshotPath);
    const decoded = snapshotPath.endsWith('.gz') ? gunzipSync(bytes) : bytes;
    const snapshot = JSON.parse(decoded.toString('utf8')) as ComposerSnapshot;
    const composerId = snapshot.composerId
      ?? basename(snapshotPath).replace(/\.json(?:\.gz)?$/i, '').replace(/\.\d+$/, '');
    const header: ComposerHeader | undefined = snapshot.sourceProjectPath
      ? { workspaceIdentifier: { uri: { fsPath: snapshot.sourceProjectPath } } }
      : undefined;
    const conversation = composerConversation(
      composerId,
      snapshot.composerData ?? {},
      snapshot.bubbleEntries ?? {},
      header,
      snapshotPath,
      opts,
    );
    if (conversation) {
      conversation.metadata = {
        ...conversation.metadata,
        source: 'cursor-composer-snapshot',
        file: snapshotPath,
      };
    }
    return conversation;
  } catch {
    return null;
  }
}

function parseChatData(value: string, workspaceId: string, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];

  try {
    const data = JSON.parse(value) as CursorChatData;
    if (!data.tabs) return conversations;

    for (const tab of data.tabs) {
      const messages = parseTabBubbles(tab.bubbles);
      if (messages.length === 0) continue;

      const createdAt = tab.createdAt ?? messages[0].createdAt ?? Date.now();
      const updatedAt = tab.lastUpdatedAt ?? messages[messages.length - 1].createdAt;

      // Check incremental sync
      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      conversations.push({
        externalId: tab.id,
        title: tab.title ?? deriveTitle(messages),
        createdAt,
        updatedAt,
        workspace: workspaceId,
        messages,
        metadata: {
          source: 'cursor-chat',
          tabId: tab.id,
        },
      });
    }
  } catch { /* ignore parse errors */ }

  return conversations;
}

function parseTabBubbles(bubbles?: CursorBubble[]): Message[] {
  if (!bubbles) return [];

  const messages: Message[] = [];

  for (const bubble of bubbles) {
    if (!bubble.type) continue;

    const content = bubble.text ?? bubble.rawText;
    if (!content) continue;

    const role = bubble.type === 'user' ? 'user' : 'assistant';
    const createdAt = bubble.timingInfo?.startTime;

    messages.push({
      role,
      content,
      parts: textOnlyParts(content),
      createdAt,
      model: bubble.modelType,
      metadata: {
        state: bubble.state,
      },
    });
  }

  return messages;
}

function parsePrompts(value: string, workspaceId: string, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];

  try {
    const prompts = JSON.parse(value) as CursorPrompt[];
    if (!Array.isArray(prompts)) return conversations;

    // Group prompts by conversationId
    const grouped = new Map<string, CursorPrompt[]>();
    for (const prompt of prompts) {
      const convId = prompt.conversationId ?? 'default';
      if (!grouped.has(convId)) {
        grouped.set(convId, []);
      }
      grouped.get(convId)!.push(prompt);
    }

    for (const [convId, convPrompts] of grouped) {
      const messages: Message[] = [];
      let firstTime: number | undefined;
      let lastTime: number | undefined;

      for (const prompt of convPrompts) {
        if (prompt.prompt) {
          messages.push({
            role: 'user',
            content: prompt.prompt,
            parts: textOnlyParts(prompt.prompt),
            createdAt: prompt.createdAt,
          });
        }
        if (prompt.response) {
          messages.push({
            role: 'assistant',
            content: prompt.response,
            parts: textOnlyParts(prompt.response),
            createdAt: prompt.createdAt,
            model: prompt.model,
          });
        }

        if (prompt.createdAt) {
          if (!firstTime || prompt.createdAt < firstTime) firstTime = prompt.createdAt;
          if (!lastTime || prompt.createdAt > lastTime) lastTime = prompt.createdAt;
        }
      }

      if (messages.length === 0) continue;

      const createdAt = firstTime ?? Date.now();
      const updatedAt = lastTime;

      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      conversations.push({
        externalId: convId,
        title: deriveTitle(messages),
        createdAt,
        updatedAt,
        workspace: workspaceId,
        messages,
        metadata: {
          source: 'cursor-prompts',
        },
      });
    }
  } catch { /* ignore */ }

  return conversations;
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

async function findSnapshotFiles(path: string, depth = 4): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];
  if (stats.isFile()) {
    return /\.json(?:\.gz)?$/i.test(path) ? [path] : [];
  }
  if (!stats.isDirectory() || depth <= 0) return [];

  const files: string[] = [];
  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(path, entry.name);
    if (entry.isDirectory()) {
      files.push(...await findSnapshotFiles(entryPath, depth - 1));
    } else if (entry.isFile() && /\.json(?:\.gz)?$/i.test(entry.name)) {
      files.push(entryPath);
    }
  }
  return files.sort();
}

async function findStateFiles(path: string): Promise<string[]> {
  const files: string[] = [];
  const stats = await stat(path).catch(() => null);
  
  if (!stats) return files;

  // Direct file
  if (stats.isFile() && path.endsWith('.vscdb')) {
    files.push(path);
    return files;
  }

  if (!stats.isDirectory()) return files;

  // globalStorage itself contains state.vscdb; workspaceStorage contains
  // one state database per child directory.
  const rootDatabase = join(path, 'state.vscdb');
  if ((await stat(rootDatabase).catch(() => null))?.isFile()) {
    files.push(rootDatabase);
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;

    const dbPath = join(path, entry.name, 'state.vscdb');
    const dbStats = await stat(dbPath).catch(() => null);
    if (dbStats?.isFile()) {
      files.push(dbPath);
    }
  }

  return files;
}

runAdapter(adapter);
