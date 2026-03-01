/**
 * Jan.ai adapter for hstry
 *
 * Parses Jan conversation threads from ~/jan/threads/
 * Each thread is a directory with messages.jsonl
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
} from '../types/index.ts';

// Platform-specific paths
const DEFAULT_PATHS = [
  // Default jan data folder
  join(homedir(), 'jan', 'threads'),
  // macOS
  join(homedir(), 'Library', 'Application Support', 'Jan', 'data', 'threads'),
  // Linux
  join(homedir(), '.config', 'Jan', 'data', 'threads'),
  // Windows
  join(process.env.APPDATA || '', 'Jan', 'data', 'threads'),
];

interface ThreadMeta {
  id?: string;
  object?: string;
  title?: string;
  assistants?: { assistant_id?: string; model?: string }[];
  created?: number;
  updated?: number;
  created_at?: number;
  updated_at?: number;
  metadata?: Record<string, unknown>;
}

interface JanMessage {
  id?: string;
  object?: string;
  thread_id?: string;
  role?: string;
  content?: JanContent[];
  status?: string;
  created?: number;
  created_at?: number;
  updated?: number;
  updated_at?: number;
  model?: string;
  metadata?: Record<string, unknown>;
}

interface JanContent {
  type?: string;
  text?: { value?: string; annotations?: unknown[] };
  image_url?: { url?: string; detail?: string };
  tool_call_id?: string;
  tool_name?: string;
  input?: unknown;
  output?: unknown;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'jan',
      displayName: 'Jan.ai',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const threads = await findThreadDirectories(path);
    if (threads.length === 0) return null;

    // Check if any thread has messages.jsonl
    for (const threadPath of threads.slice(0, 3)) {
      const messagesPath = join(threadPath, 'messages.jsonl');
      const msgStats = await stat(messagesPath).catch(() => null);
      if (msgStats?.isFile()) return 0.9;
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const threads = await findThreadDirectories(path);
    if (threads.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const threadPath of threads) {
      const conv = await parseThread(threadPath, opts);
      if (conv && conv.messages.length > 0) {
        conversations.push(conv);

        if (opts?.limit && conversations.length >= opts.limit) break;
      }
    }

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return conversations;
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

    if (opts.format !== 'jan') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    // Export as jan thread structure
    const files = buildJanFiles(conversations);
    return {
      format: 'jan',
      files,
      mimeType: 'application/json',
      metadata: { root: 'threads/' },
    };
  },
};

async function parseThread(threadPath: string, opts?: ParseOptions): Promise<Conversation | null> {
  const threadId = basename(threadPath);

  // Read thread metadata (thread.json)
  let meta: ThreadMeta = {};
  const metaPath = join(threadPath, 'thread.json');
  const metaContent = await readFile(metaPath, 'utf-8').catch(() => null);
  if (metaContent) {
    try {
      meta = JSON.parse(metaContent);
    } catch { /* ignore */ }
  }

  // Read messages (messages.jsonl)
  const messagesPath = join(threadPath, 'messages.jsonl');
  const messagesContent = await readFile(messagesPath, 'utf-8').catch(() => null);
  if (!messagesContent) return null;

  const messages: Message[] = [];
  let firstTimestamp: number | undefined;
  let lastTimestamp: number | undefined;

  const lines = messagesContent.split(/\r?\n/).filter(line => line.trim());
  for (const line of lines) {
    try {
      const msg = JSON.parse(line) as JanMessage;
      const parsed = parseMessage(msg);
      if (parsed) {
        messages.push(parsed);

        const ts = parsed.createdAt;
        if (ts) {
          if (!firstTimestamp || ts < firstTimestamp) firstTimestamp = ts;
          if (!lastTimestamp || ts > lastTimestamp) lastTimestamp = ts;
        }
      }
    } catch { /* skip invalid lines */ }
  }

  if (messages.length === 0) return null;

  const createdAt = parseTimestamp(meta.created ?? meta.created_at) ?? firstTimestamp ?? Date.now();
  const updatedAt = parseTimestamp(meta.updated ?? meta.updated_at) ?? lastTimestamp;

  // Check incremental sync
  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const model = meta.assistants?.[0]?.model ?? deriveModel(messages);

  return {
    externalId: meta.id ?? threadId,
    title: meta.title ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    model,
    messages,
    metadata: {
      threadPath,
      assistants: meta.assistants,
      ...meta.metadata,
    },
  };
}

function parseMessage(msg: JanMessage): Message | null {
  if (!msg.role) return null;

  const extracted = extractContentParts(msg.content, msg.id);
  if (!extracted) return null;

  const parts = extracted.parts ?? textOnlyParts(extracted.content);

  return {
    role: mapRole(msg.role),
    content: extracted.content,
    parts,
    createdAt: parseTimestamp(msg.created ?? msg.created_at),
    model: msg.model,
    toolCalls: extracted.toolCalls,
    metadata: {
      id: msg.id,
      status: msg.status,
      ...msg.metadata,
    },
  };
}

