/**
 * Hermes Agent adapter for hstry.
 *
 * Three sources, all normalized to the same session/message shape:
 *   1. ~/.hermes/state.db  — the canonical SQLite store (sessions/messages).
 *   2. `hermes sessions export --format jsonl` — one session object per line
 *      (content-detected; the export mirrors the state.db columns).
 *   3. ~/.hermes/sessions/session_<id>.json — the legacy JSON snapshot files.
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  CanonPart,
  Conversation,
  Message,
  ParseOptions,
  ToolCall,
} from '../types/index.ts';
import {
  runAdapter,
  textOnlyParts,
  textPart,
  thinkingPart,
  toolCallPart,
  toolResultPart,
  isUnderAnyCanonicalRoot,
} from '../types/index.ts';
import { findFirstRealUserMessage, formatFrumTitle } from '../types/first-message.ts';

const HERMES_HOME = join(process.env.HERMES_HOME || join(homedir(), '.hermes'));
const SESSIONS_DIR = join(HERMES_HOME, 'sessions');
const STATE_DB = join(HERMES_HOME, 'state.db');
const ROOTS = [SESSIONS_DIR, STATE_DB];

// Dynamic SQLite support: bun:sqlite (Bun) or better-sqlite3 (Node).
interface SqliteDb {
  prepare(sql: string): SqliteStatement;
  close(): void;
}
interface SqliteStatement {
  all(...params: unknown[]): unknown[];
  get(...params: unknown[]): unknown;
}

let openDb: ((path: string) => SqliteDb) | null = null;
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
  // SQLite unavailable: state.db is skipped, JSON snapshots still work.
}

declare const Bun: unknown;

/** Either the legacy JSON snapshot or a state.db/export session row. */
interface HermesSession {
  id?: string;
  session_id?: string;
  source?: string;
  model?: string;
  base_url?: string;
  billing_base_url?: string;
  platform?: string | null;
  session_start?: string;
  last_updated?: string;
  started_at?: number;
  last_active?: number;
  ended_at?: number | null;
  system_prompt?: string;
  parent_session_id?: string | null;
  title?: string | null;
  cwd?: string | null;
  message_count?: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  reasoning_tokens?: number;
  cost?: number;
  estimated_cost_usd?: number;
  actual_cost_usd?: number;
  messages?: HermesMessage[];
}

interface HermesMessage {
  role?: string;
  content?: unknown;
  reasoning?: string | null;
  reasoning_content?: string | null;
  finish_reason?: string;
  timestamp?: string | number;
  tool_call_id?: string | null;
  tool_name?: string | null;
  tool_calls?: string | HermesToolCall[] | null;
  active?: number;
  _compressed_summary?: number;
}

interface HermesToolCall {
  id?: string;
  call_id?: string;
  type?: string;
  function?: { name?: string; arguments?: string } | null;
  name?: string;
  arguments?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return { name: 'hermes', displayName: 'Hermes Agent', version: '1.1.0', defaultPaths: ROOTS };
  },

  async detect(path: string): Promise<number | null> {
    // `hermes sessions export` JSONL is content-detected (users write it
    // anywhere), so it is the one location-independent case.
    if (path.endsWith('.jsonl')) {
      return (await looksLikeExport(path)) ? 0.95 : null;
    }
    if (!isUnderAnyCanonicalRoot(path, ROOTS)) return null;
    if (await hasStateDb(path)) return 0.95;
    return (await findSessionFiles(path)).length > 0 ? 0.9 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const conversations: Conversation[] = [];

    if (path.endsWith('.jsonl')) {
      const exported = await parseExportFile(path, opts);
      if (exported) conversations.push(...exported);
      return conversations.sort((a, b) => b.createdAt - a.createdAt);
    }

    const dbFile = await stateDbPath(path);
    if (dbFile) {
      conversations.push(...(await parseStateDb(dbFile, opts)));
    }

    for (const filePath of await findSessionFiles(path)) {
      const conv = await parseSessionFile(filePath, opts);
      if (conv) conversations.push(conv);
      if (opts?.limit && conversations.length >= opts.limit) break;
    }

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return opts?.limit ? conversations.slice(0, opts.limit) : conversations;
  },
};

// ============================================================================
// Shared session -> Conversation builder
// ============================================================================

