/**
 * OpenCode adapter for hstry
 * 
 * Parses OpenCode session history from ~/.local/share/opencode/
 * 
 * Supports three layouts:
 * 
 * SQLITE (v1.1.45+):
 *   opencode.db (SQLite database with session, message, part, project tables)
 * 
 * NEW (v1.1.25+): 
 *   storage/session/<project-id>/<session-id>.json
 *   storage/message/<session-id>/<msg-id>.json
 *   storage/part/<msg-id>/<part-id>.json
 * 
 * OLD (legacy):
 *   project/<project-name>/storage/session/info/<session-id>.json
 *   project/<project-name>/storage/session/message/<session-id>/<msg-id>.json
 *   project/<project-name>/storage/session/part/<session-id>/<msg-id>/<part-id>.json
 */

import { readdir, readFile, stat } from 'fs/promises';
import { existsSync } from 'fs';
import { join, basename } from 'path';
import { homedir } from 'os';
import type { 
  Adapter, 
  AdapterInfo, 
  CanonPart,
  Conversation, 
  Message, 
  ParseOptions,
  ParseStreamResult,
  ToolCall,
} from '../types/index.ts';
import {
  runAdapter,
  textPart,
  toolCallPart,
  toolResultPart,
  textOnlyParts,
} from '../types/index.ts';

// Dynamic SQLite support: bun:sqlite (Bun) or better-sqlite3 (Node)
let openDb: ((path: string) => SqliteDb) | null = null;

interface SqliteDb {
  prepare(sql: string): SqliteStatement;
  close(): void;
}
interface SqliteStatement {
  all(...params: unknown[]): unknown[];
  get(...params: unknown[]): unknown;
}

try {
  if (typeof Bun !== 'undefined') {
    // @ts-ignore - bun:sqlite is Bun-only
    const { Database: BunDb } = await import('bun:sqlite');
    openDb = (path: string) => new BunDb(path, { readonly: true }) as unknown as SqliteDb;
  } else {
    const mod = await import('better-sqlite3');
    const BetterSqlite = mod.default;
    openDb = (path: string) => BetterSqlite(path, { readonly: true }) as unknown as SqliteDb;
  }
} catch {
  // SQLite not available - sqlite layout will be skipped
}

declare const Bun: unknown;

// OpenCode storage structures
interface SessionInfo {
  id: string;
  version?: string;
  slug?: string;
  title?: string;
  parentID?: string;
  directory?: string;
  projectID?: string;
  time: {
    created: number;
    updated: number;
  };
  summary?: {
    additions?: number;
    deletions?: number;
    files?: number;
  };
}

interface MessageInfo {
  id: string;
  sessionID: string;
  role: string;
  time: {
    created: number;
    completed?: number;
  };
  parentID?: string;
  modelID?: string;
  providerID?: string;
  agent?: string;
  summary?: {
    title?: string;
  };
  tokens?: {
    input?: number;
    output?: number;
    reasoning?: number;
  };
  cost?: number;
}

interface PartInfo {
  id: string;
  messageID: string;
  sessionID: string;
  type: string;
  text?: string;
  tool?: string;
  state?: {
    status?: string;
    input?: unknown;
    output?: string;
    title?: string;
  };
}

const DEFAULT_OPENCODE_PATH = join(homedir(), '.local/share/opencode');

/** A lightweight reference to a session, used for streaming enumeration. */
interface SessionRef {
  /** Path to the session JSON file. */
  path: string;
  /** 'new' or 'old' layout. */
  layout: 'new' | 'old';
  /** For old layout: the project name directory. */
  projectName?: string;
}

/**
 * Enumerate all session file references without loading their contents.
 * Deduplicates sessions that appear in both layouts (prefers new layout).
 */
async function enumerateSessionRefs(basePath: string): Promise<SessionRef[]> {
  const refs: SessionRef[] = [];
  const seenIds = new Set<string>();

  // Enumerate new layout first (preferred)
  const newSessionDir = join(basePath, 'storage', 'session');
  try {
    const projectIds = await readdir(newSessionDir);
    for (const projectId of projectIds) {
      const projectPath = join(newSessionDir, projectId);
      try {
        const projectStats = await stat(projectPath);
        if (!projectStats.isDirectory()) continue;
        const sessionFiles = await readdir(projectPath);
        for (const f of sessionFiles) {
          if (f.startsWith('ses_') && f.endsWith('.json')) {
            const sessionId = f.replace('.json', '');
            seenIds.add(sessionId);
            refs.push({ path: join(projectPath, f), layout: 'new' });
          }
        }
      } catch { /* skip */ }
    }
  } catch { /* no new layout */ }

  // Enumerate old layout (skip sessions already seen in new layout)
  const oldProjectDir = join(basePath, 'project');
  try {
    const projects = await readdir(oldProjectDir);
    for (const projectName of projects) {
      const infoDir = join(oldProjectDir, projectName, 'storage/session/info');
      try {
        const sessionFiles = await readdir(infoDir);
        for (const f of sessionFiles) {
          if (f.startsWith('ses_') && f.endsWith('.json')) {
            const sessionId = f.replace('.json', '');
            if (!seenIds.has(sessionId)) {
              seenIds.add(sessionId);
              refs.push({ path: join(infoDir, f), layout: 'old', projectName });
            }
          }
        }
      } catch { /* skip */ }
    }
  } catch { /* no old layout */ }

  return refs;
}

