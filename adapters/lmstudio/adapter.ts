/**
 * LM Studio adapter for hstry
 *
 * Parses LM Studio conversation JSON files from ~/.cache/lm-studio/conversations/
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
import { runAdapter, textOnlyParts } from '../types/index.ts';

// Platform-specific default paths
const DEFAULT_PATHS = [
  join(homedir(), '.cache', 'lm-studio', 'conversations'),
  // Windows
  join(homedir(), 'AppData', 'Local', 'LM Studio', 'conversations'),
  join(homedir(), '.lmstudio', 'conversations'),
];

interface LMStudioConversation {
  id?: string;
  title?: string;
  createdAt?: string | number;
  updatedAt?: string | number;
  modelIdentifier?: string;
  messages?: LMStudioMessageEntry[];
  // Alternate formats
  chat?: LMStudioMessageEntry[];
  history?: LMStudioMessageEntry[];
}

interface LMStudioMessageEntry {
  role?: string;
  content?: string | LMStudioContentPart[];
  text?: string;
  timestamp?: string | number;
  createdAt?: string | number;
  model?: string;
  versions?: LMStudioMessageVersion[];
  currentlySelected?: number;
}

interface LMStudioMessageVersion {
  type?: string;
  role?: string;
  content?: string | LMStudioContentPart[];
  steps?: LMStudioMessageStep[];
  senderInfo?: {
    senderName?: string;
  };
  genInfo?: {
    identifier?: string;
    indexedModelIdentifier?: string;
  };
}

interface LMStudioMessageStep {
  type?: string;
  content?: LMStudioContentPart[];
}

interface LMStudioContentPart {
  type?: string;
  text?: string;
  image_url?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'lmstudio',
      displayName: 'LM Studio',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findConversationFiles(path, { shallowOnly: true });
    if (files.length === 0) return null;

    // Check if any file looks like an LM Studio conversation
    for (const file of files.slice(0, 3)) {
      const content = await readFile(file, 'utf-8').catch(() => '');
      try {
        const data = JSON.parse(content);
        if (isLMStudioConversation(data)) {
          return 0.85;
        }
      } catch {
        continue;
      }
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findConversationFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const raw = await readFile(filePath, 'utf-8').catch(() => null);
      if (!raw) continue;

      let data: unknown;
      try {
        data = JSON.parse(raw);
      } catch {
        continue;
      }

      const conv = parseConversation(data, filePath, opts);
      if (conv) {
        conversations.push(conv);

        if (opts?.limit && conversations.length >= opts.limit) {
          break;
        }
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

    if (opts.format !== 'lmstudio') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = conversations.map(conv => {
      const data = buildLMStudioFormat(conv);
      const name = `${conv.externalId ?? 'conversation'}.json`;
      return { path: name, content: JSON.stringify(data, null, 2) };
    });

    return {
      format: 'lmstudio',
      files,
      mimeType: 'application/json',
    };
  },
};

function isLMStudioConversation(data: unknown): boolean {
  if (!data || typeof data !== 'object') return false;
  const obj = data as Record<string, unknown>;

  // Check for LM Studio specific fields
  if (obj.modelIdentifier) return true;
  if (obj.messages && Array.isArray(obj.messages)) {
    const msgs = obj.messages as unknown[];
    if (msgs.length > 0 && msgs[0] && typeof msgs[0] === 'object') {
      const first = msgs[0] as Record<string, unknown>;
      if (first.role && first.content !== undefined) return true;
      if (Array.isArray(first.versions)) return true;
    }
  }
  if (obj.chat && Array.isArray(obj.chat)) return true;
  if (obj.history && Array.isArray(obj.history)) return true;

  return false;
}

function parseConversation(
  data: unknown,
  filePath: string,
  opts?: ParseOptions
): Conversation | null {
  if (!data || typeof data !== 'object') return null;
  const raw = data as LMStudioConversation;

  const messages = extractMessages(raw);
  if (messages.length === 0) return null;

  const timestamps = messages
    .map(m => m.createdAt)
    .filter((ts): ts is number => typeof ts === 'number');

  const createdAt = parseTimestamp(raw.createdAt) ??
    (timestamps.length > 0 ? Math.min(...timestamps) : Date.now());
  const updatedAt = parseTimestamp(raw.updatedAt) ??
    (timestamps.length > 0 ? Math.max(...timestamps) : undefined);

  // Check both created and updated time for incremental sync
  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const model = raw.modelIdentifier ?? deriveModel(messages);

  return {
    externalId: raw.id ?? basename(filePath, extname(filePath)),
    title: raw.title ?? deriveTitle(messages),
    createdAt,
    updatedAt,
    model,
    messages,
    metadata: {
      file: filePath,
      source: 'lm-studio',
    },
  };
}

function extractMessages(raw: LMStudioConversation): Message[] {
  const msgArray = raw.messages ?? raw.chat ?? raw.history ?? [];
  if (!Array.isArray(msgArray)) return [];

  const messages: Message[] = [];

  for (const entry of msgArray) {
    if (!entry || typeof entry !== 'object') continue;
    const msg = entry as LMStudioMessageEntry;

    const version = selectVersion(msg);
    const role = version?.role ?? msg.role;
    if (!role) continue;

    const content = extractMessageContent(msg, version);
    if (!content) continue;

    const createdAt = parseTimestamp(msg.timestamp ?? msg.createdAt);
    const model =
      version?.senderInfo?.senderName ??
      version?.genInfo?.identifier ??
      version?.genInfo?.indexedModelIdentifier ??
      msg.model;

    messages.push({
      role: mapRole(role),
      content,
      parts: textOnlyParts(content),
      createdAt,
      model,
    });
  }

  return messages;
}

function selectVersion(message: LMStudioMessageEntry): LMStudioMessageVersion | undefined {
  const versions = message.versions ?? [];
  if (versions.length === 0) return undefined;
  const index = message.currentlySelected ?? 0;
  return versions[index] ?? versions[0];
}

function extractMessageContent(
  message: LMStudioMessageEntry,
  version?: LMStudioMessageVersion
): string {
  if (version?.content) {
    const content = extractContentFromValue(version.content);
    if (content) return content;
  }

  if (version?.steps) {
    const content = extractContentFromSteps(version.steps);
    if (content) return content;
  }

  if (message.text) return message.text.trim();
  return extractContentFromValue(message.content);
}

function extractContentFromSteps(steps: LMStudioMessageStep[]): string {
  const parts: string[] = [];
  for (const step of steps) {
    if (step.type !== 'contentBlock' || !step.content) continue;
    const content = extractContentFromValue(step.content);
    if (content) parts.push(content);
  }
  return parts.join('\n').trim();
}

function extractContentFromValue(
  content?: string | LMStudioContentPart[]
): string {
  if (!content) return '';

  if (typeof content === 'string') return content.trim();

  if (Array.isArray(content)) {
    const parts: string[] = [];
    for (const part of content) {
      if (typeof part === 'string') {
        parts.push(part);
      } else if (part && typeof part === 'object') {
        if (part.type === 'text' && part.text) {
          parts.push(part.text);
        } else if (part.type === 'image_url' && part.image_url) {
          parts.push(`[image: ${part.image_url}]`);
        }
      }
    }
    return parts.join('\n').trim();
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
    case 'model':
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

function parseTimestamp(value?: string | number): number | undefined {
  if (!value) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
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

function buildLMStudioFormat(conv: Conversation): LMStudioConversation {
  return {
    id: conv.externalId,
    title: conv.title,
    createdAt: new Date(conv.createdAt).toISOString(),
    updatedAt: conv.updatedAt ? new Date(conv.updatedAt).toISOString() : undefined,
    modelIdentifier: conv.model,
    messages: conv.messages.map(msg => ({
      role: msg.role,
      content: msg.content,
      timestamp: msg.createdAt ? new Date(msg.createdAt).toISOString() : undefined,
      model: msg.model,
    })),
  };
}

async function findConversationFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const files: string[] = [];

  const stats = await stat(path).catch(() => null);
  if (!stats) return files;

  if (stats.isFile()) {
    if (extname(path) === '.json') files.push(path);
    return files;
  }

  if (!stats.isDirectory()) return files;

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(path, entry.name);

    if (entry.isFile() && extname(entry.name) === '.json') {
      files.push(entryPath);
      continue;
    }

    if (entry.isDirectory() && !opts.shallowOnly) {
      // Check subdirectories for JSON files
      const nested = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const child of nested) {
        if (child.isFile() && extname(child.name) === '.json') {
          files.push(join(entryPath, child.name));
        }
      }
    }
  }

  return files;
}

runAdapter(adapter);