function buildConversation(session: HermesSession, filePath?: string): Conversation | null {
  const sessionId = session.session_id ?? session.id ?? (filePath ? basename(filePath, '.json').replace(/^session_/, '') : undefined);
  if (!sessionId) return null;

  const createdAt = sessionStart(session) ?? Date.now();
  const updatedAt = sessionUpdated(session);
  const model = session.model || undefined;
  const baseUrl = session.base_url || session.billing_base_url;

  const messages: Message[] = [];
  const pendingToolCalls = new Map<string, ToolCall>();

  for (const raw of session.messages ?? []) {
    // state.db keeps superseded/compressed rows; they are not the live transcript.
    if (raw.active === 0 || raw._compressed_summary === 1) continue;

    const role = mapRole(raw.role);
    const content = typeof raw.content === 'string' ? raw.content : contentToText(raw.content);
    const timestamp = messageTimestamp(raw.timestamp) ?? createdAt;
    const parts: CanonPart[] = [];

    for (const reasoning of [raw.reasoning, raw.reasoning_content]) {
      if (reasoning && reasoning.trim()) parts.push(thinkingPart(reasoning));
    }
    if (content.trim()) parts.push(textPart(content));

    const toolCalls = Array.isArray(raw.tool_calls) ? raw.tool_calls : safeParseJson(raw.tool_calls);
    const messageToolCalls: ToolCall[] = [];
    if (Array.isArray(toolCalls)) {
      for (const tc of toolCalls as HermesToolCall[]) {
        const callId = tc.id ?? tc.call_id ?? cryptoRandom('tc');
        const name = tc.function?.name ?? tc.name ?? tc.type ?? 'tool';
        const input = safeParseJson(tc.function?.arguments ?? tc.arguments);
        parts.push(toolCallPart(callId, name, input));
        const call: ToolCall = { toolName: name, input, status: 'pending' };
        pendingToolCalls.set(callId, call);
        messageToolCalls.push(call);
      }
    }

    if (role === 'tool') {
      const callId = raw.tool_call_id ?? '';
      const name = raw.tool_name ?? pendingToolCalls.get(callId)?.toolName ?? 'tool';
      const pending = pendingToolCalls.get(callId);
      if (pending) {
        pending.output = content;
        pending.status = 'success';
      }
      parts.push(toolResultPart(callId, content, { name }));
      pendingToolCalls.delete(callId);
    }

    messages.push({
      role,
      content,
      parts: parts.length ? parts : textOnlyParts(content),
      createdAt: timestamp,
      model,
      toolCalls: messageToolCalls.length ? messageToolCalls : undefined,
    });
  }

  if (messages.length === 0) return null;

  const frum = findFirstRealUserMessage(messages.map(m => ({ role: m.role, content: m.content })));
  const title = session.title?.trim() || (frum ? formatFrumTitle(frum) : undefined);
  const tokensIn = (session.input_tokens ?? 0) + (session.cache_read_tokens ?? 0);

  const conv: Conversation = {
    externalId: sessionId,
    title,
    createdAt,
    updatedAt,
    model,
    provider: baseUrl ? deriveProvider(baseUrl) : undefined,
    workspace: session.cwd ?? undefined,
    tokensIn: tokensIn > 0 ? tokensIn : undefined,
    tokensOut: (session.output_tokens ?? 0) > 0 ? session.output_tokens : undefined,
    costUsd: session.actual_cost_usd ?? session.estimated_cost_usd ?? session.cost ?? undefined,
    messages,
    metadata: {
      ...(filePath ? { file: filePath } : {}),
      platform: session.platform ?? undefined,
      source: session.source,
      baseUrl,
      messageCount: session.message_count,
      parentSessionId: session.parent_session_id ?? undefined,
    },
  };
  if (session.parent_session_id) {
    conv.parentExternalId = session.parent_session_id;
    conv.forkType = 'thread';
  }
  return conv;
}

function sessionStart(session: HermesSession): number | undefined {
  return parseTimestamp(session.session_start) ?? secondsToMs(session.started_at);
}

function sessionUpdated(session: HermesSession): number | undefined {
  return parseTimestamp(session.last_updated) ?? secondsToMs(session.last_active);
}

// ============================================================================
// state.db (canonical SQLite store)
// ============================================================================

async function stateDbPath(path: string): Promise<string | null> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return null;
  if (stats.isFile()) return path.endsWith('.db') ? path : null;
  if (stats.isDirectory()) {
    const candidate = join(path, 'state.db');
    return (await stat(candidate).catch(() => null)) ? candidate : null;
  }
  return null;
}