// Detect which layout is being used
type LayoutType = 'sqlite' | 'new' | 'old' | 'none';

async function detectLayout(basePath: string): Promise<LayoutType> {
  // Check SQLite layout first (newest, v1.1.45+)
  if (openDb && existsSync(join(basePath, 'opencode.db'))) {
    return 'sqlite';
  }

  // Check new layout: storage/session/<project-id>/
  const newStoragePath = join(basePath, 'storage', 'session');
  try {
    const stats = await stat(newStoragePath);
    if (stats.isDirectory()) {
      const dirs = await readdir(newStoragePath);
      for (const dir of dirs) {
        const projectPath = join(newStoragePath, dir);
        const projectStats = await stat(projectPath);
        if (projectStats.isDirectory()) {
          const sessions = await readdir(projectPath);
          if (sessions.some(s => s.startsWith('ses_') && s.endsWith('.json'))) {
            return 'new';
          }
        }
      }
    }
  } catch { /* ignore */ }

  // Check old layout: project/<name>/storage/session/info/
  const oldProjectPath = join(basePath, 'project');
  try {
    const stats = await stat(oldProjectPath);
    if (stats.isDirectory()) {
      const projects = await readdir(oldProjectPath);
      for (const project of projects) {
        const infoPath = join(oldProjectPath, project, 'storage/session/info');
        try {
          const infoStats = await stat(infoPath);
          if (infoStats.isDirectory()) {
            const sessions = await readdir(infoPath);
            if (sessions.some(s => s.startsWith('ses_') && s.endsWith('.json'))) {
              return 'old';
            }
          }
        } catch { /* continue */ }
      }
    }
  } catch { /* ignore */ }

  return 'none';
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'opencode',
      displayName: 'OpenCode',
      version: '3.0.0',
      defaultPaths: [DEFAULT_OPENCODE_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    const layout = await detectLayout(path);
    return layout !== 'none' ? 0.95 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const layout = await detectLayout(path);
    
    if (layout === 'sqlite') {
      return parseSqliteLayout(path, opts);
    } else if (layout === 'new') {
      return parseNewLayout(path, opts);
    } else if (layout === 'old') {
      return parseOldLayout(path, opts);
    }
    
    return [];
  },

  supportsIncremental: true,

  async parseSince(path: string, since: number): Promise<Conversation[]> {
    return this.parse(path, { since });
  },

  async parseStream(path: string, opts?: ParseOptions): Promise<ParseStreamResult> {
    const layout = await detectLayout(path);

    // SQLite layout: use LIMIT/OFFSET for efficient batching
    if (layout === 'sqlite') {
      return parseSqliteStream(path, opts);
    }

    // JSON layouts: enumerate session refs and batch through them
    const batchSize = opts?.batchSize ?? 5;
    const cursor = opts?.cursor as { index: number; total: number } | undefined;

    // Always re-enumerate refs (lightweight - just readdir, no file reads).
    // The cursor only carries the index to keep it small.
    const refs = await enumerateSessionRefs(path);
    const startIndex = cursor?.index ?? 0;

    const conversations: Conversation[] = [];
    const endIndex = Math.min(startIndex + batchSize, refs.length);

    for (let i = startIndex; i < endIndex; i++) {
      const ref = refs[i];
      try {
        const conv = await parseSingleSession(path, ref, opts);
        if (conv) {
          conversations.push(conv);
        }
      } catch (err) {
        // Skip individual session errors to avoid blocking the whole sync
        console.error(`Error parsing session ${ref.path}: ${err}`);
      }
    }

    const done = endIndex >= refs.length;

    return {
      conversations,
      cursor: done ? undefined : { index: endIndex, total: refs.length },
      done,
    };
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

    if (opts.format !== 'opencode') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = buildOpenCodeFiles(conversations);
    return {
      format: 'opencode',
      files,
      mimeType: 'application/json',
      metadata: {
        root: 'project/',
      },
    };
  },
};

/**
 * Parse a single session from either layout, given a SessionRef.
 * Used by parseStream to process one session at a time.
 */

// ============================================================================
// SQLite layout (v1.1.45+)
// ============================================================================

