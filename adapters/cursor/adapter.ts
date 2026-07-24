/**
 * Cursor adapter for hstry
 *
 * Full Composer session import (ported from am-history-importer):
 * - globalStorage/state.vscdb: composer.composerHeaders, composerData:{id}, bubbleId:{id}:*
 * - ItemTable + cursorDiskKV
 * - ~/.cursaves/snapshots/*.json.gz
 *
 * Legacy fallbacks: workbench chat tabs, aiService.prompts (workspace-only)
 */

import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from 'fs';
import { readdir, stat } from 'fs/promises';
import { basename, dirname, join } from 'path';
import { homedir, tmpdir } from 'os';
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

let Database: typeof import('better-sqlite3') | null = null;
try {
  Database = (await import('better-sqlite3')).default;
} catch {
  // SQLite not available
}

const MAX_TEXT = 20_000;
const BUBBLE_USER = 1;
const BUBBLE_ASSISTANT = 2;

const CHAT_DATA_KEY = 'workbench.panel.aichat.view.aichat.chatdata';
const PROMPTS_KEY = 'aiService.prompts';
const GENERATIONS_KEY = 'aiService.generations';
const COMPOSER_HEADERS_KEY = 'composer.composerHeaders';

function cursorRoots(): string[] {
  const home = homedir();
  const appData = process.env.APPDATA || join(home, 'AppData', 'Roaming');
  const localAppData = process.env.LOCALAPPDATA || join(home, 'AppData', 'Local');
  const roots = [
    join(home, '.cursaves', 'snapshots'),
    join(appData, 'Cursor', 'User', 'globalStorage'),
    join(appData, 'Cursor', 'User', 'workspaceStorage'),
  ];
  if (process.platform === 'win32') {
    roots.push(
      join(localAppData, 'Cursor', 'User', 'globalStorage'),
      join(localAppData, 'Cursor', 'User', 'workspaceStorage'),
    );
  } else if (process.platform === 'darwin') {
    roots.push(
      join(home, 'Library', 'Application Support', 'Cursor', 'User', 'globalStorage'),
      join(home, 'Library', 'Application Support', 'Cursor', 'User', 'workspaceStorage'),
    );
  } else {
    roots.push(
      join(home, '.config', 'Cursor', 'User', 'globalStorage'),
      join(home, '.config', 'Cursor', 'User', 'workspaceStorage'),
    );
  }
  return [...new Set(roots)];
}

const DEFAULT_PATHS = cursorRoots();

interface BubbleHeader {
  bubbleId?: string;
  type?: number;
}

interface BubbleBody {
  type?: number;
  text?: string;
  richText?: string;
  createdAt?: string | number;
  toolResults?: unknown;
  toolFormerData?: unknown;
}

interface ComposerSessionData {
  name?: string;
  createdAt?: number | string;
  lastUpdatedAt?: number | string;
  fullConversationHeadersOnly?: BubbleHeader[];
  conversationMap?: Record<string, BubbleBody>;
}

interface ComposerHeaderEntry {
  composerId?: string;
  name?: string;
  subtitle?: string;
  createdAt?: number;
  lastUpdatedAt?: number;
  workspaceIdentifier?: {
    uri?: { fsPath?: string; path?: string };
  };
}

interface SnapshotFile {
  version?: number;
  composerId?: string;
  sourceProjectPath?: string;
  projectIdentifier?: string;
  composerData?: ComposerSessionData;
  bubbleEntries?: Record<string, BubbleBody>;
}

interface CursorPromptLegacy {
  prompt?: string;
  response?: string;
  createdAt?: number;
  conversationId?: string;
  model?: string;
}

interface CursorPromptModern {
  text?: string;
  commandType?: number;
}

type CursorPromptEntry = CursorPromptLegacy & CursorPromptModern;

class SqliteKv {
  private db: InstanceType<NonNullable<typeof Database>>;
  private tmpDir: string | null = null;

