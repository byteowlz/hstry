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
  ToolCall,
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

      if (opts?.since && createdAt < opts.since) {
        continue;
      }

      // Get session name from session_info entry if present
      const sessionInfo = entries.find(
        (e): e is SessionInfoEntry => e.type === 'session_info' && 'name' in e
      );
      const title = sessionInfo?.name || deriveTitle(messages);

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

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return conversations;
  },
};

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
  const firstUser = messages.find(m => m.role === 'user');
  if (!firstUser?.content) return undefined;
  
  const text = firstUser.content.slice(0, 100);
  return text.length < firstUser.content.length ? `${text}...` : text;
}

function parseTimestamp(value?: string | number): number | undefined {
  if (!value) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
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
