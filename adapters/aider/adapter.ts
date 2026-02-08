/**
 * Aider adapter for hstry
 *
 * Parses Aider markdown chat logs (e.g., .aider.chat.history.md)
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, dirname, join } from 'path';
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
  join(homedir(), 'projects'),
  join(homedir(), 'work'),
  join(homedir(), 'src'),
];

const HISTORY_FILES = ['.aider.chat.history.md', '.aider.chat.history.txt'];

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'aider',
      displayName: 'Aider',
      version: '1.0.0',
      defaultPaths: DEFAULT_SEARCH_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findHistoryFiles(path, { shallowOnly: true });
    return files.length > 0 ? 0.8 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findHistoryFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const content = await readFile(filePath, 'utf-8');
      const stats = await stat(filePath).catch(() => null);

      const messages = parseMarkdownMessages(content);
      if (messages.length === 0) continue;

      const createdAt = stats?.mtimeMs ? Math.floor(stats.mtimeMs) : Date.now();
      const updatedAt = stats?.mtimeMs ? Math.floor(stats.mtimeMs) : undefined;

      // Check both created and updated time so modified sessions are re-imported
      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      conversations.push({
        externalId: basename(filePath),
        title: deriveTitle(content),
        createdAt,
        updatedAt,
        workspace: dirname(filePath),
        messages,
        metadata: {
          file: filePath,
        },
      });

      if (opts?.limit && conversations.length >= opts.limit) {
        break;
      }
    }

    conversations.sort((a, b) => b.createdAt - a.createdAt);
    return conversations;
  },

  async export(conversations, opts) {
    if (opts.format === 'markdown' || opts.format === 'aider') {
      return {
        format: opts.format,
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

    throw new Error(`Unsupported export format: ${opts.format}`);
  },
};

function parseMarkdownMessages(content: string): Message[] {
  const lines = content.split(/\r?\n/);
  const messages: Message[] = [];

  let currentRole: Message['role'] | null = null;
  let buffer: string[] = [];

  const pushMessage = () => {
    if (!currentRole) return;
    const text = buffer.join('\n').trim();
    if (text.length === 0) return;
    messages.push({
      role: currentRole,
      content: text,
      parts: textOnlyParts(text),
    });
  };

  for (const line of lines) {
    const heading = parseRoleHeading(line);
    if (heading) {
      pushMessage();
      currentRole = heading;
      buffer = [];
      continue;
    }

    buffer.push(line);
  }

  pushMessage();

  return messages;
}

function parseRoleHeading(line: string): Message['role'] | null {
  const match = line.match(/^#{2,6}\s+(user|assistant|system|tool)\b/i);
  if (!match) return null;
  return mapRole(match[1]);
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

function deriveTitle(content: string): string | undefined {
  const firstHeading = content.split(/\r?\n/).find(line => line.startsWith('# '));
  if (firstHeading) {
    const title = firstHeading.replace(/^#\s+/, '').trim();
    if (title.length > 0) return title;
  }
  return undefined;
}

function conversationsToMarkdown(conversations: Conversation[]): string {
  const blocks: string[] = [];
  for (const conv of conversations) {
    const title = conv.title ?? 'Aider Session';
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

async function findHistoryFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const files: string[] = [];

  const stats = await stat(path).catch(() => null);
  if (!stats) return files;

  if (stats.isFile()) {
    if (HISTORY_FILES.includes(basename(path))) {
      files.push(path);
    }
    return files;
  }

  if (!stats.isDirectory()) return files;

  for (const filename of HISTORY_FILES) {
    const direct = join(path, filename);
    const directStats = await stat(direct).catch(() => null);
    if (directStats?.isFile()) {
      files.push(direct);
    }
  }

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const entryPath = join(path, entry.name);

    for (const filename of HISTORY_FILES) {
      const candidate = join(entryPath, filename);
      const candidateStats = await stat(candidate).catch(() => null);
      if (candidateStats?.isFile()) {
        files.push(candidate);
      }
    }

    if (!opts.shallowOnly) {
      const nested = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const nestedEntry of nested) {
        if (!nestedEntry.isDirectory()) continue;
        const nestedPath = join(entryPath, nestedEntry.name);
        for (const filename of HISTORY_FILES) {
          const candidate = join(nestedPath, filename);
          const candidateStats = await stat(candidate).catch(() => null);
          if (candidateStats?.isFile()) {
            files.push(candidate);
          }
        }
      }
    }
  }

  return files;
}

runAdapter(adapter);