  constructor(dbPath: string) {
    if (!Database) throw new Error('better-sqlite3 not available');
    try {
      this.db = new Database(dbPath, { readonly: true });
      this.db.prepare('SELECT 1').get();
    } catch {
      const dir = join(tmpdir(), `hstry-cursor-${Date.now()}-${Math.random().toString(16).slice(2)}`);
      mkdirSync(dir, { recursive: true });
      this.tmpDir = dir;
      const tmpDb = join(dir, 'state.vscdb');
      copyFileSync(dbPath, tmpDb);
      for (const suffix of ['-wal', '-shm']) {
        const side = dbPath + suffix;
        if (existsSync(side)) copyFileSync(side, tmpDb + suffix);
      }
      this.db = new Database(tmpDb);
      try {
        this.db.exec('PRAGMA wal_checkpoint(TRUNCATE)');
      } catch {
        /* ignore */
      }
    }
  }

  getItem(key: string, table = 'ItemTable'): string | null {
    try {
      const row = this.db
        .prepare(`SELECT value FROM ${table} WHERE key = ?`)
        .get(key) as { value: string | Buffer | null } | undefined;
      if (!row || row.value == null) return null;
      if (typeof row.value === 'string') return row.value;
      return Buffer.from(row.value).toString('utf8');
    } catch {
      return null;
    }
  }

  getJson<T>(key: string, table = 'ItemTable'): T | null {
    const raw = this.getItem(key, table) ?? this.getItem(key, 'cursorDiskKV');
    if (!raw) return null;
    try {
      return JSON.parse(raw) as T;
    } catch {
      return null;
    }
  }

  listKeys(prefix: string, table = 'cursorDiskKV'): string[] {
    try {
      const rows = this.db
        .prepare(`SELECT key FROM ${table} WHERE key LIKE ?`)
        .all(`${prefix}%`) as Array<{ key: string }>;
      return rows.map((r) => r.key);
    } catch {
      return [];
    }
  }

  hasComposerData(): boolean {
    if (this.getJson(COMPOSER_HEADERS_KEY)) return true;
    const prefixes = [
      ...this.listKeys('composerData:', 'cursorDiskKV'),
      ...this.listKeys('composerData:', 'ItemTable'),
      ...this.listKeys('bubbleId:', 'cursorDiskKV'),
      ...this.listKeys('bubbleId:', 'ItemTable'),
    ];
    return prefixes.length > 0;
  }

  close(): void {
    try {
      this.db.close();
    } catch {
      /* ignore */
    }
    if (this.tmpDir) {
      rmSync(this.tmpDir, { recursive: true, force: true });
    }
  }
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}

function toMs(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
    return value < 1e12 ? value * 1000 : value;
  }
  if (typeof value === 'string') {
    if (value.includes('T')) {
      const parsed = Date.parse(value);
      if (Number.isFinite(parsed)) return parsed;
    }
    const n = Number(value);
    if (Number.isFinite(n) && n > 0) return n < 1e12 ? n * 1000 : n;
  }
  return undefined;
}

function bubbleText(b: BubbleBody | undefined): string {
  if (!b) return '';
  if (typeof b.text === 'string' && b.text.trim()) return b.text.trim();
  if (typeof b.richText === 'string' && b.richText.trim()) return b.richText.trim();
  return '';
}

function extractToolCalls(body: BubbleBody | undefined): ToolCall[] {
  if (!body?.toolResults) return [];
  const tools = Array.isArray(body.toolResults) ? body.toolResults : [body.toolResults];
  const out: ToolCall[] = [];
  for (const tool of tools) {
    if (!tool || typeof tool !== 'object') continue;
    const t = tool as Record<string, unknown>;
    const name =
      (typeof t.name === 'string' && t.name) ||
      (typeof t.toolName === 'string' && t.toolName) ||
      'tool';
    const output =
      typeof t.result === 'string'
        ? t.result
        : JSON.stringify(t.result ?? t.output ?? '');
    out.push({
      toolName: name,
      input: t.params ?? t.input,
      output: truncate(output, 8000),
      status: 'success',
    });
  }
  return out;
}

