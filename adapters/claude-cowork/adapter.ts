/**
 * Claude Cowork adapter for hstry
 *
 * Parses Claude desktop app agent sessions ("Cowork" / local agent mode)
 * stored under the app data directory:
 *   macOS:   ~/Library/Application Support/Claude/local-agent-mode-sessions
 *   Linux:   ~/.config/Claude/local-agent-mode-sessions
 *   Windows: %APPDATA%/Claude/local-agent-mode-sessions
 *
 * Layout (verified against desktop app, June 2026):
 *   <accountId>/<orgId>/local_<uuid>.json          session metadata
 *   <accountId>/<orgId>/local_<uuid>/              session working dir
 *     audit.jsonl                                  audit copy of messages
 *     .claude/projects/<encoded>/<cliSessionId>.jsonl  Claude Code transcript
 *
 * The .claude transcript is authoritative; audit.jsonl is only used as a
 * fallback when the transcript is missing. Cowork session directories are
 * ephemeral (the app deletes them on cleanup), so frequent syncing matters
 * more here than for other sources.
 */

import { readdir, readFile, stat } from 'fs/promises';
import { basename, dirname, extname, join } from 'path';
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
  isUnderAnyCanonicalRoot,
} from '../types/index.ts';
import { findFirstRealUserMessage, formatFrumTitle } from '../types/first-message.ts';

function defaultRoots(): string[] {
  const home = homedir();
  const roots = [
    join(home, 'Library', 'Application Support', 'Claude', 'local-agent-mode-sessions'),
    join(process.env.XDG_CONFIG_HOME ?? join(home, '.config'), 'Claude', 'local-agent-mode-sessions'),
  ];
  if (process.env.APPDATA) {
    roots.push(join(process.env.APPDATA, 'Claude', 'local-agent-mode-sessions'));
  }
  return roots;
}

const DEFAULT_PATHS = defaultRoots();

interface CoworkMeta {
  sessionId?: string;
  cliSessionId?: string;
  processName?: string;
  title?: string;
  isArchived?: boolean;
  initialMessage?: string;
  createdAt?: string | number;
  lastActivityAt?: string | number;
  cwd?: string;
  userSelectedFolders?: string[];
  model?: string;
  permissionMode?: string;
}

interface RawBlock {
  type?: string;
  text?: string;
  thinking?: string;
  name?: string;
  input?: unknown;
  output?: unknown;
  content?: string;
  id?: string;
  tool_use_id?: string;
}

interface RawMessage {
  role?: string;
  type?: string;
  content?: string | RawBlock[];
  text?: string;
  model?: string;
  timestamp?: string | number;
  _audit_timestamp?: string;
  message?: RawMessage;
  uuid?: string;
  isCompactSummary?: boolean;
  isVisibleInTranscriptOnly?: boolean;
  isMeta?: boolean;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'claude-cowork',
      displayName: 'Claude Cowork',
      version: '0.2.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    // Defense in depth: cowork only owns the Claude desktop app's
    // local-agent-mode-sessions trees.
    if (!isUnderAnyCanonicalRoot(path, DEFAULT_PATHS)) {
      return null;
    }
    const files = await findMetaFiles(path, 3);
    return files.length > 0 ? 0.8 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const metaFiles = await findMetaFiles(path, 5);
    if (metaFiles.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const metaPath of metaFiles) {
      const conv = await parseSession(metaPath, opts);
      if (conv) {
        conversations.push(conv);
      }
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
    throw new Error(`Unsupported export format: ${opts.format}`);
  },
};

async function parseSession(
  metaPath: string,
  opts?: ParseOptions
): Promise<Conversation | null> {
  let meta: CoworkMeta;
  try {
    meta = JSON.parse(await readFile(metaPath, 'utf-8')) as CoworkMeta;
  } catch {
    return null;
  }
  if (!meta || typeof meta !== 'object') return null;

  // The session working dir shares the metadata file's stem:
  // local_<uuid>.json -> local_<uuid>/
  const sessionDir = join(dirname(metaPath), basename(metaPath, '.json'));

  // Authoritative transcript: Claude Code JSONL under the session's embedded
  // .claude home. Fall back to audit.jsonl, then to the initial message stub.
  let messages = await extractTranscriptMessages(sessionDir, meta.cliSessionId);
  if (messages.length === 0) {
    messages = await parseJsonlFile(join(sessionDir, 'audit.jsonl'));
  }
  if (messages.length === 0 && meta.initialMessage) {
    messages = [
      {
        role: 'user',
        content: meta.initialMessage,
        parts: textOnlyParts(meta.initialMessage),
        createdAt: parseTimestamp(meta.createdAt),
      },
    ];
  }
  if (messages.length === 0) return null;

  const timestamps = messages
    .map(msg => msg.createdAt)
    .filter((ts): ts is number => typeof ts === 'number');
  const createdAt =
    parseTimestamp(meta.createdAt) ??
    (timestamps.length > 0 ? Math.min(...timestamps) : Date.now());
  const updatedAt =
    parseTimestamp(meta.lastActivityAt) ??
    (timestamps.length > 0 ? Math.max(...timestamps) : undefined);

  // Check both created and updated time so modified sessions are re-imported
  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const titleFallback = (() => {
    const frum = findFirstRealUserMessage(messages);
    return frum ? formatFrumTitle(frum) : undefined;
  })();

  return {
    externalId: meta.sessionId ?? basename(metaPath, '.json'),
    title: meta.title || titleFallback,
    createdAt,
    updatedAt,
    model: meta.model,
    workspace: meta.userSelectedFolders?.[0] ?? meta.cwd,
    messages,
    metadata: {
      file: metaPath,
      cliSessionId: meta.cliSessionId,
      processName: meta.processName,
      permissionMode: meta.permissionMode,
      ...(meta.userSelectedFolders?.length
        ? { userSelectedFolders: meta.userSelectedFolders }
        : {}),
      ...(meta.isArchived ? { archived: true } : {}),
    },
  };
}

