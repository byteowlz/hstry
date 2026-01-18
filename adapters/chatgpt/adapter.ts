/**
 * ChatGPT export adapter for hstry
 *
 * Parses OpenAI ChatGPT export data from conversations.json
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, join } from 'path';
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

type ConversationMap = Record<string, ConversationNode>;

interface ConversationNode {
  id?: string;
  parent?: string | null;
  children?: string[];
  message?: ConversationMessage | null;
}

interface ConversationMessage {
  id?: string;
  author?: {
    role?: string;
  };
  create_time?: number;
  update_time?: number;
  content?: {
    content_type?: string;
    parts?: Array<string | { text?: string } | null>;
    text?: string;
  } | null;
  metadata?: Record<string, unknown> | null;
}

interface RawConversation {
  id?: string;
  title?: string;
  create_time?: number;
  update_time?: number;
  mapping?: ConversationMap;
  current_node?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'chatgpt',
      displayName: 'ChatGPT Export',
      version: '1.0.0',
      defaultPaths: DEFAULT_SEARCH_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
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

      for (const entry of parsed as RawConversation[]) {
        const conv = parseConversation(entry, opts);
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

    if (opts.format !== 'chatgpt') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const exportData = conversations.map(conv => buildChatGptExport(conv));
    return {
      format: 'chatgpt',
      content: JSON.stringify(exportData, null, opts.pretty ? 2 : 0),
      mimeType: 'application/json',
      metadata: {
        filename: 'conversations.json',
      },
    };
  },
};

function sortConversations(conversations: Conversation[]): Conversation[] {
  conversations.sort((a, b) => b.createdAt - a.createdAt);
  return conversations;
}

function parseConversation(entry: RawConversation, opts?: ParseOptions): Conversation | null {
  const messages = extractMessages(entry.mapping);

  if (messages.length === 0) {
    return null;
  }

  const createdAtSec = entry.create_time ?? earliestMessageTime(messages) ?? 0;
  const createdAt = createdAtSec * 1000;

  if (opts?.since && createdAt < opts.since) {
    return null;
  }

  const updatedAtSec = entry.update_time ?? latestMessageTime(messages) ?? undefined;

  const model = deriveModel(messages);

  const conversation: Conversation = {
    externalId: entry.id,
    title: entry.title || undefined,
    createdAt,
    updatedAt: updatedAtSec ? updatedAtSec * 1000 : undefined,
    model,
    messages,
    metadata: {
      currentNode: entry.current_node,
    },
  };

  return conversation;
}

function extractMessages(mapping?: ConversationMap): Message[] {
  if (!mapping) return [];

  const messages: Message[] = [];

  for (const node of Object.values(mapping)) {
    const msg = node.message;
    if (!msg || !msg.author?.role) {
      continue;
    }

    const content = extractContent(msg.content);
    if (!content) {
      continue;
    }

    const createdAt = msg.create_time ? msg.create_time * 1000 : undefined;

    messages.push({
      role: mapRole(msg.author.role),
      content,
      createdAt,
      model: extractModel(msg.metadata),
      metadata: {
        id: msg.id,
        nodeId: node.id,
        parentId: node.parent ?? undefined,
        contentType: msg.content?.content_type,
      },
    });
  }

  messages.sort((a, b) => {
    const aTime = a.createdAt ?? 0;
    const bTime = b.createdAt ?? 0;
    if (aTime !== bTime) return aTime - bTime;
    return a.content.localeCompare(b.content);
  });

  return messages;
}

function extractContent(content: ConversationMessage['content']): string {
  if (!content) return '';
  if (typeof content.text === 'string' && content.text.trim()) {
    return content.text.trim();
  }

  const parts = content.parts;
  if (Array.isArray(parts)) {
    const textParts = parts
      .map(part => {
        if (typeof part === 'string') return part;
        if (part && typeof part.text === 'string') return part.text;
        return '';
      })
      .map(part => part.trim())
      .filter(Boolean);

    if (textParts.length > 0) {
      return textParts.join('\n');
    }
  }

  return '';
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

function extractModel(metadata?: Record<string, unknown> | null): string | undefined {
  if (!metadata) return undefined;
  const value = metadata['model_slug'] ?? metadata['model'] ?? metadata['model_name'];
  return typeof value === 'string' ? value : undefined;
}

function deriveModel(messages: Message[]): string | undefined {
  for (const msg of messages) {
    if (msg.model) return msg.model;
  }
  return undefined;
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

function buildChatGptExport(conv: Conversation): RawConversation {
  const mapping: ConversationMap = {};
  const rootId = 'root';
  mapping[rootId] = {
    id: rootId,
    parent: null,
    children: [],
  };

  let previousId = rootId;
  conv.messages.forEach((msg, index) => {
    const nodeId = `msg-${index + 1}`;
    const createTime = (msg.createdAt ?? conv.createdAt) / 1000;
    const node: ConversationNode = {
      id: nodeId,
      parent: previousId,
      children: [],
      message: {
        id: nodeId,
        author: { role: msg.role },
        content: {
          content_type: 'text',
          parts: [msg.content],
        },
        create_time: createTime,
        metadata: msg.model ? { model_slug: msg.model } : undefined,
      },
    };
    mapping[previousId]?.children?.push(nodeId);
    mapping[nodeId] = node;
    previousId = nodeId;
  });

  const createdAt = conv.createdAt / 1000;
  const updatedAt = (conv.updatedAt ?? conv.createdAt) / 1000;

  return {
    id: conv.externalId ?? `chatgpt-${Date.now()}`,
    title: conv.title ?? 'Conversation',
    create_time: createdAt,
    update_time: updatedAt,
    mapping,
    current_node: previousId,
  };
}

async function findConversationFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const candidates: string[] = [];

  const pathStats = await stat(path).catch(() => null);
  if (!pathStats) return candidates;

  if (pathStats.isFile()) {
    if (basename(path) === 'conversations.json') {
      candidates.push(path);
    }
    return candidates;
  }

  if (!pathStats.isDirectory()) {
    return candidates;
  }

  const direct = join(path, 'conversations.json');
  const directStats = await stat(direct).catch(() => null);
  if (directStats?.isFile()) {
    candidates.push(direct);
    return candidates;
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries.slice(0, 200)) {
    if (entry.isFile() && entry.name === 'conversations.json') {
      candidates.push(join(path, entry.name));
      break;
    }

    if (!entry.isDirectory()) continue;

    const childFile = join(path, entry.name, 'conversations.json');
    const childStats = await stat(childFile).catch(() => null);
    if (childStats?.isFile()) {
      candidates.push(childFile);
      if (opts.shallowOnly) {
        break;
      }
    }
  }

  return candidates;
}

runAdapter(adapter);