/** SQLite row types matching the opencode schema */
interface SqliteSessionRow {
  id: string;
  project_id: string;
  parent_id: string | null;
  slug: string;
  directory: string;
  title: string;
  version: string;
  summary_additions: number | null;
  summary_deletions: number | null;
  summary_files: number | null;
  time_created: number;
  time_updated: number;
}

interface SqliteMessageRow {
  id: string;
  session_id: string;
  time_created: number;
  time_updated: number;
  data: string; // JSON blob
}

interface SqlitePartRow {
  id: string;
  message_id: string;
  session_id: string;
  time_created: number;
  time_updated: number;
  data: string; // JSON blob
}

interface SqliteProjectRow {
  id: string;
  worktree: string;
}

/**
 * Parse all conversations from the SQLite layout.
 */
async function parseSqliteLayout(basePath: string, opts?: ParseOptions): Promise<Conversation[]> {
  const result = await parseSqliteStream(basePath, { ...opts, batchSize: 999999 });
  return result.conversations;
}

/**
 * Parse conversations from SQLite in batches using LIMIT/OFFSET.
 */
async function parseSqliteStream(basePath: string, opts?: ParseOptions): Promise<ParseStreamResult> {
  if (!openDb) {
    return { conversations: [], done: true };
  }

  const dbPath = join(basePath, 'opencode.db');
  const batchSize = opts?.batchSize ?? 25;
  const cursor = opts?.cursor as { offset: number } | undefined;
  const offset = cursor?.offset ?? 0;

  const db = openDb(dbPath);
  try {
    // Build session query with optional time filter
    let sessionQuery = `
      SELECT s.id, s.project_id, s.parent_id, s.slug, s.directory, s.title,
             s.version, s.summary_additions, s.summary_deletions, s.summary_files,
             s.time_created, s.time_updated
      FROM session s
    `;
    const params: unknown[] = [];

    if (opts?.since) {
      sessionQuery += ` WHERE s.time_created >= ? OR s.time_updated >= ?`;
      params.push(opts.since, opts.since);
    }

    sessionQuery += ` ORDER BY s.time_created DESC LIMIT ? OFFSET ?`;
    params.push(batchSize + 1, offset); // fetch one extra to check if done

    const sessionRows = db.prepare(sessionQuery).all(...params) as SqliteSessionRow[];

    const hasMore = sessionRows.length > batchSize;
    const rows = hasMore ? sessionRows.slice(0, batchSize) : sessionRows;

    if (rows.length === 0) {
      return { conversations: [], done: true };
    }

    // Build a project worktree lookup
    const projectIds = [...new Set(rows.map(r => r.project_id))];
    const projectMap = new Map<string, string>();
    if (projectIds.length > 0) {
      const placeholders = projectIds.map(() => '?').join(',');
      const projects = db.prepare(
        `SELECT id, worktree FROM project WHERE id IN (${placeholders})`
      ).all(...projectIds) as SqliteProjectRow[];
      for (const p of projects) {
        projectMap.set(p.id, p.worktree);
      }
    }

    // Load messages and parts for all sessions in this batch
    const sessionIds = rows.map(r => r.id);
    const placeholders = sessionIds.map(() => '?').join(',');

    const messageRows = db.prepare(
      `SELECT id, session_id, time_created, time_updated, data
       FROM message WHERE session_id IN (${placeholders})
       ORDER BY session_id, time_created`
    ).all(...sessionIds) as SqliteMessageRow[];

    // Group messages by session
    const messagesBySession = new Map<string, SqliteMessageRow[]>();
    for (const row of messageRows) {
      let list = messagesBySession.get(row.session_id);
      if (!list) {
        list = [];
        messagesBySession.set(row.session_id, list);
      }
      list.push(row);
    }

    // Load parts for all messages
    const messageIds = messageRows.map(r => r.id);
    const partsByMessage = new Map<string, SqlitePartRow[]>();
    if (messageIds.length > 0) {
      // Process in chunks to avoid SQLite variable limit
      const chunkSize = 500;
      for (let i = 0; i < messageIds.length; i += chunkSize) {
        const chunk = messageIds.slice(i, i + chunkSize);
        const ph = chunk.map(() => '?').join(',');
        const partRows = db.prepare(
          `SELECT id, message_id, session_id, time_created, time_updated, data
           FROM part WHERE message_id IN (${ph})
           ORDER BY message_id, id`
        ).all(...chunk) as SqlitePartRow[];

        for (const row of partRows) {
          let list = partsByMessage.get(row.message_id);
          if (!list) {
            list = [];
            partsByMessage.set(row.message_id, list);
          }
          list.push(row);
        }
      }
    }

    // Build conversations
    const conversations: Conversation[] = [];
    for (const session of rows) {
      try {
        const conv = buildSqliteConversation(
          session,
          messagesBySession.get(session.id) ?? [],
          partsByMessage,
          projectMap.get(session.project_id),
          opts,
        );
        if (conv) conversations.push(conv);
      } catch (err) {
        console.error(`Error building conversation for session ${session.id}: ${err}`);
      }
    }

    const done = !hasMore;

    return {
      conversations,
      cursor: done ? undefined : { offset: offset + batchSize },
      done,
    };
  } finally {
    db.close();
  }
}