async function extractTranscriptMessages(
  sessionDir: string,
  cliSessionId?: string
): Promise<Message[]> {
  const projectsDir = join(sessionDir, '.claude', 'projects');
  const files = await walkForJsonl(projectsDir, 2);
  if (files.length === 0) return [];

  // The metadata names the CLI session backing this Cowork session; other
  // JSONL files in the projects tree are sub-sessions or leftovers.
  const transcript = cliSessionId
    ? files.find(file => basename(file, '.jsonl') === cliSessionId)
    : undefined;
  const selected = transcript ? [transcript] : files;

  const messages: Message[] = [];
  for (const filePath of selected) {
    messages.push(...(await parseJsonlFile(filePath)));
  }
  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

async function parseJsonlFile(filePath: string): Promise<Message[]> {
  let raw: string;
  try {
    raw = await readFile(filePath, 'utf-8');
  } catch {
    return [];
  }

  const messages: Message[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    let entry: RawMessage | null = null;
    try {
      entry = JSON.parse(line) as RawMessage;
    } catch {
      continue;
    }
    const msg = toMessage(entry);
    if (msg) messages.push(msg);
  }

  messages.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return messages;
}

/** Map a raw transcript item (flat or Claude Code-shaped envelope) to a Message. */
function toMessage(raw: RawMessage | null): Message | null {
  if (!raw || typeof raw !== 'object') return null;

  // Synthetic Claude Code entries: compaction summaries duplicate the prior
  // transcript; transcript-only/meta entries are UI artifacts.
  if (raw.isCompactSummary || raw.isVisibleInTranscriptOnly || raw.isMeta) return null;

  // Claude Code-style envelope: { type, timestamp, message: { role, content } }
  const inner = raw.message && typeof raw.message === 'object' ? raw.message : raw;
  const role = inner.role ?? (raw.type === 'user' || raw.type === 'assistant' ? raw.type : undefined);
  if (!role) return null;

  const rawContent = inner.content ?? inner.text;
  const parts = buildParts(rawContent);
  let content = extractContent(rawContent);
  if (!content && parts) {
    content = parts
      .filter(part => part.type === 'tool_call')
      .map(part => (part as { name?: string }).name ?? '')
      .filter(Boolean)
      .join('\n');
  }
  if (!content && (!parts || parts.length === 0)) return null;

  return {
    role: mapRole(role),
    content,
    parts: parts ?? textOnlyParts(content),
    createdAt: parseTimestamp(raw.timestamp ?? raw._audit_timestamp ?? inner.timestamp),
    model: inner.model,
  };
}

function buildParts(content?: string | RawBlock[]): CanonPart[] | undefined {
  if (!content || typeof content === 'string') return undefined;
  const parts: CanonPart[] = [];
  for (const block of content) {
    if (!block) continue;
    if (block.type === 'text' && block.text) {
      parts.push(textPart(block.text));
    } else if (block.type === 'thinking' && block.thinking) {
      parts.push(thinkingPart(block.thinking));
    } else if (block.type === 'tool_use' && block.name) {
      parts.push(toolCallPart(block.id ?? block.name, block.name, block.input));
    } else if (block.type === 'tool_result') {
      const output = typeof block.content === 'string' ? block.content : JSON.stringify(block.output);
      parts.push(toolResultPart(block.tool_use_id ?? 'unknown', output));
    }
  }
  return parts.length > 0 ? parts : undefined;
}

function extractContent(content?: string | RawBlock[]): string {
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

function parseTimestamp(value?: string | number): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    // Heuristic: values before ~2001-09 in ms are second-resolution epochs.
    return Math.floor(value < 1e12 ? value * 1000 : value);
  }
  if (typeof value === 'string') {
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? undefined : parsed;
  }
  return undefined;
}

function conversationsToMarkdown(conversations: Conversation[]): string {
  const blocks: string[] = [];
  for (const conv of conversations) {
    blocks.push(`# ${conv.title ?? 'Conversation'}`);
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

async function findMetaFiles(path: string, depth: number): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    return isMetaFile(path) ? [path] : [];
  }

  const results: string[] = [];
  await walk(path, depth, entryPath => {
    if (isMetaFile(entryPath)) results.push(entryPath);
  });
  return results;
}

function isMetaFile(path: string): boolean {
  const name = basename(path);
  return name.startsWith('local_') && extname(name) === '.json';
}

async function walkForJsonl(root: string, depth: number): Promise<string[]> {
  const results: string[] = [];
  await walk(root, depth, entryPath => {
    if (extname(entryPath) === '.jsonl') results.push(entryPath);
  });
  return results;
}

async function walk(
  root: string,
  depth: number,
  onFile: (path: string) => void
): Promise<void> {
  if (depth < 0) return;
  const entries = await readdir(root, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(root, entry.name);
    if (entry.isFile()) {
      onFile(entryPath);
    } else if (entry.isDirectory()) {
      await walk(entryPath, depth - 1, onFile);
    }
  }
}

runAdapter(adapter);