async function hasStateDb(path: string): Promise<boolean> {
  if (!openDb) return false;
  const dbFile = await stateDbPath(path);
  if (!dbFile) return false;
  try {
    const db = openDb(dbFile);
    try {
      return Boolean(db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'").get());
    } finally {
      db.close();
    }
  } catch {
    return false;
  }
}

async function parseStateDb(dbPath: string, opts?: ParseOptions): Promise<Conversation[]> {
  if (!openDb) return [];
  const db = openDb(dbPath);
  try {
    const sessions = db.prepare('SELECT * FROM sessions').all() as Record<string, unknown>[];
    const conversations: Conversation[] = [];
    for (const session of sessions) {
      if (Number(session.hidden ?? 0) === 1) continue;
      const id = String(session.id ?? '');
      if (!id) continue;
      if (opts?.since && (secondsToMs(session.last_activity_at as number) ?? 0) < opts.since && (secondsToMs(session.started_at as number) ?? 0) < opts.since) continue;
      let rows: Record<string, unknown>[] = [];
      try {
        rows = db.prepare('SELECT * FROM messages WHERE session_id = ? ORDER BY id').all(id) as Record<string, unknown>[];
      } catch {
        rows = [];
      }
      const conv = buildConversation({ ...(session as unknown as HermesSession), messages: rows as HermesMessage[] });
      if (conv) conversations.push(conv);
      if (opts?.limit && conversations.length >= opts.limit) break;
    }
    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return conversations;
  } finally {
    db.close();
  }
}

// ============================================================================
// `hermes sessions export --format jsonl` (one session per line)
// ============================================================================

async function looksLikeExport(path: string): Promise<boolean> {
  const raw = await readFile(path, 'utf-8').catch(() => null);
  if (!raw) return false;
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const value = JSON.parse(line) as Record<string, unknown>;
      return Boolean(value && typeof value === 'object' && ('id' in value || 'session_id' in value) && Array.isArray(value.messages) && ('source' in value || 'started_at' in value));
    } catch {
      return false;
    }
  }
  return false;
}

async function parseExportFile(filePath: string, opts?: ParseOptions): Promise<Conversation[]> {
  const raw = await readFile(filePath, 'utf-8').catch(() => null);
  if (!raw) return [];
  const conversations: Conversation[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const session = JSON.parse(line) as HermesSession;
      const conv = buildConversation(session);
      if (conv) conversations.push(conv);
    } catch {
      // A malformed line does not invalidate the rest of the export.
    }
    if (opts?.limit && conversations.length >= opts.limit) break;
  }
  return conversations;
}

// ============================================================================
// Legacy JSON snapshot files
// ============================================================================

async function parseSessionFile(filePath: string, opts?: ParseOptions): Promise<Conversation | null> {
  const raw = await readFile(filePath, 'utf-8').catch(() => null);
  if (!raw) return null;

  let session: HermesSession;
  try {
    session = JSON.parse(raw) as HermesSession;
  } catch {
    return null;
  }

  const conv = buildConversation(session, filePath);
  if (!conv) return null;
  if (opts?.since) {
    const lastModified = conv.updatedAt ?? conv.createdAt;
    if (conv.createdAt < opts.since && lastModified < opts.since) return null;
  }
  return conv;
}

async function findSessionFiles(path: string): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    return path.endsWith('.json') && basename(path).startsWith('session_') ? [path] : [];
  }
  if (!stats.isDirectory()) return [];

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  const files: string[] = [];
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    if (!entry.name.startsWith('session_')) continue;
    if (!entry.name.endsWith('.json')) continue;
    files.push(join(path, entry.name));
  }
  return files;
}

// ============================================================================
// Helpers
// ============================================================================

function deriveProvider(baseUrl: string): string | undefined {
  try {
    return new URL(baseUrl).host;
  } catch {
    return undefined;
  }
}

function contentToText(content: unknown): string {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content
    .map(part => (typeof part === 'string' ? part : (part?.text ?? part?.content ?? '')))
    .filter(Boolean)
    .join('\n')
    .trim();
}

function safeParseJson(value?: string | null): unknown {
  if (!value) return undefined;
  if (typeof value !== 'string') return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function secondsToMs(value?: number | null): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value)) return undefined;
  return Math.floor(value < 1e12 ? value * 1000 : value);
}

function messageTimestamp(value?: string | number): number | undefined {
  return parseTimestamp(value);
}

function parseTimestamp(value?: string | number | null): number | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value === 'number') return secondsToMs(value);
  // Hermes writes naive ISO timestamps without timezone; coerce to UTC.
  const normalized = /[zZ]|[+-]\d{2}:?\d{2}$/.test(value) ? value : `${value}Z`;
  const parsed = Date.parse(normalized);
  return Number.isNaN(parsed) ? undefined : parsed;
}

function mapRole(role?: string): Message['role'] {
  switch ((role ?? '').toLowerCase()) {
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

function cryptoRandom(prefix: string): string {
  if (typeof globalThis.crypto?.randomUUID === 'function') {
    return `${prefix}-${globalThis.crypto.randomUUID().replace(/-/g, '').slice(0, 12)}`;
  }
  return `${prefix}-${Date.now()}${Math.random().toString(16).slice(2, 8)}`;
}

runAdapter(adapter);