/**
 * Build a Conversation from SQLite rows.
 */
function buildSqliteConversation(
  session: SqliteSessionRow,
  messageRows: SqliteMessageRow[],
  partsByMessage: Map<string, SqlitePartRow[]>,
  worktree: string | undefined,
  opts?: ParseOptions,
): Conversation | null {
  const messages: Message[] = [];

  for (const msgRow of messageRows) {
    try {
      const msgData = JSON.parse(msgRow.data);
      const role = msgData.role ?? 'assistant';

      // Reconstruct parts from the part table
      const partRows = partsByMessage.get(msgRow.id) ?? [];
      const parts: PartInfo[] = [];

      for (const partRow of partRows) {
        try {
          const partData = JSON.parse(partRow.data);
          // The data blob stores type/text/tool/state etc. directly
          parts.push({
            id: partRow.id,
            messageID: msgRow.id,
            sessionID: msgRow.session_id,
            type: partData.type ?? 'text',
            text: partData.text,
            tool: partData.tool,
            state: partData.state,
          });
        } catch {
          // Skip corrupted part
        }
      }

      const textParts = parts.filter(p => p.type === 'text' && p.text);
      const content = textParts.map(p => p.text).join('\n');

      let toolCalls: ToolCall[] | undefined;
      if (opts?.includeTools !== false) {
        const toolParts = parts.filter(p => p.type === 'tool' && p.tool);
        if (toolParts.length > 0) {
          toolCalls = toolParts.map(p => ({
            toolName: p.tool!,
            input: p.state?.input,
            output: p.state?.output ?? p.state?.error,
            status: mapToolStatus(p.state?.status),
          }));
        }
      }

      const canonParts = buildCanonParts(parts, role, opts?.includeTools !== false) ?? textOnlyParts(content);

      // Extract token info from assistant message data
      const tokens = (msgData.tokens?.input || 0) + (msgData.tokens?.output || 0) + (msgData.tokens?.reasoning || 0);

      messages.push({
        role: mapRole(role),
        content,
        parts: canonParts,
        createdAt: msgData.time?.created ?? msgRow.time_created,
        model: msgData.modelID || undefined,
        tokens: tokens > 0 ? tokens : undefined,
        costUsd: msgData.cost || undefined,
        toolCalls,
        metadata: {
          id: msgRow.id,
          parentId: msgData.parentID,
          provider: msgData.providerID,
          agent: msgData.agent,
        },
      });
    } catch (err) {
      console.error(`Skipping corrupted message ${msgRow.id}: ${err}`);
    }
  }

  return buildConversation(
    {
      id: session.id,
      version: session.version,
      slug: session.slug,
      title: session.title,
      parentID: session.parent_id ?? undefined,
      directory: session.directory,
      projectID: session.project_id,
      time: {
        created: session.time_created,
        updated: session.time_updated,
      },
      summary: (session.summary_additions !== null || session.summary_deletions !== null) ? {
        additions: session.summary_additions ?? undefined,
        deletions: session.summary_deletions ?? undefined,
        files: session.summary_files ?? undefined,
      } : undefined,
    },
    messages,
    session.directory || worktree,
    session.project_id,
  );
}

/** Map opencode tool status to hstry ToolStatus. */
function mapToolStatus(status?: string): 'pending' | 'success' | 'error' | undefined {
  switch (status) {
    case 'completed': return 'success';
    case 'error': return 'error';
    case 'pending':
    case 'running': return 'pending';
    default: return undefined;
  }
}

// ============================================================================
// JSON layout helpers (shared by new/old layouts and parseStream)
// ============================================================================

