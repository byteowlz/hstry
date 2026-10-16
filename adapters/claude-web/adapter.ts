/**
 * Claude Web export adapter for hstry
 *
 * Parses Claude.ai export JSON files (best-effort)
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, extname, join } from 'path';
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
  textOnlyParts,
  textPart,
  thinkingPart,
  toolCallPart,
  toolResultPart,
} from '../types/index.ts';

const DEFAULT_SEARCH_PATHS = [
  join(homedir(), 'Downloads'),
  join(homedir(), 'Desktop'),
  join(homedir(), 'Documents'),
];

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'claude-web',
      displayName: 'Claude Web Export',
      version: '1.0.0',
      defaultPaths: DEFAULT_SEARCH_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const candidates = await findExportFiles(path, { shallowOnly: true });
    for (const filePath of candidates) {
      const score = await sniffFile(filePath);
      if (score) return score;
    }
    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findExportFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const raw = await readFile(filePath, 'utf-8');
      let parsed: unknown;
      try {
        parsed = JSON.parse(raw);
      } catch {
        continue;
      }

      const convs = parseClaudeExport(parsed, opts, filePath);
      for (const conv of convs) {
        conversations.push(conv);
        if (opts?.limit && conversations.length >= opts.limit) {
          return sortConversations(conversations);
        }
      }
    }

    return sortConversations(conversations);
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

    if (opts.format !== 'claude-web') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const exportData = conversations.map(conv => buildClaudeWebExport(conv));
    return {
      format: 'claude-web',
      content: JSON.stringify(exportData, null, opts.pretty ? 2 : 0),
      mimeType: 'application/json',
      metadata: {
        filename: 'claude-conversations.json',
      },
    };
  },
};

function sortConversations(conversations: Conversation[]): Conversation[] {
  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

function parseClaudeExport(
  data: unknown,
  opts: ParseOptions | undefined,
  filePath: string
): Conversation[] {
  const conversations: Conversation[] = [];
  const entries = normalizeConversationArray(data);
  if (!entries) return conversations;

  for (const raw of entries) {
    const conv = parseConversation(raw, opts, filePath);
    if (conv) conversations.push(conv);
  }

  return conversations;
}

function normalizeConversationArray(data: unknown): Record<string, unknown>[] | null {
  if (Array.isArray(data)) {
    return data.filter(entry => typeof entry === 'object' && entry !== null) as Record<
      string,
      unknown
    >[];
  }

  if (data && typeof data === 'object') {
    const obj = data as Record<string, unknown>;
    if (Array.isArray(obj.conversations)) {
      return obj.conversations.filter(entry => typeof entry === 'object' && entry !== null) as Record<
        string,
        unknown
      >[];
    }
    if (Array.isArray(obj.chats)) {
      return obj.chats.filter(entry => typeof entry === 'object' && entry !== null) as Record<
        string,
        unknown
      >[];
    }
    if (Array.isArray(obj.data)) {
      return obj.data.filter(entry => typeof entry === 'object' && entry !== null) as Record<
        string,
        unknown
      >[];
    }
  }

  return null;
}

function parseConversation(
  raw: Record<string, unknown>,
  opts: ParseOptions | undefined,
  filePath: string
): Conversation | null {
  const messages = extractMessages(raw);
  if (messages.length === 0) return null;

  const createdAtMs =
    parseTimestamp(raw.created_at ?? raw.create_time ?? raw.createdAt ?? raw.created) ??
    earliestMessageTime(messages) ??
    Date.now();

  const updatedAtMs =
    parseTimestamp(raw.updated_at ?? raw.update_time ?? raw.updatedAt ?? raw.updated) ??
    latestMessageTime(messages);

  // Check both created and updated time so modified sessions are re-imported
  if (opts?.since) {
    const lastModified = updatedAtMs ?? createdAtMs;
    if (createdAtMs < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const model = deriveModel(messages) ?? stringOrUndefined(raw.model ?? raw.model_name);

  return {
    externalId: stringOrUndefined(raw.uuid ?? raw.id ?? raw.conversation_id),
    title: stringOrUndefined(raw.name ?? raw.title ?? raw.subject),
    createdAt: createdAtMs,
    updatedAt: updatedAtMs ?? undefined,
    model,
    messages,
    metadata: {
      file: filePath,
    },
  };
}

function extractMessages(raw: Record<string, unknown>): Message[] {
  const chatMessages = raw.chat_messages ?? raw.messages ?? raw.message_list;
  if (Array.isArray(chatMessages)) {
    const msgs = parseMessageArray(chatMessages);
    if (msgs.length > 0) return msgs;
  }

  if (raw.mapping && typeof raw.mapping === 'object') {
    return extractMessagesFromMapping(raw.mapping as Record<string, unknown>);
  }

  return [];
}

function parseMessageArray(entries: unknown[]): Message[] {
  const messages: Message[] = [];

  for (const entry of entries) {
    if (!entry || typeof entry !== 'object') continue;
    const msg = entry as Record<string, unknown>;

    const role = stringOrUndefined(
      msg.role ??
        msg.sender ??
        (msg.author && typeof msg.author === 'object'
          ? (msg.author as Record<string, unknown>).role
          : undefined)
    );

    if (!role) continue;

    const built = buildClaudeWebParts(msg.content ?? msg.text ?? msg.message ?? msg.parts);
    const parts = built.parts;
    const content = built.content || (parts ? summarizeParts(parts) : '');
    if (!content && !parts) continue;

    const createdAt = parseTimestamp(msg.created_at ?? msg.create_time ?? msg.timestamp);

    messages.push({
      role: mapRole(role),
      content,
      parts: parts ?? textOnlyParts(content),
      createdAt,
      model: stringOrUndefined(msg.model ?? msg.model_name ?? msg.model_slug),
    });
  }

  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

function extractMessagesFromMapping(mapping: Record<string, unknown>): Message[] {
  const messages: Message[] = [];

  for (const node of Object.values(mapping)) {
    if (!node || typeof node !== 'object') continue;
    const msg = (node as Record<string, unknown>).message as Record<string, unknown> | undefined;
    if (!msg) continue;

    const role = stringOrUndefined(
      msg.role ??
        (msg.author && typeof msg.author === 'object'
          ? (msg.author as Record<string, unknown>).role
          : undefined)
    );
    if (!role) continue;

    const built = buildClaudeWebParts(msg.content ?? (msg as unknown));
    const parts = built.parts;
    const content = built.content || (parts ? summarizeParts(parts) : '');
    if (!content && !parts) continue;

    const createdAt = parseTimestamp(msg.create_time ?? msg.created_at ?? msg.timestamp);

    messages.push({
      role: mapRole(role),
      content,
      parts: parts ?? textOnlyParts(content),
      createdAt,
      model: stringOrUndefined((msg.metadata as Record<string, unknown> | undefined)?.model_slug),
    });
  }

  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

interface ClaudeWebContentBlock {
  type?: string;
  text?: string;
  thinking?: string;
  id?: string;
  name?: string;
  input?: unknown;
  tool_use_id?: string;
  content?: unknown;
  is_error?: boolean;
}

/**
 * Map the export's typed content blocks to canonical parts. The claude.ai
 * export carries `text`, `thinking`, `tool_use` and `tool_result` blocks;
 * keeping them as parts preserves reasoning and tool association instead of
 * flattening everything to prose.
 */