function sessionFromComposer(opts: {
  composerId: string;
  composerData: ComposerSessionData;
  bubbles: Record<string, BubbleBody>;
  projectPath?: string;
  sourcePath: string;
  opts?: ParseOptions;
}): Conversation | null {
  const headers = opts.composerData.fullConversationHeadersOnly || [];
  const messages: Message[] = [];
  let firstTime: number | undefined;
  let lastTime: number | undefined;

  const pushBubble = (header: BubbleHeader, body: BubbleBody | undefined) => {
    const text = truncate(bubbleText(body), MAX_TEXT);
    if (!text) return;
    const createdAt = toMs(body?.createdAt) ?? toMs(opts.composerData.createdAt);
    if (createdAt) {
      if (!firstTime || createdAt < firstTime) firstTime = createdAt;
      if (!lastTime || createdAt > lastTime) lastTime = createdAt;
    }
    const type = header.type ?? body?.type;
    if (type === BUBBLE_USER) {
      messages.push({
        role: 'user',
        content: text,
        parts: textOnlyParts(text),
        createdAt,
      });
    } else if (type === BUBBLE_ASSISTANT) {
      const toolCalls = extractToolCalls(body);
      messages.push({
        role: 'assistant',
        content: text,
        parts: textOnlyParts(text),
        createdAt,
        toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
      });
      for (const tc of toolCalls) {
        messages.push({
          role: 'tool',
          content: tc.output || '',
          parts: textOnlyParts(tc.output || ''),
          createdAt,
          metadata: { toolName: tc.toolName },
        });
      }
    }
  };

  if (headers.length > 0) {
    for (const header of headers) {
      const id = header.bubbleId;
      if (!id) continue;
      const body = opts.bubbles[id] || opts.composerData.conversationMap?.[id];
      pushBubble(header, body);
    }
  } else {
    const map = opts.composerData.conversationMap || opts.bubbles;
    for (const [id, body] of Object.entries(map)) {
      pushBubble({ bubbleId: id, type: body.type }, body);
    }
  }

  if (messages.length === 0) return null;

  const createdAt = firstTime ?? toMs(opts.composerData.createdAt) ?? Date.now();
  const updatedAt = lastTime ?? toMs(opts.composerData.lastUpdatedAt) ?? createdAt;

  if (opts.opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.opts.since && lastModified < opts.opts.since) return null;
  }

  const workspace = opts.projectPath || basename(opts.sourcePath);
  return {
    externalId: opts.composerId,
    title: opts.composerData.name || `cursor:${opts.composerId.slice(0, 8)}`,
    createdAt,
    updatedAt,
    workspace,
    provider: 'cursor',
    messages,
    metadata: {
      source: 'cursor-composer',
      composerId: opts.composerId,
      sourcePath: opts.sourcePath,
    },
  };
}

