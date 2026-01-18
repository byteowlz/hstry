/**
 * Gemini export adapter for hstry
 *
 * Parses Gemini/AI Studio exports (JSON)
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
} from '../types/index.ts';
import { runAdapter } from '../types/index.ts';

const DEFAULT_SEARCH_PATHS = [
  join(homedir(), 'Downloads'),
  join(homedir(), 'Desktop'),
  join(homedir(), 'Documents'),
];

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'gemini',
      displayName: 'Gemini Export',
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

      const convs = parseGeminiExport(parsed, opts);
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

    if (opts.format !== 'gemini') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const exportData = conversations.map(conv => buildGeminiExport(conv));
    return {
      format: 'gemini',
      content: JSON.stringify(exportData, null, opts.pretty ? 2 : 0),
      mimeType: 'application/json',
    };
  },
};

function sortConversations(conversations: Conversation[]): Conversation[] {
  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

function parseGeminiExport(data: unknown, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];

  const convArray = normalizeConversationArray(data);
  if (!convArray) return conversations;

  for (const raw of convArray) {
    const conv = parseConversation(raw, opts);
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
  }

  return null;
}

function parseConversation(
  raw: Record<string, unknown>,
  opts?: ParseOptions
): Conversation | null {
  const messages = extractMessages(raw);
  if (messages.length === 0) return null;

  const createdAtSec = numberOrUndefined(raw.create_time) ?? earliestMessageTime(messages);
  const createdAt = createdAtSec ? createdAtSec * 1000 : Date.now();

  if (opts?.since && createdAt < opts.since) {
    return null;
  }

  const updatedAtSec =
    numberOrUndefined(raw.update_time) ??
    numberOrUndefined(raw.updated_at) ??
    latestMessageTime(messages);

  const model = deriveModel(messages);

  return {
    externalId: stringOrUndefined(raw.id ?? raw.conversation_id),
    title: stringOrUndefined(raw.title ?? raw.name),
    createdAt,
    updatedAt: updatedAtSec ? updatedAtSec * 1000 : undefined,
    model,
    messages,
    metadata: {
      source: 'gemini-export',
    },
  };
}

function extractMessages(raw: Record<string, unknown>): Message[] {
  if (raw.mapping && typeof raw.mapping === 'object') {
    return extractMessagesFromMapping(raw.mapping as Record<string, unknown>);
  }

  const messagesArray = raw.messages;
  if (!Array.isArray(messagesArray)) return [];

  const messages: Message[] = [];
  for (const entry of messagesArray) {
    if (!entry || typeof entry !== 'object') continue;
    const msg = entry as Record<string, unknown>;
    const role = stringOrUndefined(msg.role ?? msg.author?.role ?? msg.sender?.role);
    if (!role) continue;

    const content = extractContent(msg.content ?? msg.text ?? msg.parts);
    if (!content) continue;

    const createdAt = parseTimestamp(msg.create_time ?? msg.created_at ?? msg.timestamp);

    messages.push({
      role: mapRole(role),
      content,
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

    const author = msg.author as Record<string, unknown> | undefined;
    const role = stringOrUndefined(author?.role ?? msg.role);
    if (!role) continue;

    const content = extractContent((msg.content as unknown) ?? (msg as unknown));
    if (!content) continue;

    const createdAt = parseTimestamp(msg.create_time ?? msg.created_at ?? msg.timestamp);

    messages.push({
      role: mapRole(role),
      content,
      createdAt,
      model: stringOrUndefined((msg.metadata as Record<string, unknown> | undefined)?.model_slug),
    });
  }

  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

function extractContent(source: unknown): string {
  if (!source) return '';
  if (typeof source === 'string') return source.trim();

  if (Array.isArray(source)) {
    const parts = source
      .map(part => {
        if (typeof part === 'string') return part;
        if (part && typeof part === 'object') {
          const text = (part as Record<string, unknown>).text;
          if (typeof text === 'string') return text;
        }
        return '';
      })
      .map(part => part.trim())
      .filter(Boolean);
    return parts.join('\n');
  }

  if (typeof source === 'object') {
    const obj = source as Record<string, unknown>;
    if (typeof obj.text === 'string') return obj.text.trim();
    if (Array.isArray(obj.parts)) return extractContent(obj.parts);
    if (typeof obj.content === 'string') return obj.content.trim();
  }

  return '';
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
    case 'human':
      return 'user';
    case 'assistant':
    case 'model':
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

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined;
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function earliestMessageTime(messages: Message[]): number | undefined {
  let earliest: number | undefined;
  for (const msg of messages) {
    if (!msg.createdAt) continue;
    const sec = Math.floor(msg.createdAt / 1000);
    if (earliest === undefined || sec < earliest) {
      earliest = sec;
    }
  }
  return earliest;
}

function latestMessageTime(messages: Message[]): number | undefined {
  let latest: number | undefined;
  for (const msg of messages) {
    if (!msg.createdAt) continue;
    const sec = Math.floor(msg.createdAt / 1000);
    if (latest === undefined || sec > latest) {
      latest = sec;
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

function buildGeminiExport(conv: Conversation): Record<string, unknown> {
  const createdAtSec = Math.floor(conv.createdAt / 1000);
  const updatedAtSec = Math.floor((conv.updatedAt ?? conv.createdAt) / 1000);
  return {
    id: conv.externalId ?? `gemini-${Date.now()}`,
    title: conv.title ?? 'Conversation',
    create_time: createdAtSec,
    update_time: updatedAtSec,
    messages: conv.messages.map(msg => ({
      role: msg.role,
      content: msg.content,
      create_time: msg.createdAt ? Math.floor(msg.createdAt / 1000) : createdAtSec,
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

  const directCandidates = [
    join(path, 'conversations.json'),
    join(path, 'gemini-export.json'),
  ];

  for (const candidate of directCandidates) {
    const candidateStats = await stat(candidate).catch(() => null);
    if (candidateStats?.isFile()) files.push(candidate);
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries.slice(0, 200)) {
    const entryPath = join(path, entry.name);

    if (entry.isFile() && looksLikeExportFile(entry.name)) {
      files.push(entryPath);
      continue;
    }

    if (entry.isDirectory()) {
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
  }

  return files;
}

function looksLikeExportFile(nameOrPath: string): boolean {
  if (extname(nameOrPath) !== '.json') return false;
  const lower = nameOrPath.toLowerCase();
  return (
    lower.includes('gemini') ||
    lower.includes('ai-studio') ||
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
    if (sample.mapping || sample.messages) return 0.75;
    if (sample.create_time || sample.title) return 0.6;
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
