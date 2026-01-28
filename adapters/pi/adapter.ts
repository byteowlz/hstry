/**
 * Pi adapter for hstry
 *
 * Parses Pi session logs stored as JSONL files in ~/.pi/agent/sessions
 * 
 * Pi uses an append-only tree structure where entries are linked via id/parentId.
 * Sessions are organized by workspace directory.
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
  ParseStreamResult,
  ToolCall,
  ExportOptions,
  ExportResult,
  ExportFile,
} from '../types/index.ts';
import { runAdapter } from '../types/index.ts';

const DEFAULT_PI_PATH = join(homedir(), '.pi', 'agent', 'sessions');

// Session header (first line of JSONL)
interface SessionHeader {
  type: 'session';
  version?: number;
  id: string;
  timestamp: string;
  cwd: string;
  parentSession?: string;
}

// Base entry structure
interface SessionEntryBase {
  type: string;
  id: string;
  parentId: string | null;
  timestamp: string;
}

// Message entry
interface MessageEntry extends SessionEntryBase {
  type: 'message';
  message: PiMessage;
}

// Model change entry
interface ModelChangeEntry extends SessionEntryBase {
  type: 'model_change';
  provider: string;
  modelId: string;
}

// Compaction entry (context summarization)
interface CompactionEntry extends SessionEntryBase {
  type: 'compaction';
  summary: string;
  firstKeptEntryId: string;
  tokensBefore: number;
}

// Session info entry
interface SessionInfoEntry extends SessionEntryBase {
  type: 'session_info';
  name?: string;
}

type SessionEntry = SessionHeader | MessageEntry | ModelChangeEntry | CompactionEntry | SessionInfoEntry | SessionEntryBase;

// Pi message types
interface PiMessage {
  role: 'user' | 'assistant' | 'toolResult' | 'custom';
  content?: PiContentBlock[];
  timestamp?: number;
  // Assistant-specific fields
  api?: string;
  provider?: string;
  model?: string;
  usage?: PiUsage;
  stopReason?: string;
  // Tool result fields
  toolCallId?: string;
  toolName?: string;
  isError?: boolean;
}

interface PiContentBlock {
  type: string;
  text?: string;
  thinking?: string;
  // Tool call fields
  id?: string;
  name?: string;
  arguments?: Record<string, unknown>;
  // Image fields
  data?: string;
  mimeType?: string;
}

interface PiUsage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  totalTokens: number;
  cost: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    total: number;
  };
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'pi',
      displayName: 'Pi',
      version: '1.0.0',
      defaultPaths: [DEFAULT_PI_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findJsonlFiles(path, { shallowOnly: true });
    if (files.length === 0) return null;

    // Check if any file looks like a Pi session (has session header with cwd)
    for (const file of files.slice(0, 3)) {
      const content = await readFile(file, 'utf-8').catch(() => '');
      const firstLine = content.split('\n')[0];
      try {
        const header = JSON.parse(firstLine);
        if (header.type === 'session' && header.cwd && header.id) {
          return 0.95;
        }
      } catch {
        continue;
      }
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findJsonlFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations = await parseFiles(files, opts);
    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return conversations;
  },

  async parseStream(path: string, opts?: ParseOptions): Promise<ParseStreamResult> {
    const files = await findJsonlFiles(path, { shallowOnly: false });
    const cursor = normalizeCursor(opts?.cursor);

    let pendingFiles = cursor.pending;
    let fileStates = cursor.files;

    if (!pendingFiles || pendingFiles.length === 0) {
      const scan = await findChangedFiles(files, cursor.files);
      pendingFiles = scan.pending;
      fileStates = scan.files;
    }

    if (!pendingFiles || pendingFiles.length === 0) {
      return {
        conversations: [],
        cursor: { files: fileStates, pending: [] },
        done: true,
      };
    }

    const batchSize = Math.max(1, opts?.batchSize ?? pendingFiles.length);
    const batchFiles = pendingFiles.slice(0, batchSize);
    const remaining = pendingFiles.slice(batchSize);

    const conversations = await parseFiles(batchFiles, opts);
    return {
      conversations,
      cursor: { files: fileStates, pending: remaining },
      done: remaining.length === 0,
    };
  },

  async export(conversations: Conversation[], opts: ExportOptions): Promise<ExportResult> {
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

    if (opts.format !== 'pi') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = buildPiFiles(conversations, opts);
    return {
      format: 'pi',
      files,
      mimeType: 'application/x-ndjson',
      metadata: {
        root: 'sessions/',
      },
    };
  },
};

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

function buildPiFiles(conversations: Conversation[], opts: ExportOptions): ExportFile[] {
  const files: ExportFile[] = [];

  for (const conv of conversations) {
    const sessionId = conv.externalId ?? randomId('ses_');
    const workspace = conv.workspace ?? process.cwd();
    const safePath = `--${workspace.replace(/^[/\\]/, '').replace(/[/\\:]/g, '-')}--`;
    const timestamp = new Date(conv.createdAt).toISOString().replace(/[:.]/g, '-');
    const filename = `${timestamp}_${sessionId}.jsonl`;
    const filePath = `sessions/${safePath}/${filename}`;

    const lines: string[] = [];

    // Session header
    const header: SessionHeader = {
      type: 'session',
      version: 3,
      id: sessionId,
      timestamp: new Date(conv.createdAt).toISOString(),
      cwd: workspace,
    };
    lines.push(JSON.stringify(header));

    // Model change entry (if we have model info)
    let lastEntryId: string | null = null;
    if (conv.model) {
      const modelEntry = {
        type: 'model_change',
        id: randomId(''),
        parentId: lastEntryId,
        timestamp: new Date(conv.createdAt).toISOString(),
        provider: deriveProvider(conv.model),
        modelId: conv.model,
      };
      lines.push(JSON.stringify(modelEntry));
      lastEntryId = modelEntry.id;
    }

    // Session name (if title exists and isn't derived from first message)
    if (conv.title) {
      const infoEntry: SessionInfoEntry = {
        type: 'session_info',
        id: randomId(''),
        parentId: lastEntryId,
        timestamp: new Date(conv.createdAt).toISOString(),
        name: conv.title,
      };
      lines.push(JSON.stringify(infoEntry));
      lastEntryId = infoEntry.id;
    }

    // Messages
    for (const msg of conv.messages) {
      const msgId = randomId('msg_');
      const msgTimestamp = msg.createdAt
        ? new Date(msg.createdAt).toISOString()
        : new Date(conv.createdAt).toISOString();

      const contentBlocks: PiContentBlock[] = [];
      if (msg.content) {
        contentBlocks.push({ type: 'text', text: msg.content });
      }

      // Add tool calls if present
      if (opts.includeTools && msg.toolCalls) {
        for (const tc of msg.toolCalls) {
          contentBlocks.push({
            type: 'toolCall',
            id: randomId('call_'),
            name: tc.toolName,
            arguments: tc.input as Record<string, unknown> | undefined,
          });
        }
      }

      const piMessage: PiMessage = {
        role: mapRoleToPi(msg.role),
        content: contentBlocks.length > 0 ? contentBlocks : undefined,
        timestamp: msg.createdAt,
        model: msg.model,
        usage: msg.tokens ? {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: msg.tokens,
          cost: msg.costUsd ? {
            input: 0,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            total: msg.costUsd,
          } : undefined as unknown as PiUsage['cost'],
        } : undefined,
      };

      const msgEntry: MessageEntry = {
        type: 'message',
        id: msgId,
        parentId: lastEntryId,
        timestamp: msgTimestamp,
        message: piMessage,
      };
      lines.push(JSON.stringify(msgEntry));
      lastEntryId = msgId;
    }

    files.push({
      path: filePath,
      content: lines.join('\n') + '\n',
    });
  }

  return files;
}

function mapRoleToPi(role: Message['role']): PiMessage['role'] {
  switch (role) {
    case 'user': return 'user';
    case 'assistant': return 'assistant';
    case 'tool': return 'toolResult';
    case 'system': return 'custom';
    default: return 'user';
  }
}

function deriveProvider(model: string): string {
  const lower = model.toLowerCase();
  if (lower.includes('claude') || lower.includes('anthropic')) return 'anthropic';
  if (lower.includes('gpt') || lower.includes('openai') || lower.includes('codex')) return 'openai';
  if (lower.includes('gemini') || lower.includes('google')) return 'google';
  if (lower.includes('llama') || lower.includes('meta')) return 'meta';
  return 'unknown';
}

function randomId(prefix: string): string {
  const uuid = typeof globalThis.crypto?.randomUUID === 'function'
    ? globalThis.crypto.randomUUID()
    : `${Date.now()}${Math.random().toString(16).slice(2)}`;
  return `${prefix}${uuid.replace(/-/g, '').slice(0, 12)}`;
}

function extractMessages(entries: SessionEntry[], includeTools: boolean): Message[] {
  const messages: Message[] = [];

  for (const entry of entries) {
    if (entry.type !== 'message') continue;
    const msgEntry = entry as MessageEntry;
    const msg = msgEntry.message;
    if (!msg || !msg.role) continue;

    const content = extractContent(msg.content);
    
    // Skip empty messages unless they have tool calls
    const toolCalls = includeTools ? extractToolCalls(msg.content) : undefined;
    if (!content && (!toolCalls || toolCalls.length === 0)) continue;

    const createdAt = msg.timestamp ?? parseTimestamp(msgEntry.timestamp);

    messages.push({
      role: mapRole(msg.role),
      content,
      createdAt,
      model: msg.model,
      tokens: msg.usage?.totalTokens,
      costUsd: msg.usage?.cost?.total,
      toolCalls: toolCalls?.length ? toolCalls : undefined,
      metadata: {
        id: msgEntry.id,
        parentId: msgEntry.parentId,
        provider: msg.provider,
        api: msg.api,
        stopReason: msg.stopReason,
        isError: msg.isError,
        toolCallId: msg.toolCallId,
        toolName: msg.toolName,
      },
    });
  }

  // Messages are already in tree order from the JSONL append-only format
  return messages;
}

function extractContent(content?: PiContentBlock[]): string {
  if (!content || !Array.isArray(content)) return '';

  const parts: string[] = [];
  for (const block of content) {
    if (!block) continue;
    if (block.type === 'text' && block.text) {
      parts.push(block.text.trim());
    } else if (block.type === 'thinking' && block.thinking) {
      parts.push(block.thinking.trim());
    }
  }

  return parts.filter(Boolean).join('\n').trim();
}

function extractToolCalls(content?: PiContentBlock[]): ToolCall[] | undefined {
  if (!content || !Array.isArray(content)) return undefined;

  const calls: ToolCall[] = [];
  for (const block of content) {
    if (block.type === 'toolCall' && block.name) {
      calls.push({
        toolName: block.name,
        input: block.arguments,
      });
    }
  }

  return calls.length > 0 ? calls : undefined;
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
      return 'user';
    case 'assistant':
      return 'assistant';
    case 'toolresult':
      return 'tool';
    case 'system':
      return 'system';
    default:
      return 'assistant';
  }
}

function calculateTotals(entries: SessionEntry[]): {
  tokensIn?: number;
  tokensOut?: number;
  costUsd?: number;
  model?: string;
} {
  let tokensIn = 0;
  let tokensOut = 0;
  let costUsd = 0;
  let model: string | undefined;

  for (const entry of entries) {
    if (entry.type !== 'message') continue;
    const msg = (entry as MessageEntry).message;
    if (msg.role !== 'assistant' || !msg.usage) continue;

    tokensIn += msg.usage.input + msg.usage.cacheRead;
    tokensOut += msg.usage.output;
    costUsd += msg.usage.cost?.total ?? 0;
    if (msg.model) model = msg.model;
  }

  return {
    tokensIn: tokensIn > 0 ? tokensIn : undefined,
    tokensOut: tokensOut > 0 ? tokensOut : undefined,
    costUsd: costUsd > 0 ? costUsd : undefined,
    model,
  };
}

function deriveTitle(messages: Message[]): string | undefined {
  // Use first user message as title, truncated
  // Skip messages that are AGENTS.md content
  const firstUser = messages.find(m => m.role === 'user' && !isAgentsMdContent(m.content));
  if (!firstUser?.content) return undefined;
  
  const text = firstUser.content.slice(0, 100);
  return text.length < firstUser.content.length ? `${text}...` : text;
}

/** Detect if content is an AGENTS.md file or similar config content. */
function isAgentsMdContent(content?: string): boolean {
  if (!content) return false;
  
  const strongMarkers = [
    '# AGENTS.md',
    '# Agent Configuration',
    '<available_skills>',
    'Guidance for coding agents',
  ];

  for (const marker of strongMarkers) {
    if (content.includes(marker)) {
      return true;
    }
  }

  // Check for filename reference with context
  if (content.includes('AGENTS.md') && content.includes('instructions')) {
    return true;
  }

  return false;
}