function loadFromGlobalDb(dbPath: string, opts?: ParseOptions): Conversation[] {
  const db = new SqliteKv(dbPath);
  const conversations: Conversation[] = [];
  try {
    const headers =
      db.getJson<{ allComposers?: ComposerHeaderEntry[] }>(COMPOSER_HEADERS_KEY)?.allComposers ||
      [];

    const composerIds = new Set<string>();
    for (const h of headers) {
      if (typeof h.composerId === 'string') composerIds.add(h.composerId);
    }

    for (const key of [
      ...db.listKeys('composerData:', 'cursorDiskKV'),
      ...db.listKeys('composerData:', 'ItemTable'),
    ]) {
      const id = key.slice('composerData:'.length);
      if (id) composerIds.add(id);
    }

    for (const composerId of composerIds) {
      const composerData =
        db.getJson<ComposerSessionData>(`composerData:${composerId}`, 'cursorDiskKV') ||
        db.getJson<ComposerSessionData>(`composerData:${composerId}`, 'ItemTable');
      if (!composerData) continue;

      const bubbles: Record<string, BubbleBody> = { ...(composerData.conversationMap || {}) };
      for (const key of [
        ...db.listKeys(`bubbleId:${composerId}:`, 'cursorDiskKV'),
        ...db.listKeys(`bubbleId:${composerId}:`, 'ItemTable'),
      ]) {
        const bubbleId = key.slice(`bubbleId:${composerId}:`.length);
        const body =
          db.getJson<BubbleBody>(key, 'cursorDiskKV') || db.getJson<BubbleBody>(key, 'ItemTable');
        if (body) bubbles[bubbleId] = body;
      }

      const headerEntry = headers.find((h) => h.composerId === composerId);
      const projectPath =
        headerEntry?.workspaceIdentifier?.uri?.fsPath ||
        headerEntry?.workspaceIdentifier?.uri?.path;

      const session = sessionFromComposer({
        composerId,
        composerData: {
          ...composerData,
          name:
            composerData.name ||
            (typeof headerEntry?.name === 'string' ? headerEntry.name : undefined),
        },
        bubbles,
        projectPath,
        sourcePath: `${dbPath}#${composerId}`,
        opts,
      });
      if (session) conversations.push(session);
    }
  } finally {
    db.close();
  }
  return conversations;
}

function parseSnapshotFile(path: string, opts?: ParseOptions): Conversation | null {
  const raw = path.endsWith('.gz') ? gunzipSync(readFileSync(path)) : readFileSync(path);
  const data = JSON.parse(raw.toString('utf8')) as SnapshotFile;
  const composerId =
    data.composerId ||
    basename(path)
      .replace(/\.json(\.gz)?$/i, '')
      .replace(/\.\d+$/, '');
  return sessionFromComposer({
    composerId,
    composerData: data.composerData || {},
    bubbles: data.bubbleEntries || {},
    projectPath: data.sourceProjectPath,
    sourcePath: path,
    opts,
  });
}

