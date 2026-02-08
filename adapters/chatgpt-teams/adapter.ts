/**
 * ChatGPT Teams browser export adapter for hstry
 *
 * Parses ChatGPT Teams data from browser extension exports
 * Format: Array of conversations with chat.history.messages structure
 */

import { readdir, readFile, stat } from 'fs/promises';
import { join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  Conversation,
  Message,
  ParseOptions,
} from '../types/index.ts';
import { runAdapter, textOnlyParts } from '../types/index.ts';

const DEFAULT_SEARCH_PATHS = [
  join(homedir(), 'Downloads'),
  join(homedir(), 'Desktop'),
  join(homedir(), 'Documents'),
];

interface TeamsMessage {
  id: string;
  parentId?: string;
  childrenIds?: string[];
  role: 'user' | 'assistant' | 'system';
  content: string;
  model?: string;
  done?: boolean;
  context?: unknown;
}

interface TeamsHistory {
  currentId?: string;
  messages?: Record<string, TeamsMessage>;
}

interface TeamsChat {
  history?: TeamsHistory;
  timestamp?: number;
  models?: string[];
}

interface RawConversation {
  id?: string;
  user_id?: string;
  title?: string;
  chat?: TeamsChat;
  created_at?: number;
  updated_at?: number;
  archived?: boolean;
  pinned?: boolean;
  folder_id?: string | null;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'chatgpt-teams',
      displayName: 'ChatGPT Teams Browser Export',
      version: '1.0.0',
      defaultPaths: DEFAULT_SEARCH_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const pathStats = await stat(path).catch(() => null);
    if (pathStats?.isFile() && path.endsWith('.json')) {
      const isExport = await looksLikeTeamsExport(path);
      return isExport ? 0.9 : null;
    }

    const candidates = await findConversationFiles(path, { shallowOnly: true });
    return candidates.length > 0 ? 0.9 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findConversationFiles(path, { shallowOnly: false });
    if (files.length === 0) {
      return [];
    }

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const raw = await readFile(filePath, 'utf-8');
      let parsed: unknown;
      try {
        parsed = JSON.parse(raw);
      } catch {
        continue;
      }

      if (!Array.isArray(parsed)) {
        continue;
      }

      // Extract date from filename (e.g., history_20250822_120830.json)
      const fileDate = extractDateFromFilename(filePath);

      for (const entry of parsed as RawConversation[]) {
        const conv = parseConversation(entry, opts, fileDate);
        if (!conv) {
          continue;
        }

        conversations.push(conv);

        if (opts?.limit && conversations.length >= opts.limit) {
          return sortConversations(conversations);
        }
      }
    }

    return sortConversations(conversations);
  },
};

function sortConversations(conversations: Conversation[]): Conversation[] {
  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

/**
 * Extract a date from filename patterns like:
 * - history_20250822_120830.json -> Aug 22, 2025 12:08:30
 * - history_20250822.json -> Aug 22, 2025
 */
function extractDateFromFilename(filePath: string): number | null {
  const filename = filePath.split('/').pop() || '';
  
  // Match patterns: history_YYYYMMDD_HHMMSS.json or history_YYYYMMDD.json
  const fullMatch = filename.match(/(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})/);
  if (fullMatch) {
    const [, year, month, day, hour, min, sec] = fullMatch;
    const date = new Date(
      parseInt(year),
      parseInt(month) - 1,
      parseInt(day),
      parseInt(hour),
      parseInt(min),
      parseInt(sec)
    );
    return date.getTime();
  }
  
  const dateMatch = filename.match(/(\d{4})(\d{2})(\d{2})/);
  if (dateMatch) {
    const [, year, month, day] = dateMatch;
    const date = new Date(parseInt(year), parseInt(month) - 1, parseInt(day));
    return date.getTime();
  }
  
  return null;
}

function parseConversation(entry: RawConversation, opts?: ParseOptions, fileDate?: number | null): Conversation | null {
  const messages = extractMessages(entry.chat?.history?.messages);

  if (messages.length === 0) {
    return null;
  }

  // Use conversation timestamps if available, fall back to chat timestamp, then file date, then now
  const createdAtSec = entry.created_at ?? entry.chat?.timestamp;
  const updatedAtSec = entry.updated_at;
  
  // Convert from seconds to milliseconds (these are Unix timestamps in seconds)
  const createdAt = createdAtSec ? createdAtSec * 1000 : (fileDate ?? Date.now());
  const updatedAt = updatedAtSec ? updatedAtSec * 1000 : undefined;

  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const model = deriveModel(messages);

  const conversation: Conversation = {
    externalId: entry.id,
    title: entry.title || undefined,
    createdAt,
    updatedAt,
    model,
    messages,
    metadata: {
      userId: entry.user_id,
      currentId: entry.chat?.history?.currentId,
    },
  };

  return conversation;
}

function extractMessages(messagesMap?: Record<string, TeamsMessage>): Message[] {
  if (!messagesMap) return [];

  const messages: Message[] = [];

  for (const msg of Object.values(messagesMap)) {
    if (!msg.role || !msg.content) {
      continue;
    }

    const content = msg.content.trim();
    if (!content) {
      continue;
    }

    messages.push({
      role: mapRole(msg.role),
      content,
      parts: textOnlyParts(content),
      model: msg.model,
      metadata: {
        id: msg.id,
        parentId: msg.parentId,
        childrenIds: msg.childrenIds,
        done: msg.done,
      },
    });
  }

  messages.sort((a, b) => {
    if (a.metadata?.parentId === b.metadata?.id) return 1;
    if (b.metadata?.parentId === a.metadata?.id) return -1;
    if (a.role === 'user' && b.role === 'assistant') return -1;
    if (a.role === 'assistant' && b.role === 'user') return 1;
    return 0;
  });

  return messages;
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

function deriveModel(messages: Message[]): string | undefined {
  for (const msg of messages) {
    if (msg.model) return msg.model;
  }
  return undefined;
}

async function findConversationFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const candidates: string[] = [];

  const pathStats = await stat(path).catch(() => null);
  if (!pathStats) return candidates;

  if (pathStats.isFile()) {
    if (path.endsWith('.json') && (await looksLikeTeamsExport(path))) {
      candidates.push(path);
    }
    return candidates;
  }

  if (!pathStats.isDirectory()) {
    return candidates;
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries.slice(0, 200)) {
    if (entry.isFile() && entry.name.endsWith('.json')) {
      const filePath = join(path, entry.name);
      if (await looksLikeTeamsExport(filePath)) {
        candidates.push(filePath);
        if (opts.shallowOnly) {
          break;
        }
      }
    }
  }

  return candidates;
}

async function looksLikeTeamsExport(filePath: string): Promise<boolean> {
  try {
    const content = await readFile(filePath, 'utf-8');
    const sample = content.slice(0, 10240);

    if (!sample.startsWith('[')) return false;

    const hasChat = sample.includes('"chat"');
    const hasHistory = sample.includes('"history"');
    const hasMessages = sample.includes('"messages"');
    const hasRole = sample.includes('"role"');
    const hasContent = sample.includes('"content"');

    return hasChat && hasHistory && hasMessages && hasRole && hasContent;
  } catch {
    return false;
  }
}

runAdapter(adapter);