function parseTimestamp(value?: string | number): number | undefined {
  if (!value) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
}

interface PiCursorState {
  files: Record<string, { mtimeMs: number; size: number }>;
  pending?: string[];
}

function normalizeCursor(cursor: unknown): PiCursorState {
  if (!cursor || typeof cursor !== 'object') {
    return { files: {} };
  }
  const record = cursor as Record<string, unknown>;
  const files = typeof record.files === 'object' && record.files
    ? record.files as Record<string, { mtimeMs: number; size: number }>
    : {};
  const pending = Array.isArray(record.pending) ? record.pending.filter(p => typeof p === 'string') : undefined;
  return { files, pending };
}

async function findChangedFiles(
  files: string[],
  previous: Record<string, { mtimeMs: number; size: number }>
): Promise<{ pending: string[]; files: Record<string, { mtimeMs: number; size: number }> }> {
  const pending: string[] = [];
  const nextFiles: Record<string, { mtimeMs: number; size: number }> = { ...previous };

  for (const file of files) {
    const stats = await stat(file).catch(() => null);
    if (!stats || !stats.isFile()) {
      continue;
    }
    const prev = previous[file];
    const state = { mtimeMs: stats.mtimeMs, size: stats.size };
    nextFiles[file] = state;
    if (!prev || prev.mtimeMs !== state.mtimeMs || prev.size !== state.size) {
      pending.push(file);
    }
  }

  return { pending, files: nextFiles };
}

