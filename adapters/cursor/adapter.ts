/**
 * Cursor adapter for hstry
 *
 * Parses Cursor chat history from VSCode SQLite state files
 * Location: ~/Library/Application Support/Cursor/User/workspaceStorage/<hash>/state.vscdb
 */

import { readdir, stat } from 'fs/promises';
import { basename, join } from 'path';
import { homedir } from 'os';
import type {
  Adapter,
  AdapterInfo,
  Conversation,
  Message,
  ParseOptions,
} from '../types/index.ts';
import { runAdapter, textOnlyParts } from '../types/index.ts';

// Dynamic import for better-sqlite3 (optional dependency)
let Database: typeof import('better-sqlite3') | null = null;
try {
  Database = (await import('better-sqlite3')).default;
} catch {
  // SQLite not available - adapter will return empty results
}

// Platform-specific paths
const DEFAULT_PATHS = (() => {
  const home = homedir();
  switch (process.platform) {
    case 'darwin':
      return [join(home, 'Library', 'Application Support', 'Cursor', 'User', 'workspaceStorage')];
    case 'win32':
      return [join(process.env.APPDATA || '', 'Cursor', 'User', 'workspaceStorage')];
    default: // Linux
      return [join(home, '.config', 'Cursor', 'User', 'workspaceStorage')];
  }
})();

interface StateRow {
  key: string;
  value: string;
}

interface CursorChatData {
  tabs?: CursorTab[];
  currentTabId?: string;
}

interface CursorTab {
  id?: string;
  title?: string;
  createdAt?: number;
  lastUpdatedAt?: number;
  bubbles?: CursorBubble[];
}

interface CursorBubble {
  type?: string;
  text?: string;
  rawText?: string;
  state?: string;
  modelType?: string;
  timingInfo?: { startTime?: number; endTime?: number };
}

interface CursorPrompt {
  id?: string;
  prompt?: string;
  response?: string;
  createdAt?: number;
  conversationId?: string;
  model?: string;
}

const CHAT_DATA_KEY = 'workbench.panel.aichat.view.aichat.chatdata';
const PROMPTS_KEY = 'aiService.prompts';

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'cursor',
      displayName: 'Cursor',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!Database) return null;

    const dbFiles = await findStateFiles(path);
    if (dbFiles.length === 0) return null;

    // Check if any database has cursor chat data
    for (const dbPath of dbFiles.slice(0, 3)) {
      try {
        const db = new Database(dbPath, { readonly: true });
        const row = db.prepare("SELECT value FROM ItemTable WHERE key = ?").get(CHAT_DATA_KEY) as StateRow | undefined;
        db.close();
        if (row) return 0.9;
      } catch { /* continue */ }
    }

    return null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    if (!Database) return [];

    const dbFiles = await findStateFiles(path);
    if (dbFiles.length === 0) return [];

    const conversations: Conversation[] = [];
    const seenIds = new Set<string>();

    for (const dbPath of dbFiles) {
      const workspaceId = basename(dbPath.replace(/\/state\.vscdb$/, ''));
      const convs = await parseStateDb(dbPath, workspaceId, opts);
      
      for (const conv of convs) {
        // Avoid duplicates
        const key = conv.externalId ?? `${conv.createdAt}-${conv.title}`;
        if (seenIds.has(key)) continue;
        seenIds.add(key);
        
        conversations.push(conv);

        if (opts?.limit && conversations.length >= opts.limit) break;
      }

      if (opts?.limit && conversations.length >= opts.limit) break;
    }

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return opts?.limit ? conversations.slice(0, opts.limit) : conversations;
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

    // Cursor doesn't have a good import format, export as generic JSON
    if (opts.format === 'cursor') {
      return {
        format: 'cursor',
        content: JSON.stringify(conversations, null, opts.pretty ? 2 : 0),
        mimeType: 'application/json',
      };
    }

    throw new Error(`Unsupported export format: ${opts.format}`);
  },
};

async function parseStateDb(dbPath: string, workspaceId: string, opts?: ParseOptions): Promise<Conversation[]> {
  const conversations: Conversation[] = [];

  try {
    const db = new Database(dbPath, { readonly: true });

    // Try to get chat data
    const chatRow = db.prepare("SELECT value FROM ItemTable WHERE key = ?").get(CHAT_DATA_KEY) as StateRow | undefined;
    if (chatRow) {
      const chatConvs = parseChatData(chatRow.value, workspaceId, opts);
      conversations.push(...chatConvs);
    }

    // Also try prompts (may have additional data)
    const promptsRow = db.prepare("SELECT value FROM ItemTable WHERE key = ?").get(PROMPTS_KEY) as StateRow | undefined;
    if (promptsRow) {
      const promptConvs = parsePrompts(promptsRow.value, workspaceId, opts);
      conversations.push(...promptConvs);
    }

    db.close();
  } catch (err) {
    console.error('Error reading Cursor database:', err);
  }

  return conversations;
}