function extractContentParts(
  content?: JanContent[],
  messageId?: string
): { content: string; parts?: CanonPart[]; toolCalls?: ToolCall[] } | null {
  if (!content || !Array.isArray(content)) return null;

  const parts: CanonPart[] = [];
  const textBlocks: string[] = [];
  const toolCalls: ToolCall[] = [];

  content.forEach((item, index) => {
    if (!item) return;

    if (item.type === 'text' && item.text?.value) {
      textBlocks.push(item.text.value);
      parts.push(textPart(item.text.value));
      return;
    }

    if (item.type === 'reasoning' && item.text?.value) {
      textBlocks.push(item.text.value);
      parts.push(thinkingPart(item.text.value));
      return;
    }

    if (item.type === 'image_url' && item.image_url?.url) {
      const label = `[image: ${item.image_url.url}]`;
      textBlocks.push(label);
      parts.push(textPart(label));
      return;
    }

    if (item.type === 'tool_call') {
      const toolName = item.tool_name ?? 'tool';
      const toolCallId = item.tool_call_id ?? `${messageId ?? 'tool'}-${index}`;

      if (item.input !== undefined) {
        parts.push(toolCallPart(toolCallId, toolName, item.input));
      }
      if (item.output !== undefined) {
        parts.push(toolResultPart(toolCallId, item.output, { name: toolName }));
      }

      toolCalls.push({
        toolName,
        input: item.input,
        output: typeof item.output === 'string' ? item.output : undefined,
        status: item.output !== undefined ? 'success' : 'pending',
      });
    }
  });

  let text = textBlocks.join('\n').trim();
  if (!text && toolCalls.length > 0) {
    text = toolCalls
      .map(call => `Tool call: ${call.toolName}`)
      .join('\n');
  }

  if (!text && parts.length === 0) return null;

  return {
    content: text,
    parts: parts.length > 0 ? parts : undefined,
    toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
  };
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

function parseTimestamp(value?: number): number | undefined {
  if (!value) return undefined;
  // Jan uses milliseconds
  return value < 1e12 ? value * 1000 : value;
}

function deriveModel(messages: Message[]): string | undefined {
  for (const msg of messages) {
    if (msg.model) return msg.model;
  }
  return undefined;
}

function deriveTitle(messages: Message[]): string | undefined {
  const firstUser = messages.find(m => m.role === 'user');
  if (!firstUser?.content) return undefined;
  const text = firstUser.content.slice(0, 80);
  return text.length < firstUser.content.length ? `${text}...` : text;
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

function buildJanFiles(conversations: Conversation[]): { path: string; content: string }[] {
  const files: { path: string; content: string }[] = [];

  for (const conv of conversations) {
    const threadId = conv.externalId ?? `thread_${Date.now()}`;
    const threadDir = `threads/${threadId}`;

    // thread.json
    const threadMeta: ThreadMeta = {
      id: threadId,
      object: 'thread',
      title: conv.title,
      created: Math.floor(conv.createdAt / 1000),
      updated: conv.updatedAt ? Math.floor(conv.updatedAt / 1000) : undefined,
      assistants: conv.model ? [{ model: conv.model }] : [],
    };
    files.push({
      path: `${threadDir}/thread.json`,
      content: JSON.stringify(threadMeta, null, 2),
    });

    // messages.jsonl
    const messageLines = conv.messages.map((msg, idx) => {
      const contentParts: JanContent[] = [];

      if (msg.parts && msg.parts.length > 0) {
        for (const part of msg.parts) {
          if (part.type === 'text') {
            contentParts.push({
              type: 'text',
              text: { value: part.text, annotations: [] },
            });
          } else if (part.type === 'thinking') {
            contentParts.push({
              type: 'reasoning',
              text: { value: part.text, annotations: [] },
            });
          } else if (part.type === 'tool_call') {
            contentParts.push({
              type: 'tool_call',
              tool_call_id: part.toolCallId,
              tool_name: part.name,
              input: part.input,
            });
          } else if (part.type === 'tool_result') {
            contentParts.push({
              type: 'tool_call',
              tool_call_id: part.toolCallId,
              tool_name: part.name,
              output: part.output,
            });
          }
        }
      }

      if (contentParts.length === 0) {
        contentParts.push({
          type: 'text',
          text: { value: msg.content, annotations: [] },
        });
      }

      const janMsg: JanMessage = {
        id: `msg_${idx}`,
        object: 'thread.message',
        thread_id: threadId,
        role: msg.role,
        content: contentParts,
        status: 'ready',
        created: msg.createdAt ? Math.floor(msg.createdAt / 1000) : undefined,
        model: msg.model,
      };
      return JSON.stringify(janMsg);
    });
    files.push({
      path: `${threadDir}/messages.jsonl`,
      content: messageLines.join('\n') + '\n',
    });
  }

  return files;
}

async function findThreadDirectories(path: string): Promise<string[]> {
  const threads: string[] = [];
  const stats = await stat(path).catch(() => null);
  if (!stats?.isDirectory()) return threads;

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    
    const threadPath = join(path, entry.name);
    // Check if it looks like a jan thread (has messages.jsonl or thread.json)
    const msgPath = join(threadPath, 'messages.jsonl');
    const metaPath = join(threadPath, 'thread.json');
    
    const [msgStats, metaStats] = await Promise.all([
      stat(msgPath).catch(() => null),
      stat(metaPath).catch(() => null),
    ]);

    if (msgStats?.isFile() || metaStats?.isFile()) {
      threads.push(threadPath);
    }
  }

  return threads;
}

runAdapter(adapter);
