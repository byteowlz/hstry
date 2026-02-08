/**
 * Claude Code adapter for hstry
 *
 * Parses Claude Code session logs stored as JSONL files in ~/.claude/projects
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
  textPart,
  thinkingPart,
  toolCallPart,
  toolResultPart,
  textOnlyParts,
} from '../types/index.ts';

const DEFAULT_CLAUDE_PATH = join(homedir(), '.claude', 'projects');

interface JsonlEntry {
  type?: string;
  summary?: string;
  leafUuid?: string;
  uuid?: string;
  timestamp?: string | number;
  message?: ClaudeMessage;
  sessionId?: string;
  project_path?: string;
  cwd?: string;
  version?: string;
}

interface ClaudeMessage {
  role?: string;
  content?: string | ClaudeContentBlock[];
  model?: string;
}

interface ClaudeContentBlock {
  type?: string;
  text?: string;
  thinking?: string;
  name?: string;
  input?: unknown;
  output?: unknown;
  content?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'claude-code',
      displayName: 'Claude Code',
      version: '1.0.0',
      defaultPaths: [DEFAULT_CLAUDE_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findJsonlFiles(path, { shallowOnly: true });
    return files.length > 0 ? 0.9 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findJsonlFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const raw = await readFile(filePath, 'utf-8');
      const lines = raw.split(/\r?\n/).filter(line => line.trim().length > 0);
      if (lines.length === 0) continue;

      const entries = lines
        .map(line => {
          try {
            return JSON.parse(line) as JsonlEntry;
          } catch {
            return null;
          }
        })
        .filter((entry): entry is JsonlEntry => entry !== null);

      if (entries.length === 0) continue;

      const messages = extractMessages(entries);
      if (messages.length === 0) continue;

      const timestamps = messages
        .map(msg => msg.createdAt)
        .filter((ts): ts is number => typeof ts === 'number');

      const createdAt = timestamps.length > 0 ? Math.min(...timestamps) : Date.now();
      const updatedAt = timestamps.length > 0 ? Math.max(...timestamps) : undefined;

      // Check both created and updated time so modified sessions are re-imported
      if (opts?.since) {
        const lastModified = updatedAt ?? createdAt;
        if (createdAt < opts.since && lastModified < opts.since) {
          continue;
        }
      }

      const summary = entries.find(e => e.type === 'summary' && e.summary)?.summary;
      const sessionId =
        entries.find(e => e.sessionId)?.sessionId ??
        entries.find(e => e.message && e.uuid)?.uuid ??
        basename(filePath, extname(filePath));

      const workspace =
        entries.find(e => e.project_path)?.project_path ||
        entries.find(e => e.cwd)?.cwd ||
        deriveWorkspace(filePath);

      conversations.push({
        externalId: sessionId,
        title: summary || undefined,
        createdAt,
        updatedAt,
        workspace,
        messages,
        metadata: {
          file: filePath,
          version: entries.find(e => e.version)?.version,
          leafUuid: entries.find(e => e.leafUuid)?.leafUuid,
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

    if (opts.format !== 'claude-code') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = conversations.map(conv => {
      const jsonl = buildClaudeCodeJsonl(conv);
      const name = `${conv.externalId ?? 'session'}.jsonl`;
      return { path: name, content: jsonl };
    });

    return {
      format: 'claude-code',
      files,
      mimeType: 'application/jsonl',
    };
  },
};

function extractMessages(entries: JsonlEntry[]): Message[] {
  const messages: Message[] = [];

  for (const entry of entries) {
    if (entry.type === 'summary') continue;

    const msg = entry.message;
    if (!msg || !msg.role) continue;

    const content = extractContent(msg.content);
    if (!content) continue;

    const createdAt = parseTimestamp(entry.timestamp);
    const parts = buildClaudeCodeParts(msg.content) ?? textOnlyParts(content);

    messages.push({
      role: mapRole(msg.role),
      content,
      parts,
      createdAt,
      model: msg.model,
      metadata: {
        uuid: entry.uuid,
        type: entry.type,
      },
    });
  }

  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

/** Build CanonPart[] from Claude Code content blocks. */
function buildClaudeCodeParts(content?: string | ClaudeContentBlock[]): CanonPart[] | undefined {
  if (!content || typeof content === 'string') return undefined;
  const parts: CanonPart[] = [];
  for (const block of content) {
    if (!block) continue;
    if (block.type === 'text' && block.text) {
      parts.push(textPart(block.text));
    } else if (block.type === 'thinking' && block.thinking) {
      parts.push(thinkingPart(block.thinking));
    } else if (block.type === 'tool_use' && block.name) {
      const callId = (block as Record<string, unknown>).id as string ?? block.name;
      parts.push(toolCallPart(callId, block.name, block.input));
    } else if (block.type === 'tool_result') {
      const callId = (block as Record<string, unknown>).tool_use_id as string ?? 'unknown';
      const output = typeof block.content === 'string' ? block.content : JSON.stringify(block.output);
      parts.push(toolResultPart(callId, output));
    }
  }
  return parts.length > 0 ? parts : undefined;
}

function extractContent(content?: string | ClaudeContentBlock[]): string {
  if (!content) return '';
  if (typeof content === 'string') return content.trim();

  const parts: string[] = [];
  for (const block of content) {
    if (!block) continue;
    if (block.type === 'text' && block.text) {
      parts.push(block.text.trim());
    } else if (block.type === 'thinking' && block.thinking) {
      parts.push(block.thinking.trim());
    } else if (block.type === 'tool_result' && typeof block.content === 'string') {
      parts.push(block.content.trim());
    }
  }

  return parts.filter(Boolean).join('\n').trim();
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

function buildClaudeCodeJsonl(conv: Conversation): string {
  const lines: string[] = [];
  const sessionId = conv.externalId ?? `session-${Date.now()}`;
  const summary = conv.title ?? undefined;

  if (summary) {
    lines.push(
      JSON.stringify({
        type: 'summary',
        summary,
        sessionId,
        timestamp: new Date(conv.createdAt).toISOString(),
      })
    );
  }

  for (const msg of conv.messages) {
    const timestamp = new Date(msg.createdAt ?? conv.createdAt).toISOString();
    lines.push(
      JSON.stringify({
        type: 'message',
        sessionId,
        timestamp,
        message: {
          role: msg.role,
          content: [{ type: 'text', text: msg.content }],
          model: msg.model,
        },
      })
    );
  }

  return lines.join('\n') + '\n';
}

function parseTimestamp(value?: string | number): number | undefined {
  if (!value) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? value * 1000 : value;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
}

function deriveWorkspace(filePath: string): string | undefined {
  const parts = filePath.split(/[\\/]/);
  const projectsIndex = parts.lastIndexOf('projects');
  if (projectsIndex >= 0 && parts.length > projectsIndex + 1) {
    return parts[projectsIndex + 1];
  }
  return undefined;
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

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(path, entry.name);

    if (entry.isFile() && extname(entry.name) === '.jsonl') {
      files.push(entryPath);
      continue;
    }

    if (entry.isDirectory()) {
      const nested = await readdir(entryPath, { withFileTypes: true }).catch(() => []);
      for (const child of nested) {
        if (child.isFile() && extname(child.name) === '.jsonl') {
          files.push(join(entryPath, child.name));
        }
      }

      if (!opts.shallowOnly) {
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
