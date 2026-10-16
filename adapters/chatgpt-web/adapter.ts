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
    name?: string | null;
  };
  create_time?: number;
  update_time?: number;
  recipient?: string;
  content?: {
    content_type?: string;
    parts?: Array<string | { text?: string } | null>;
    text?: string;
    /** reasoning_recap keeps its text in `content`. */
    content?: string;
    /** thoughts content type. */
    thoughts?: Array<{ summary?: string; content?: string }>;
    language?: string;
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
  workspace_id?: string;
  workspace_title?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'chatgpt-web',
      displayName: 'ChatGPT Web',
      version: '1.0.0',
      defaultPaths: DEFAULT_SEARCH_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    // For direct file paths, check if it looks like a ChatGPT export
    const pathStats = await stat(path).catch(() => null);
    if (pathStats?.isFile() && path.endsWith('.json')) {
      const isExport = await looksLikeChatGptExport(path);
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

      // Handle both array format (conversations.json) and single object format (ChatGPT-*.json)
      const entries: RawConversation[] = Array.isArray(parsed) 
        ? parsed 
        : [parsed as RawConversation];

      for (const entry of entries) {
        // Skip if entry doesn't have the expected structure
        if (!entry || typeof entry !== 'object' || !entry.mapping) {
          continue;
        }

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
  const messages = extractMessages(entry.mapping, entry.current_node);

  if (messages.length === 0) {
    return null;
  }

  const createdAtSec = entry.create_time ?? earliestMessageTime(messages) ?? 0;
  const createdAt = Math.floor(createdAtSec * 1000);

  const updatedAtSec = entry.update_time ?? latestMessageTime(messages) ?? undefined;
  const updatedAt = updatedAtSec ? Math.floor(updatedAtSec * 1000) : undefined;

  // Check both created and updated time so modified sessions are re-imported
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
    workspace: entry.workspace_title ?? entry.workspace_id,
    messages,
    metadata: {
      currentNode: entry.current_node,
      workspaceId: entry.workspace_id,
      workspaceTitle: entry.workspace_title,
    },
  };

  return conversation;
}

function extractMessages(mapping?: ConversationMap, currentNode?: string): Message[] {
  if (!mapping) return [];

  const messages: Message[] = [];
  const chain = activeChain(mapping, currentNode);
  const entries = chain ?? sortNodesByTime(Object.values(mapping));

  // A `code` message and the `execution_output` that follows it are separate
  // nodes with no explicit id link; remember the last code call so the output
  // can be associated with it.
  let lastToolCallId: string | undefined;

  for (const node of entries) {
    const msg = node.message;
    if (!msg || !msg.author?.role) continue;

    const built = buildMessageContent(msg, lastToolCallId);
    if (built.toolCallId) lastToolCallId = built.toolCallId;
    if (!built.content && !built.parts) continue;

    const createdAt = msg.create_time ? Math.floor(msg.create_time * 1000) : undefined;

    messages.push({
      role: mapRole(msg.author.role),
      content: built.content,
      parts: built.parts ?? textOnlyParts(built.content),
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

  return messages;
}

/**
 * Walk current_node -> parents back to the root. ChatGPT keeps every
 * regenerated/edited variant in `mapping`; only the chain ending at
 * `current_node` is the transcript the user sees.
 */
function activeChain(mapping: ConversationMap, currentNode?: string): ConversationNode[] | null {
  if (!currentNode || !mapping[currentNode]) return null;
  const chain: ConversationNode[] = [];
  const seen = new Set<string>();
  let id: string | undefined = currentNode;
  while (id && mapping[id] && !seen.has(id)) {
    seen.add(id);
    chain.push(mapping[id]);
    id = mapping[id].parent ?? undefined;
  }
  return chain.reverse();
}

function sortNodesByTime(nodes: ConversationNode[]): ConversationNode[] {
  return [...nodes].sort((a, b) => (a.message?.create_time ?? 0) - (b.message?.create_time ?? 0));
}

function buildMessageContent(
  msg: ConversationMessage,
  lastToolCallId: string | undefined
): { content: string; parts?: CanonPart[]; toolCallId?: string } {
  const content = msg.content;
  const type = content?.content_type;
  const parts: CanonPart[] = [];
  const chunks: string[] = [];

  if (type === 'thoughts' && Array.isArray(content?.thoughts)) {
    const text = content.thoughts
      .map(thought => [thought.summary, thought.content].filter(Boolean).join(': '))
      .filter(Boolean)
      .join('\n')
      .trim();
    if (text) {
      parts.push(thinkingPart(text));
      chunks.push(text);
    }
  } else if (type === 'reasoning_recap') {
    const text = (typeof content.content === 'string' ? content.content : content.text ?? '').trim();
    if (text) {
      parts.push(thinkingPart(text));
      chunks.push(text);
    }
  } else if (type === 'code') {
    const code = (content.text ?? '').trim();
    const name = msg.recipient && msg.recipient !== 'all' ? msg.recipient : 'python';
    const callId = msg.id ?? 'code';
    parts.push(toolCallPart(callId, name, { code, language: content.language }));
    if (code) chunks.push(code);
    return { content: chunks.join('\n').trim(), parts, toolCallId: callId };
  } else if (type === 'execution_output') {
    const output = (content.text ?? '').trim();
    parts.push(toolResultPart(lastToolCallId ?? 'unknown', output, { name: 'python' }));
    if (output) chunks.push(output);
  } else {
    const text = extractContent(content);
    if (text) {
      parts.push(textPart(text));
      chunks.push(text);
    }
  }

  return { content: chunks.join('\n').trim(), parts: parts.length ? parts : undefined };
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
    // Accept any JSON file that looks like a ChatGPT export
    if (path.endsWith('.json') && (await looksLikeChatGptExport(path))) {
      candidates.push(path);
    }
    return candidates;
  }

  if (!pathStats.isDirectory()) {
    return candidates;
  }

  // First check for conversations.json directly
  const direct = join(path, 'conversations.json');
  const directStats = await stat(direct).catch(() => null);
  if (directStats?.isFile()) {
    candidates.push(direct);
    return candidates;
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (entry.isFile() && entry.name.endsWith('.json')) {
      const filePath = join(path, entry.name);
      // Prioritize conversations.json but also check other JSON files
      if (entry.name === 'conversations.json' || (await looksLikeChatGptExport(filePath))) {
        candidates.push(filePath);
        if (opts.shallowOnly) {
          break;
        }
      }
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

/**
 * Check if a JSON file looks like a ChatGPT export by examining its structure.
 * ChatGPT exports can be:
 * 1. An array of conversations (conversations.json from bulk export)
 * 2. A single conversation object (individual export files like ChatGPT-*.json)
 */
async function looksLikeChatGptExport(filePath: string): Promise<boolean> {
  try {
    const content = await readFile(filePath, 'utf-8');
    // Read just enough to detect the structure (first 10KB should be enough)
    const sample = content.slice(0, 10240);
    
    // Check for ChatGPT export markers
    // Can be an array of conversations OR a single conversation object
    const startsWithArray = sample.trimStart().startsWith('[');
    const startsWithObject = sample.trimStart().startsWith('{');
    
    if (!startsWithArray && !startsWithObject) return false;
    
    // Look for characteristic ChatGPT export fields
    const hasMapping = sample.includes('"mapping"');
    const hasCreateTime = sample.includes('"create_time"');
    const hasAuthorRole = sample.includes('"author"') && sample.includes('"role"');
    
    return hasMapping && hasCreateTime && hasAuthorRole;
  } catch {
    return false;
  }
}

runAdapter(adapter);