async function parseSingleSession(
  basePath: string,
  ref: SessionRef,
  opts?: ParseOptions,
): Promise<Conversation | null> {
  const sessionContent = await readFile(ref.path, 'utf-8');
  const session: SessionInfo = JSON.parse(sessionContent);

  // Apply time filter
  if (opts?.since) {
    const lastModified = session.time.updated ?? session.time.created;
    if (session.time.created < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  let messages: Message[];
  let workspace: string | undefined;

  if (ref.layout === 'new') {
    const storageDir = join(basePath, 'storage');
    const messageDir = join(storageDir, 'message');
    const partDir = join(storageDir, 'part');
    messages = await loadMessagesNew(session.id, messageDir, partDir, opts);
    workspace = session.directory;
  } else {
    const projectPath = join(basePath, 'project', ref.projectName!);
    const sessionMessageDir = join(projectPath, 'storage/session/message');
    const sessionPartDir = join(projectPath, 'storage/session/part');
    messages = await loadMessagesOld(session.id, sessionMessageDir, sessionPartDir, opts);
    workspace = session.directory || projectNameToPath(ref.projectName!);
  }

  return buildConversation(session, messages, workspace, ref.projectName);
}

/** Build a Conversation from a parsed session and its messages. */
function buildConversation(
  session: SessionInfo,
  messages: Message[],
  workspace: string | undefined,
  projectName?: string,
): Conversation {
  const conv: Conversation = {
    externalId: session.id,
    title: session.title || undefined,
    createdAt: session.time.created,
    updatedAt: session.time.updated,
    workspace,
    messages,
    metadata: {
      version: session.version,
      slug: session.slug,
      parentId: session.parentID,
      projectId: session.projectID ?? projectName,
    },
  };

  let tokensIn = 0;
  let tokensOut = 0;
  let cost = 0;
  let model: string | undefined;
  let provider: string | undefined;

  for (const msg of messages) {
    if (msg.tokens) {
      if (msg.role === 'user') tokensIn += msg.tokens;
      else tokensOut += msg.tokens;
    }
    if (msg.costUsd) cost += msg.costUsd;
    if (msg.model && !model) model = msg.model;
    if (!provider && msg.metadata && typeof msg.metadata.provider === 'string') {
      provider = msg.metadata.provider;
    }
  }

  if (tokensIn > 0) conv.tokensIn = tokensIn;
  if (tokensOut > 0) conv.tokensOut = tokensOut;
  if (cost > 0) conv.costUsd = cost;
  if (model) conv.model = model;
  if (provider) conv.provider = provider;

  return conv;
}

/**
 * Parse NEW layout (v1.1.25+):
 *   storage/session/<project-id>/<session-id>.json
 *   storage/message/<session-id>/<msg-id>.json
 *   storage/part/<msg-id>/<part-id>.json
 */
async function parseNewLayout(basePath: string, opts?: ParseOptions): Promise<Conversation[]> {
  const conversations: Conversation[] = [];
  const storageDir = join(basePath, 'storage');
  const sessionDir = join(storageDir, 'session');
  const messageDir = join(storageDir, 'message');
  const partDir = join(storageDir, 'part');

  try {
    // Iterate over project directories in storage/session/
    const projectIds = await readdir(sessionDir);

    for (const projectId of projectIds) {
      const projectPath = join(sessionDir, projectId);
      const projectStats = await stat(projectPath);
      if (!projectStats.isDirectory()) continue;

      try {
        const sessionFiles = await readdir(projectPath);

        for (const sessionFile of sessionFiles) {
          if (!sessionFile.startsWith('ses_') || !sessionFile.endsWith('.json')) {
            continue;
          }

          const sessionPath = join(projectPath, sessionFile);
          const sessionContent = await readFile(sessionPath, 'utf-8');
          const session: SessionInfo = JSON.parse(sessionContent);

          // Apply time filter
          if (opts?.since) {
            const lastModified = session.time.updated ?? session.time.created;
            if (session.time.created < opts.since && lastModified < opts.since) {
              continue;
            }
          }

          // Load messages for this session (new layout)
          const messages = await loadMessagesNew(session.id, messageDir, partDir, opts);

          const conv: Conversation = {
            externalId: session.id,
            title: session.title || undefined,
            createdAt: session.time.created,
            updatedAt: session.time.updated,
            workspace: session.directory,
            messages,
            metadata: {
              version: session.version,
              slug: session.slug,
              parentId: session.parentID,
              projectId: session.projectID,
            },
          };

          // Calculate totals
          let tokensIn = 0;
          let tokensOut = 0;
          let cost = 0;
          let model: string | undefined;
          let provider: string | undefined;

          for (const msg of messages) {
            if (msg.tokens) {
              if (msg.role === 'user') tokensIn += msg.tokens;
              else tokensOut += msg.tokens;
            }
            if (msg.costUsd) cost += msg.costUsd;
            if (msg.model && !model) model = msg.model;
            if (!provider && msg.metadata && typeof msg.metadata.provider === 'string') {
              provider = msg.metadata.provider;
            }
          }

          if (tokensIn > 0) conv.tokensIn = tokensIn;
          if (tokensOut > 0) conv.tokensOut = tokensOut;
          if (cost > 0) conv.costUsd = cost;
          if (model) conv.model = model;
          if (provider) conv.provider = provider;

          conversations.push(conv);

          if (opts?.limit && conversations.length >= opts.limit) {
            break;
          }
        }
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
          console.error(`Error reading project ${projectId}:`, err);
        }
        continue;
      }

      if (opts?.limit && conversations.length >= opts.limit) {
        break;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

/**
 * Load messages for NEW layout
 */
async function loadMessagesNew(
  sessionId: string,
  messageDir: string,
  partDir: string,
  opts?: ParseOptions
): Promise<Message[]> {
  const sessionMessagePath = join(messageDir, sessionId);
  const messages: Message[] = [];

  try {
    const messageFiles = await readdir(sessionMessagePath);

    for (const messageFile of messageFiles) {
      if (!messageFile.startsWith('msg_') || !messageFile.endsWith('.json')) {
        continue;
      }

      const messagePath = join(sessionMessagePath, messageFile);
      try {
        const messageContent = await readFile(messagePath, 'utf-8');
        if (!messageContent.trim()) continue; // skip empty files
        const msgInfo: MessageInfo = JSON.parse(messageContent);

        // Load parts - in new layout: part/<msg-id>/<part-id>.json
        const msgPartDir = join(partDir, msgInfo.id);
        const parts = await loadPartsNew(msgPartDir, opts);

        const textParts = parts.filter(p => p.type === 'text' && p.text);
        const content = textParts.map(p => p.text).join('\n');

        let toolCalls: ToolCall[] | undefined;
        if (opts?.includeTools !== false) {
          const toolParts = parts.filter(p => p.type === 'tool' && p.tool);
          if (toolParts.length > 0) {
            toolCalls = toolParts.map(p => ({
              toolName: p.tool!,
              input: p.state?.input,
              output: p.state?.output,
              status: p.state?.status as 'pending' | 'success' | 'error' | undefined,
            }));
          }
        }

        const tokens = (msgInfo.tokens?.input || 0) + (msgInfo.tokens?.output || 0) + (msgInfo.tokens?.reasoning || 0);
        const canonParts = buildCanonParts(parts, msgInfo.role, opts?.includeTools !== false) ?? textOnlyParts(content);

        messages.push({
          role: mapRole(msgInfo.role),
          content,
          parts: canonParts,
          createdAt: msgInfo.time.created,
          model: msgInfo.modelID || undefined,
          tokens: tokens > 0 ? tokens : undefined,
          costUsd: msgInfo.cost || undefined,
          toolCalls,
          metadata: {
            id: msgInfo.id,
            parentId: msgInfo.parentID,
            provider: msgInfo.providerID,
            agent: msgInfo.agent,
            summaryTitle: msgInfo.summary?.title,
          },
        });
      } catch (msgErr) {
        // Skip corrupted/empty message files
        console.error(`Skipping corrupted message ${messagePath}: ${msgErr}`);
        continue;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  messages.sort((a, b) => (a.createdAt || 0) - (b.createdAt || 0));
  return messages;
}

/**
 * Load parts for NEW layout
 */
async function loadPartsNew(msgPartDir: string, opts?: ParseOptions): Promise<PartInfo[]> {
  const parts: PartInfo[] = [];

  try {
    const partFiles = await readdir(msgPartDir);

    for (const partFile of partFiles) {
      if (!partFile.startsWith('prt_') || !partFile.endsWith('.json')) {
        continue;
      }

      const partPath = join(msgPartDir, partFile);
      try {
        const partContent = await readFile(partPath, 'utf-8');
        if (!partContent.trim()) continue; // skip empty files
        const part: PartInfo = JSON.parse(partContent);
        parts.push(part);
      } catch (partErr) {
        console.error(`Skipping corrupted part ${partPath}: ${partErr}`);
        continue;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  parts.sort((a, b) => a.id.localeCompare(b.id));
  return parts;
}

/**
 * Parse OLD layout (legacy):
 *   project/<project-name>/storage/session/info/<session-id>.json
 */
async function parseOldLayout(basePath: string, opts?: ParseOptions): Promise<Conversation[]> {
  const projectDir = join(basePath, 'project');
  const conversations: Conversation[] = [];

  try {
    const projects = await readdir(projectDir);

    for (const projectName of projects) {
      const projectPath = join(projectDir, projectName);
      const projectStats = await stat(projectPath);
      if (!projectStats.isDirectory()) continue;

      const sessionInfoDir = join(projectPath, 'storage/session/info');
      const sessionMessageDir = join(projectPath, 'storage/session/message');
      const sessionPartDir = join(projectPath, 'storage/session/part');

      const workspace = projectNameToPath(projectName);

      try {
        const sessionFiles = await readdir(sessionInfoDir);

        for (const sessionFile of sessionFiles) {
          if (!sessionFile.startsWith('ses_') || !sessionFile.endsWith('.json')) {
            continue;
          }

          const sessionPath = join(sessionInfoDir, sessionFile);
          const sessionContent = await readFile(sessionPath, 'utf-8');
          const session: SessionInfo = JSON.parse(sessionContent);

          if (opts?.since) {
            const lastModified = session.time.updated ?? session.time.created;
            if (session.time.created < opts.since && lastModified < opts.since) {
              continue;
            }
          }

          const messages = await loadMessagesOld(session.id, sessionMessageDir, sessionPartDir, opts);

          const conv: Conversation = {
            externalId: session.id,
            title: session.title || undefined,
            createdAt: session.time.created,
            updatedAt: session.time.updated,
            workspace: session.directory || workspace,
            messages,
            metadata: {
              version: session.version,
              parentId: session.parentID,
              projectId: projectName,
            },
          };

          let tokensIn = 0;
          let tokensOut = 0;
          let cost = 0;
          let model: string | undefined;
          let provider: string | undefined;

          for (const msg of messages) {
            if (msg.tokens) {
              if (msg.role === 'user') tokensIn += msg.tokens;
              else tokensOut += msg.tokens;
            }
            if (msg.costUsd) cost += msg.costUsd;
            if (msg.model && !model) model = msg.model;
            if (!provider && msg.metadata && typeof msg.metadata.provider === 'string') {
              provider = msg.metadata.provider;
            }
          }

          if (tokensIn > 0) conv.tokensIn = tokensIn;
          if (tokensOut > 0) conv.tokensOut = tokensOut;
          if (cost > 0) conv.costUsd = cost;
          if (model) conv.model = model;
          if (provider) conv.provider = provider;

          conversations.push(conv);

          if (opts?.limit && conversations.length >= opts.limit) {
            break;
          }
        }
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
          console.error(`Error reading project ${projectName}:`, err);
        }
        continue;
      }

      if (opts?.limit && conversations.length >= opts.limit) {
        break;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      return [];
    }
    throw err;
  }

  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

/**
 * Convert OpenCode project name to filesystem path
 * e.g., "home-wismut-byteowlz-kittenx" -> "/home/wismut/byteowlz/kittenx"
 */
function projectNameToPath(projectName: string): string {
  if (projectName === 'global') return '~';
  return '/' + projectName.replace(/-/g, '/');
}

/**
 * Load messages for OLD layout
 */
async function loadMessagesOld(
  sessionId: string,
  messageDir: string,
  partDir: string,
  opts?: ParseOptions
): Promise<Message[]> {
  const sessionMessagePath = join(messageDir, sessionId);
  const sessionPartPath = join(partDir, sessionId);
  const messages: Message[] = [];

  try {
    const messageFiles = await readdir(sessionMessagePath);

    for (const messageFile of messageFiles) {
      if (!messageFile.startsWith('msg_') || !messageFile.endsWith('.json')) {
        continue;
      }

      const messagePath = join(sessionMessagePath, messageFile);
      try {
        const messageContent = await readFile(messagePath, 'utf-8');
        if (!messageContent.trim()) continue; // skip empty files
        const msgInfo: MessageInfo = JSON.parse(messageContent);

        // Load parts for this message
        const parts = await loadPartsOld(sessionId, msgInfo.id, sessionPartPath, opts);

        // Combine text parts into content
        const textParts = parts.filter(p => p.type === 'text' && p.text);
        const content = textParts.map(p => p.text).join('\n');

        // Extract tool calls if requested
        let toolCalls: ToolCall[] | undefined;
        if (opts?.includeTools !== false) {
          const toolParts = parts.filter(p => p.type === 'tool' && p.tool);
          if (toolParts.length > 0) {
            toolCalls = toolParts.map(p => ({
              toolName: p.tool!,
              input: p.state?.input,
              output: p.state?.output,
              status: p.state?.status as 'pending' | 'success' | 'error' | undefined,
            }));
          }
        }

        const tokens = (msgInfo.tokens?.input || 0) + (msgInfo.tokens?.output || 0) + (msgInfo.tokens?.reasoning || 0);
        const canonParts = buildCanonParts(parts, msgInfo.role, opts?.includeTools !== false) ?? textOnlyParts(content);

        const msg: Message = {
          role: mapRole(msgInfo.role),
          content,
          parts: canonParts,
          createdAt: msgInfo.time.created,
          model: msgInfo.modelID || undefined,
          tokens: tokens > 0 ? tokens : undefined,
          costUsd: msgInfo.cost || undefined,
          toolCalls,
          metadata: {
            id: msgInfo.id,
            parentId: msgInfo.parentID,
            provider: msgInfo.providerID,
            agent: msgInfo.agent,
            summaryTitle: msgInfo.summary?.title,
          },
        };

        messages.push(msg);
      } catch (msgErr) {
        // Skip corrupted/empty message files
        console.error(`Skipping corrupted message ${messagePath}: ${msgErr}`);
        continue;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  // Sort by created_at ascending (chronological)
  messages.sort((a, b) => (a.createdAt || 0) - (b.createdAt || 0));

  return messages;
}

async function loadPartsOld(
  sessionId: string,
  messageId: string,
  sessionPartPath: string,
  opts?: ParseOptions
): Promise<PartInfo[]> {
  // Parts are in: part/<session-id>/<msg-id>/<part-id>.json
  const messagePartDir = join(sessionPartPath, messageId);
  const parts: PartInfo[] = [];

  try {
    const partFiles = await readdir(messagePartDir);

    for (const partFile of partFiles) {
      if (!partFile.startsWith('prt_') || !partFile.endsWith('.json')) {
        continue;
      }

      const partPath = join(messagePartDir, partFile);
      try {
        const partContent = await readFile(partPath, 'utf-8');
        if (!partContent.trim()) continue; // skip empty files
        const part: PartInfo = JSON.parse(partContent);
        parts.push(part);
      } catch (partErr) {
        console.error(`Skipping corrupted part ${partPath}: ${partErr}`);
        continue;
      }
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  // Sort by ID (roughly chronological)
  parts.sort((a, b) => a.id.localeCompare(b.id));

  return parts;
}

/** Build CanonPart[] from opencode PartInfo[]. */
function buildCanonParts(oparts: PartInfo[], role: string, includeTools: boolean): CanonPart[] | undefined {
  const canon: CanonPart[] = [];
  for (const p of oparts) {
    if (p.type === 'text' && p.text) {
      canon.push(textPart(p.text));
    } else if (p.type === 'tool' && p.tool && includeTools) {
      // Tool parts in opencode represent both the call and result.
      // The state has input, output, status.
      const callId = p.id || p.tool;
      canon.push(toolCallPart(callId, p.tool, p.state?.input));
      if (p.state?.output !== undefined || p.state?.status === 'error') {
        canon.push(toolResultPart(callId, p.state?.output, { name: p.tool, isError: p.state?.status === 'error' }));
      }
    }
  }
  return canon.length > 0 ? canon : undefined;
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
    case 'human':
      return 'user';
    case 'assistant':
    case 'agent':
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

function buildOpenCodeFiles(conversations: Conversation[]): { path: string; content: string }[] {
  const files: { path: string; content: string }[] = [];

  for (const conv of conversations) {
    const sessionId = conv.externalId ?? randomId('ses_');
    const projectName = workspaceToProjectName(conv.workspace);
    const basePath = `project/${projectName}/storage/session`;
    const infoDir = `${basePath}/info`;
    const messageDir = `${basePath}/message/${sessionId}`;
    const partDir = `${basePath}/part/${sessionId}`;

    const sessionInfo = {
      id: sessionId,
      version: '1',
      title: conv.title ?? null,
      parentID: null,
      directory: conv.workspace ?? null,
      projectID: projectName,
      time: {
        created: conv.createdAt,
        updated: conv.updatedAt ?? conv.createdAt,
      },
    };
    files.push({
      path: `${infoDir}/${sessionId}.json`,
      content: JSON.stringify(sessionInfo, null, 2),
    });

    conv.messages.forEach((msg, idx) => {
      const msgId = `msg_${idx + 1}`;
      const created = msg.createdAt ?? conv.createdAt;
      const messageInfo = {
        id: msgId,
        sessionID: sessionId,
        role: msg.role,
        time: {
          created,
        },
        modelID: msg.model ?? null,
        providerID: null,
        agent: null,
        tokens: msg.tokens ? { input: msg.tokens, output: 0 } : undefined,
        cost: msg.costUsd ?? undefined,
      };
      files.push({
        path: `${messageDir}/${msgId}.json`,
        content: JSON.stringify(messageInfo, null, 2),
      });

      const partEntries: PartInfo[] = [];
      if (msg.content) {
        partEntries.push({
          id: `prt_${idx + 1}_text`,
          messageID: msgId,
          sessionID: sessionId,
          type: 'text',
          text: msg.content,
        });
      }

      if (msg.toolCalls) {
        msg.toolCalls.forEach((tool, toolIdx) => {
          partEntries.push({
            id: `prt_${idx + 1}_tool_${toolIdx + 1}`,
            messageID: msgId,
            sessionID: sessionId,
            type: 'tool',
            tool: tool.toolName,
            state: {
              status: tool.status,
              input: tool.input,
              output: tool.output,
            },
          });
        });
      }

      partEntries.forEach(part => {
        files.push({
          path: `${partDir}/${msgId}/${part.id}.json`,
          content: JSON.stringify(part, null, 2),
        });
      });
    });
  }

  return files;
}

function workspaceToProjectName(workspace?: string): string {
  if (!workspace) return 'global';
  return workspace.replace(/^\/+/, '').replace(/[\\/]/g, '-');
}

function randomId(prefix: string): string {
  const uuid =
    typeof globalThis.crypto?.randomUUID === 'function'
      ? globalThis.crypto.randomUUID()
      : `${Date.now()}${Math.random().toString(16).slice(2)}`;
  return `${prefix}${uuid.replace(/-/g, '').slice(0, 12)}`;
}

// Run the adapter
runAdapter(adapter);