function buildClaudeWebParts(source: unknown): { content: string; parts?: CanonPart[] } {
  if (!source) return { content: '' };
  if (typeof source === 'string') return { content: source.trim() };

  if (Array.isArray(source)) {
    const parts: CanonPart[] = [];
    const chunks: string[] = [];
    for (const block of source) {
      if (typeof block === 'string') {
        if (block.trim()) {
          parts.push(textPart(block));
          chunks.push(block.trim());
        }
        continue;
      }
      if (!block || typeof block !== 'object') continue;
      const b = block as ClaudeWebContentBlock;
      if (b.type === 'thinking' || (b.type === undefined && typeof b.thinking === 'string')) {
        const text = (b.thinking ?? b.text ?? '').trim();
        if (text) {
          parts.push(thinkingPart(text));
          chunks.push(text);
        }
      } else if (b.type === 'tool_use') {
        parts.push(toolCallPart(String(b.id ?? b.name ?? 'tool'), String(b.name ?? 'tool'), b.input));
      } else if (b.type === 'tool_result') {
        const output = typeof b.content === 'string' ? b.content : blockText(b.content);
        parts.push(toolResultPart(String(b.tool_use_id ?? 'unknown'), output, { isError: b.is_error === true }));
        if (output) chunks.push(output);
      } else if (typeof b.text === 'string' && b.text.trim()) {
        parts.push(textPart(b.text));
        chunks.push(b.text.trim());
      }
    }
    return { content: chunks.join('\n').trim(), parts: parts.length ? parts : undefined };
  }

  if (typeof source === 'object') {
    const obj = source as Record<string, unknown>;
    if (typeof obj.text === 'string') return { content: obj.text.trim() };
    if (typeof obj.content === 'string') return { content: obj.content.trim() };
    if (Array.isArray(obj.content)) return buildClaudeWebParts(obj.content);
    if (Array.isArray(obj.parts)) return buildClaudeWebParts(obj.parts);
  }

  return { content: '' };
}