async function parseFiles(files: string[], opts?: ParseOptions): Promise<Conversation[]> {
  const conversations: Conversation[] = [];

  for (const filePath of files) {
    const raw = await readFile(filePath, 'utf-8');
    const lines = raw.split(/\r?\n/).filter(line => line.trim().length > 0);
    if (lines.length === 0) continue;

    const entries: SessionEntry[] = [];
    let header: SessionHeader | null = null;

    for (const line of lines) {
      try {
        const entry = JSON.parse(line) as SessionEntry;
        if (entry.type === 'session') {
          header = entry as SessionHeader;
        }
        entries.push(entry);
      } catch {
        continue;
      }
    }

    if (!header) continue;

    const messages = extractMessages(entries, opts?.includeTools ?? true);
    if (messages.length === 0) continue;

    const timestamps = messages
      .map(msg => msg.createdAt)
      .filter((ts): ts is number => typeof ts === 'number');

    const createdAt = timestamps.length > 0 
      ? Math.min(...timestamps) 
      : parseTimestamp(header.timestamp) ?? Date.now();
    const updatedAt = timestamps.length > 0 ? Math.max(...timestamps) : undefined;

    // Skip if both createdAt AND updatedAt are before the since filter
    // This ensures we re-import sessions that were modified (e.g., renamed) after last sync
    if (opts?.since) {
      const lastModified = updatedAt ?? createdAt;
      if (createdAt < opts.since && lastModified < opts.since) {
        continue;
      }
    }

    // Get session name from the latest session_info entry (Pi appends new entries on rename)
    const sessionInfoEntries = entries.filter(
      (e): e is SessionInfoEntry => e.type === 'session_info' && 'name' in e && !!e.name
    );
    const latestSessionInfo = sessionInfoEntries[sessionInfoEntries.length - 1];
    const title = latestSessionInfo?.name || deriveTitle(messages);

    // Calculate totals from assistant messages with usage
    const { tokensIn, tokensOut, costUsd, model } = calculateTotals(entries);

    conversations.push({
      externalId: header.id,
      title,
      createdAt,
      updatedAt,
      workspace: header.cwd,
      model,
      tokensIn,
      tokensOut,
      costUsd,
      messages,
      metadata: {
        file: filePath,
        version: header.version,
        parentSession: header.parentSession,
      },
    });

    if (opts?.limit && conversations.length >= opts.limit) {
      break;
    }
  }

  return conversations;
}