function walkFiles(
  root: string,
  pred: (name: string, full: string) => boolean,
  out: string[] = [],
): string[] {
  if (!existsSync(root)) return out;
  let st;
  try {
    st = statSync(root);
  } catch {
    return out;
  }
  if (st.isFile()) {
    if (pred(basename(root), root)) out.push(root);
    return out;
  }
  let entries;
  try {
    entries = readdirSync(root, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const full = join(root, entry.name);
    if (entry.isDirectory()) walkFiles(full, pred, out);
    else if (pred(entry.name, full)) out.push(full);
  }
  return out;
}

function findGlobalDb(roots: string[]): string | null {
  for (const root of roots) {
    if (root.endsWith('state.vscdb') && existsSync(root)) return root;
    const candidate = join(root, 'state.vscdb');
    if (existsSync(candidate)) return candidate;
    if (basename(root) === 'globalStorage') {
      const p = join(root, 'state.vscdb');
      if (existsSync(p)) return p;
    }
  }
  return null;
}

function expandScanRoots(inputPath: string): string[] {
  return [...new Set([inputPath, ...DEFAULT_PATHS])];
}

function loadAllCursorSessions(inputPath: string, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];
  const seen = new Set<string>();
  const limit = opts?.limit && opts.limit > 0 ? opts.limit : 0;

  const add = (conv: Conversation | null): boolean => {
    if (!conv) return false;
    const id = conv.externalId ?? `${conv.createdAt}-${conv.title}`;
    if (seen.has(id)) return false;
    seen.add(id);
    conversations.push(conv);
    return true;
  };

  const full = (): boolean => limit > 0 && conversations.length >= limit;
  const roots = expandScanRoots(inputPath);

  for (const root of roots) {
    if (!existsSync(root) || full()) continue;

    const snapshots = walkFiles(
      root,
      (name) => name.endsWith('.json.gz') || (name.endsWith('.json') && !name.endsWith('.meta.json')),
    )
      .filter((p) => !p.endsWith('.meta.json'))
      .sort((a, b) => {
        try {
          return statSync(b).mtimeMs - statSync(a).mtimeMs;
        } catch {
          return 0;
        }
      });

    for (const snap of snapshots) {
      if (full()) break;
      try {
        add(parseSnapshotFile(snap, opts));
      } catch {
        /* skip */
      }
    }

    if (full()) continue;

    if (root.endsWith('state.vscdb') || basename(root) === 'state.vscdb') {
      for (const conv of loadFromGlobalDb(root, opts)) {
        add(conv);
        if (full()) break;
      }
    } else {
      const globalDb = findGlobalDb([root]);
      if (globalDb) {
        for (const conv of loadFromGlobalDb(globalDb, opts)) {
          add(conv);
          if (full()) break;
        }
      }
      if (full()) continue;

      const wsDbs = walkFiles(
        root,
        (name, fullPath) => name === 'state.vscdb' && fullPath.includes('workspaceStorage'),
      );
      for (const wsDb of wsDbs) {
        if (full()) break;
        try {
          for (const conv of loadFromGlobalDb(wsDb, opts)) {
            add(conv);
            if (full()) break;
          }
        } catch {
          /* ignore */
        }
      }
    }
  }

  // Fallback: workspace prompt logs when no composer sessions found
  if (conversations.length === 0) {
    for (const root of roots) {
      if (full()) break;
      const wsDbs = walkFiles(
        root,
        (name, fullPath) => name === 'state.vscdb' && fullPath.includes('workspaceStorage'),
      );
      for (const wsDb of wsDbs) {
        for (const conv of parseWorkspacePromptsFallback(wsDb, opts)) {
          add(conv);
          if (full()) break;
        }
      }
    }
  }

  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return limit > 0 ? conversations.slice(0, limit) : conversations;
}

function parseWorkspacePromptsFallback(dbPath: string, opts?: ParseOptions): Conversation[] {
  if (!Database) return [];
  const workspaceId = basename(dirname(dbPath));
  try {
    const db = new Database(dbPath, { readonly: true });
    const promptsRow = db.prepare('SELECT value FROM ItemTable WHERE key = ?').get(PROMPTS_KEY) as
      | { value: string }
      | undefined;
    const generationsRow = db
      .prepare('SELECT value FROM ItemTable WHERE key = ?')
      .get(GENERATIONS_KEY) as { value: string } | undefined;
    db.close();
    if (!promptsRow?.value) return [];
    let generations: Array<{ unixMs?: number; textDescription?: string; type?: string }> | undefined;
    if (generationsRow?.value) {
      try {
        generations = JSON.parse(generationsRow.value);
      } catch {
        /* ignore */
      }
    }
    return parseModernPromptsOnly(promptsRow.value, generations, workspaceId, opts);
  } catch {
    return [];
  }
}