function blockText(value: unknown): string {
  if (typeof value === 'string') return value.trim();
  if (Array.isArray(value)) {
    return value
      .map(item => (typeof item === 'string' ? item : typeof item?.text === 'string' ? item.text : ''))
      .filter(Boolean)
      .join('\n')
      .trim();
  }
  return '';
}

/** Compact searchable text for a turn that only has structured parts. */
function summarizeParts(parts: CanonPart[]): string {
  return parts
    .map(part => {
      if (part.type === 'tool_call') return `\u{2699} ${part.name}`;
      if (part.type === 'thinking') return part.text;
      return '';
    })
    .filter(Boolean)
    .join('\n')
    .trim();
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
      return 'tool';
    default:
      return 'assistant';
  }
}

function parseTimestamp(value: unknown): number | undefined {
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  if (typeof value === 'string') {
    const parsed = Date.parse(value);
    if (!Number.isNaN(parsed)) return parsed;
  }
  return undefined;
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function earliestMessageTime(messages: Message[]): number | undefined {
  let earliest: number | undefined;
  for (const msg of messages) {
    if (!msg.createdAt) continue;
    if (earliest === undefined || msg.createdAt < earliest) {
      earliest = msg.createdAt;
    }
  }
  return earliest;
}

function latestMessageTime(messages: Message[]): number | undefined {
  let latest: number | undefined;
  for (const msg of messages) {
    if (!msg.createdAt) continue;
    if (latest === undefined || msg.createdAt > latest) {
      latest = msg.createdAt;
    }
  }
  return latest;
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

function buildClaudeWebExport(conv: Conversation): Record<string, unknown> {
  const createdAtSec = Math.floor(conv.createdAt / 1000);
  const updatedAtSec = Math.floor((conv.updatedAt ?? conv.createdAt) / 1000);
  return {
    uuid: conv.externalId ?? `claude-${Date.now()}`,
    name: conv.title ?? 'Conversation',
    created_at: createdAtSec,
    updated_at: updatedAtSec,
    chat_messages: conv.messages.map(msg => ({
      role: msg.role,
      content: msg.content,
      created_at: msg.createdAt ? Math.floor(msg.createdAt / 1000) : createdAtSec,
      model: msg.model,
    })),
  };
}

async function findExportFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const files: string[] = [];

  const stats = await stat(path).catch(() => null);
  if (!stats) return files;

  if (stats.isFile()) {
    if (looksLikeExportFile(path)) files.push(path);
    return files;
  }

  if (!stats.isDirectory()) return files;

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries.slice(0, 200)) {
    const entryPath = join(path, entry.name);
    if (entry.isFile() && looksLikeExportFile(entry.name)) {
      files.push(entryPath);
      continue;
    }

    if (!entry.isDirectory()) continue;

    const childFile = join(entryPath, 'conversations.json');
    const childStats = await stat(childFile).catch(() => null);
    if (childStats?.isFile()) {
      files.push(childFile);
      if (opts.shallowOnly) continue;
    }

    if (!opts.shallowOnly) {
      const childEntries = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const child of childEntries) {
        if (child.isFile() && looksLikeExportFile(child.name)) {
          files.push(join(entryPath, child.name));
        }
      }
    }
  }

  return files;
}

function looksLikeExportFile(nameOrPath: string): boolean {
  if (extname(nameOrPath).toLowerCase() !== '.json') return false;
  const lower = nameOrPath.toLowerCase();
  return (
    lower.includes('claude') ||
    lower.includes('anthropic') ||
    lower.endsWith('conversations.json')
  );
}

async function sniffFile(path: string): Promise<number | null> {
  if (!looksLikeExportFile(path)) return null;
  const raw = await readFile(path, 'utf-8').catch(() => null);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    const convArray = normalizeConversationArray(parsed);
    if (!convArray || convArray.length === 0) return null;
    const sample = convArray[0];
    if (sample.chat_messages || sample.messages) return 0.8;
    if (sample.name || sample.title || sample.uuid) return 0.6;
  } catch {
    return null;
  }
  return null;
}

function deriveModel(messages: Message[]): string | undefined {
  for (const msg of messages) {
    if (msg.model) return msg.model;
  }
  return undefined;
}

runAdapter(adapter);