async function findJsonlFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const files: string[] = [];

  const stats = await stat(path).catch(() => null);
  if (!stats) return files;

  if (stats.isFile()) {
    if (extname(path) === '.jsonl') files.push(path);
    return files;
  }

  if (!stats.isDirectory()) return files;

  // Pi organizes sessions in subdirectories named after the encoded cwd
  // e.g., ~/.pi/agent/sessions/--Users-foo-project--/timestamp_uuid.jsonl
  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  
  for (const entry of entries) {
    const entryPath = join(path, entry.name);

    if (entry.isFile() && extname(entry.name) === '.jsonl') {
      files.push(entryPath);
      continue;
    }

    if (entry.isDirectory()) {
      // Search one level deep into workspace directories
      const nested = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const child of nested) {
        if (child.isFile() && extname(child.name) === '.jsonl') {
          files.push(join(entryPath, child.name));
        }
      }

      if (!opts.shallowOnly) {
        // Search deeper for any additional nesting
        for (const child of nested) {
          if (child.isDirectory()) {
            const deepPath = join(entryPath, child.name);
            const deepEntries = await readdir(deepPath, { withFileTypes: true }).catch(() => []);
            for (const deepEntry of deepEntries) {
              if (deepEntry.isFile() && extname(deepEntry.name) === '.jsonl') {
                files.push(join(deepPath, deepEntry.name));
              }
            }
          }
        }
      }
    }
  }

  return files;
}

runAdapter(adapter);