function parseChatData(value: string, workspaceId: string, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];

  try {
    const data = JSON.parse(value) as CursorChatData;
    if (!data.tabs) return conversations;

    for (const tab of data.tabs) {
      const messages = parseTabBubbles(tab.bubbles);
      if (messages.length === 0) continue;

      const createdAt = tab.createdAt ?? messages[0].createdAt ?? Date.now();
      const updatedAt = tab.lastUpdatedAt ?? messages[messages.length - 1].createdAt;

      // Check incremental sync
      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      conversations.push({
        externalId: tab.id,
        title: tab.title ?? deriveTitle(messages),
        createdAt,
        updatedAt,
        workspace: workspaceId,
        messages,
        metadata: {
          source: 'cursor-chat',
          tabId: tab.id,
        },
      });
    }
  } catch { /* ignore parse errors */ }

  return conversations;
}

function parseTabBubbles(bubbles?: CursorBubble[]): Message[] {
  if (!bubbles) return [];

  const messages: Message[] = [];

  for (const bubble of bubbles) {
    if (!bubble.type) continue;

    const content = bubble.text ?? bubble.rawText;
    if (!content) continue;

    const role = bubble.type === 'user' ? 'user' : 'assistant';
    const createdAt = bubble.timingInfo?.startTime;

    messages.push({
      role,
      content,
      parts: textOnlyParts(content),
      createdAt,
      model: bubble.modelType,
      metadata: {
        state: bubble.state,
      },
    });
  }

  return messages;
}

function parsePrompts(value: string, workspaceId: string, opts?: ParseOptions): Conversation[] {
  const conversations: Conversation[] = [];

  try {
    const prompts = JSON.parse(value) as CursorPrompt[];
    if (!Array.isArray(prompts)) return conversations;

    // Group prompts by conversationId
    const grouped = new Map<string, CursorPrompt[]>();
    for (const prompt of prompts) {
      const convId = prompt.conversationId ?? 'default';
      if (!grouped.has(convId)) {
        grouped.set(convId, []);
      }
      grouped.get(convId)!.push(prompt);
    }

    for (const [convId, convPrompts] of grouped) {
      const messages: Message[] = [];
      let firstTime: number | undefined;
      let lastTime: number | undefined;

      for (const prompt of convPrompts) {
        if (prompt.prompt) {
          messages.push({
            role: 'user',
            content: prompt.prompt,
            parts: textOnlyParts(prompt.prompt),
            createdAt: prompt.createdAt,
          });
        }
        if (prompt.response) {
          messages.push({
            role: 'assistant',
            content: prompt.response,
            parts: textOnlyParts(prompt.response),
            createdAt: prompt.createdAt,
            model: prompt.model,
          });
        }

        if (prompt.createdAt) {
          if (!firstTime || prompt.createdAt < firstTime) firstTime = prompt.createdAt;
          if (!lastTime || prompt.createdAt > lastTime) lastTime = prompt.createdAt;
        }
      }

      if (messages.length === 0) continue;

      const createdAt = firstTime ?? Date.now();
      const updatedAt = lastTime;

      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      conversations.push({
        externalId: convId,
        title: deriveTitle(messages),
        createdAt,
        updatedAt,
        workspace: workspaceId,
        messages,
        metadata: {
          source: 'cursor-prompts',
        },
      });
    }
  } catch { /* ignore */ }

  return conversations;
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

async function findStateFiles(path: string): Promise<string[]> {
  const files: string[] = [];
  const stats = await stat(path).catch(() => null);
  
  if (!stats) return files;

  // Direct file
  if (stats.isFile() && path.endsWith('.vscdb')) {
    files.push(path);
    return files;
  }

  if (!stats.isDirectory()) return files;

  // Look for workspace directories containing state.vscdb
  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;

    const dbPath = join(path, entry.name, 'state.vscdb');
    const dbStats = await stat(dbPath).catch(() => null);
    if (dbStats?.isFile()) {
      files.push(dbPath);
    }
  }

  return files;
}

runAdapter(adapter);
