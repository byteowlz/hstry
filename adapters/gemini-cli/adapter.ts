/**
 * Gemini CLI adapter for hstry — CLI-only v1
 *
 * Parses Gemini CLI session JSONL files under ~/.gemini/tmp (chats/session-*.jsonl).
 * Does not read IDE state.vscdb (ChatSessionStore).
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
  textPart,
  thinkingPart,
  textOnlyParts,
  isUnderCanonicalRoot,
} from '../types/index.ts';
import { findFirstRealUserMessage, formatFrumTitle, isSystemContext } from '../types/first-message.ts';

const DEFAULT_GEMINI_CLI_PATH = join(homedir(), '.gemini', 'tmp');

interface SessionHeader {
  sessionId?: string;
  kind?: string;
  startTime?: string;
  lastUpdated?: string;
}

interface CliMessage {
  id?: string;
  timestamp?: string;
  type?: string;
  content?: string | Array<{ text?: string }>;
  thoughts?: Array<{ subject?: string; description?: string; timestamp?: string }>;
  model?: string;
}

interface SetEnvelope {
  $set?: {
    messages?: CliMessage[];
    lastUpdated?: string;
  };
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'gemini-cli',
      displayName: 'Gemini CLI',
      version: '1.0.0',
      defaultPaths: [DEFAULT_GEMINI_CLI_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!isUnderCanonicalRoot(path, DEFAULT_GEMINI_CLI_PATH)) {
      return null;
    }
    const files = await findSessionFiles(path, { shallowOnly: true });
    return files.length > 0 ? 0.85 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findSessionFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const conv = await parseSessionFile(filePath, opts);
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
};

async function parseSessionFile(
  filePath: string,
  opts?: ParseOptions,
): Promise<Conversation | null> {
  const raw = await readFile(filePath, 'utf-8').catch(() => null);
  if (!raw) return null;

  let header: SessionHeader | undefined;
  const cliMessages: CliMessage[] = [];
  const seenIds = new Set<string>();
  let skippedLogs = 0;

  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(trimmed) as Record<string, unknown>;
    } catch {
      continue;
    }

    if ('$set' in parsed) {
      const envelope = parsed as SetEnvelope;
      for (const msg of envelope.$set?.messages ?? []) {
        ingestCliMessage(msg, cliMessages, seenIds);
      }
      continue;
    }

    if (parsed.kind === 'main' && parsed.sessionId) {
      header = parsed as SessionHeader;
      continue;
    }

    const msgType = parsed.type as string | undefined;
    if (msgType === 'warning' || msgType === 'info' || msgType === 'error') {
      skippedLogs++;
      continue;
    }

    if (msgType === 'user' || msgType === 'gemini') {
      ingestCliMessage(parsed as CliMessage, cliMessages, seenIds);
    }
  }

  if (cliMessages.length === 0) return null;

  const externalId = header?.sessionId ?? basename(filePath, '.jsonl');
  const createdAt = parseIso(header?.startTime)
    ?? parseIso(cliMessages[0]?.timestamp)
    ?? Date.now();
  let updatedAt = parseIso(header?.lastUpdated) ?? createdAt;
  let model: string | undefined;

  const messages: Message[] = [];

  for (const cli of cliMessages) {
    const ts = parseIso(cli.timestamp) ?? createdAt;
    if (ts > updatedAt) updatedAt = ts;

    if (cli.type === 'user') {
      const text = extractContent(cli.content);
      if (!text.trim()) continue;
      if (isSessionBootstrap(text)) continue;
      messages.push({
        role: 'user',
        content: text,
        parts: textOnlyParts(text),
        createdAt: ts,
      });
      continue;
    }

    if (cli.type === 'gemini') {
      const text = extractContent(cli.content);
      const parts: CanonPart[] = [];
      const thinking = formatThoughts(cli.thoughts);
      if (thinking) parts.push(thinkingPart(thinking));
      if (text.trim()) parts.push(textPart(text));
      if (parts.length === 0) continue;

      messages.push({
        role: 'assistant',
        content: text,
        parts,
        createdAt: ts,
        model: cli.model,
      });
      if (cli.model) model = cli.model;
    }
  }

  if (messages.length === 0) return null;

  if (opts?.since && createdAt < opts.since && updatedAt < opts.since) {
    return null;
  }

  const frum = findFirstRealUserMessage(
    messages.map(m => ({ role: m.role, content: m.content })),
  );
  const title = frum ? formatFrumTitle(frum) : undefined;

  return {
    externalId,
    title,
    createdAt,
    updatedAt,
    model,
    provider: 'google',
    messages,
    metadata: {
      file: filePath,
      cliOnly: true,
      skippedLogs,
    },
  };
}

function ingestCliMessage(
  msg: CliMessage,
  out: CliMessage[],
  seenIds: Set<string>,
): void {
  const id = msg.id;
  if (id) {
    if (seenIds.has(id)) return;
    seenIds.add(id);
  }
  if (msg.type === 'user' || msg.type === 'gemini') {
    out.push(msg);
  }
}

function extractContent(content?: string | Array<{ text?: string }>): string {
  if (!content) return '';
  if (typeof content === 'string') return content;
  return content.map(block => block.text ?? '').join('\n');
}

function formatThoughts(
  thoughts?: Array<{ subject?: string; description?: string }>,
): string {
  if (!thoughts?.length) return '';
  return thoughts
    .map(t => {
      const subject = t.subject?.trim();
      const description = t.description?.trim();
      if (subject && description) return `**${subject}**\n${description}`;
      return subject ?? description ?? '';
    })
    .filter(Boolean)
    .join('\n\n');
}

function isSessionBootstrap(text: string): boolean {
  if (text.includes('<session_context>')) return true;
  return isSystemContext(text);
}

function parseIso(value?: string): number | undefined {
  if (!value) return undefined;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? undefined : parsed;
}

function isSessionFile(name: string): boolean {
  return name.startsWith('session-') && name.endsWith('.jsonl');
}

async function findSessionFiles(
  path: string,
  opts: { shallowOnly: boolean },
): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    return isSessionFile(basename(path)) ? [path] : [];
  }
  if (!stats.isDirectory()) return [];

  const files: string[] = [];
  await walkDir(path, files, opts.shallowOnly ? 4 : 12);
  files.sort();
  return files;
}

async function walkDir(dir: string, files: string[], maxDepth: number): Promise<void> {
  if (maxDepth <= 0) return;

  const entries = await readdir(dir, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      await walkDir(entryPath, files, maxDepth - 1);
      continue;
    }
    if (!entry.isFile()) continue;
    if (!isSessionFile(entry.name)) continue;
    files.push(entryPath);
  }
}

runAdapter(adapter);