function parseModernPromptsOnly(
  value: string,
  generations: Array<{ unixMs?: number; textDescription?: string; type?: string }> | undefined,
  workspaceId: string,
  opts?: ParseOptions,
): Conversation[] {
  try {
    const prompts = JSON.parse(value) as CursorPromptEntry[];
    if (!Array.isArray(prompts) || prompts.length === 0) return [];

    const messages: Message[] = [];
    let firstTime: number | undefined;
    let lastTime: number | undefined;

    for (let i = 0; i < prompts.length; i++) {
      const text = prompts[i].text?.trim() || prompts[i].prompt?.trim();
      if (!text) continue;
      const gen = generations?.[i];
      const createdAt = gen?.unixMs ?? prompts[i].createdAt;
      messages.push({ role: 'user', content: text, parts: textOnlyParts(text), createdAt });
      if (prompts[i].response) {
        messages.push({
          role: 'assistant',
          content: prompts[i].response!,
          parts: textOnlyParts(prompts[i].response!),
          createdAt,
          model: prompts[i].model,
        });
      }
      if (createdAt) {
        if (!firstTime || createdAt < firstTime) firstTime = createdAt;
        if (!lastTime || createdAt > lastTime) lastTime = createdAt;
      }
    }

    if (messages.length === 0) return [];
    const createdAt = firstTime ?? Date.now();
    const updatedAt = lastTime ?? createdAt;
    if (opts?.since && createdAt < opts.since && updatedAt < opts.since) return [];

    return [
      {
        externalId: `cursor-prompts-${workspaceId}`,
        title: messages[0]?.content?.slice(0, 80),
        createdAt,
        updatedAt,
        workspace: workspaceId,
        provider: 'cursor',
        messages,
        metadata: { source: 'cursor-prompts-fallback' },
      },
    ];
  } catch {
    return [];
  }
}

async function findStateFiles(path: string): Promise<string[]> {
  const files: string[] = [];
  const stats = await stat(path).catch(() => null);
  if (!stats) return files;
  if (stats.isFile() && path.endsWith('.vscdb')) {
    files.push(path);
    return files;
  }
  if (!stats.isDirectory()) return files;

  const directDb = join(path, 'state.vscdb');
  if ((await stat(directDb).catch(() => null))?.isFile()) {
    files.push(directDb);
    return files;
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const dbPath = join(path, entry.name, 'state.vscdb');
    if ((await stat(dbPath).catch(() => null))?.isFile()) files.push(dbPath);
  }
  return files;
}

function conversationsToMarkdown(conversations: Conversation[]): string {
  const blocks: string[] = [];
  for (const conv of conversations) {
    blocks.push(`# ${conv.title ?? 'Conversation'}`);
    blocks.push('');
    for (const msg of conv.messages) {
      blocks.push(`## ${msg.role}`);
      blocks.push('');
      blocks.push(msg.content || '');
      blocks.push('');
    }
  }
  return blocks.join('\n').trim() + '\n';
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'cursor',
      displayName: 'Cursor',
      version: '2.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!Database) return null;

    const roots = expandScanRoots(path);
    for (const root of roots) {
      if (!existsSync(root)) continue;

      const snapshots = walkFiles(root, (name) => name.endsWith('.json.gz') || name.endsWith('.json'));
      if (snapshots.length > 0) return 0.95;

      const dbs: string[] = [];
      if (root.endsWith('state.vscdb')) dbs.push(root);
      const globalDb = findGlobalDb([root]);
      if (globalDb) dbs.push(globalDb);
      dbs.push(
        ...walkFiles(root, (name, fullPath) => name === 'state.vscdb' && fullPath.includes('workspaceStorage')),
      );

      for (const dbPath of dbs.slice(0, 5)) {
        try {
          const db = new SqliteKv(dbPath);
          const ok = db.hasComposerData() || db.getJson(CHAT_DATA_KEY) || db.getJson(PROMPTS_KEY);
          db.close();
          if (ok) return 0.95;
        } catch {
          /* continue */
        }
      }
    }

    const files = await findStateFiles(path);
    if (files.length > 0) return 0.7;
    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    if (!Database) return [];
    return loadAllCursorSessions(path, opts);
  },

  async export(conversations, opts) {
    if (opts.format === 'markdown') {
      return {
        format: 'markdown',
        content: conversationsToMarkdown(conversations),
        mimeType: 'text/markdown',
      };
    }
    if (opts.format === 'json' || opts.format === 'cursor') {
      return {
        format: opts.format === 'cursor' ? 'cursor' : 'json',
        content: JSON.stringify(conversations, null, opts.pretty ? 2 : 0),
        mimeType: 'application/json',
      };
    }
    throw new Error(`Unsupported export format: ${opts.format}`);
  },
};

runAdapter(adapter);
