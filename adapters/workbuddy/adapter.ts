/**
 * WorkBuddy adapter for hstry
 *
 * Parses WorkBuddy project session JSONL files under ~/.workbuddy/projects.
 * v1 skips subagents directories (sub-agent transcripts are not imported).
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
  isUnderCanonicalRoot,
} from '../types/index.ts';
import { findFirstRealUserMessage, formatFrumTitle } from '../types/first-message.ts';

const DEFAULT_WORKBUDDY_PATH = join(homedir(), '.workbuddy', 'projects');

interface WorkbuddyEvent {
  id?: string;
  timestamp?: number;
  type?: string;
  role?: string;
  content?: Array<{ type?: string; text?: string }>;
  rawContent?: Array<{ type?: string; text?: string }>;
  name?: string;
  callId?: string;
  arguments?: string;
  output?: { type?: string; text?: string };
  status?: string;
  sessionId?: string;
  cwd?: string;
  aiTitle?: string;
  providerData?: { model?: string; agent?: string };
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'workbuddy',
      displayName: 'WorkBuddy',
      version: '1.0.0',
      defaultPaths: [DEFAULT_WORKBUDDY_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    if (!isUnderCanonicalRoot(path, DEFAULT_WORKBUDDY_PATH)) {
      return null;
    }
    const files = await findSessionFiles(path, { shallowOnly: true });
    return files.length > 0 ? 0.9 : null;
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

  const events: WorkbuddyEvent[] = [];
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      events.push(JSON.parse(trimmed) as WorkbuddyEvent);
    } catch {
      // skip malformed lines
    }
  }

  if (events.length === 0) return null;

  const externalId = basename(filePath, '.jsonl');
  let title: string | undefined;
  let workspace: string | undefined;
  let model: string | undefined;
  const messages: Message[] = [];
  let createdAt = Date.now();
  let updatedAt = createdAt;
  let pendingThinking: string[] = [];

  const flushThinking = (): CanonPart[] => {
    if (pendingThinking.length === 0) return [];
    const text = pendingThinking.join('\n\n');
    pendingThinking = [];
    return [thinkingPart(text)];
  };

  for (const event of events) {
    const ts = normalizeTimestamp(event.timestamp) ?? updatedAt;
    if (ts < createdAt) createdAt = ts;
    if (ts > updatedAt) updatedAt = ts;

    if (event.cwd && !workspace) {
      workspace = event.cwd;
    }

    const eventType = event.type ?? '';

    if (eventType === 'ai-title' && event.aiTitle) {
      title = event.aiTitle;
      continue;
    }

    if (eventType === 'file-history-snapshot') {
      continue;
    }

    if (eventType === 'reasoning') {
      const reasoning = extractReasoningText(event);
      if (reasoning) pendingThinking.push(reasoning);
      continue;
    }

    if (eventType === 'function_call') {
      const thinking = flushThinking();
      const callId = event.callId ?? event.id ?? cryptoRandom('call');
      const name = event.name ?? 'tool';
      const input = safeParseJson(event.arguments);
      const parts: CanonPart[] = [...thinking, toolCallPart(callId, name, input)];
      messages.push({
        role: 'assistant',
        content: '',
        parts,
        createdAt: ts,
        model: event.providerData?.model ?? model,
      });
      if (event.providerData?.model) model = event.providerData.model;
      continue;
    }

    if (eventType === 'function_call_result') {
      flushThinking();
      const callId = event.callId ?? '';
      const name = event.name ?? 'tool';
      const outputText = event.output?.text ?? JSON.stringify(event.output ?? '');
      const isError = event.status === 'failed' || event.status === 'error';
      messages.push({
        role: 'tool',
        content: outputText,
        parts: [toolResultPart(callId, outputText, { name, isError })],
        createdAt: ts,
      });
      continue;
    }

    if (eventType !== 'message') {
      continue;
    }

    const role = mapRole(event.role);
    if (role === 'user') {
      flushThinking();
      const text = extractUserText(event);
      if (!text.trim()) continue;
      messages.push({
        role: 'user',
        content: text,
        parts: textOnlyParts(text),
        createdAt: ts,
      });
      continue;
    }

    if (role === 'assistant') {
      const thinking = flushThinking();
      const text = extractAssistantText(event);
      if (!text.trim() && thinking.length === 0) continue;
      const parts: CanonPart[] = [...thinking];
      if (text.trim()) parts.push(textPart(text));
      messages.push({
        role: 'assistant',
        content: text,
        parts: parts.length > 0 ? parts : undefined,
        createdAt: ts,
        model: event.providerData?.model ?? model,
      });
      if (event.providerData?.model) model = event.providerData.model;
    }
  }

  flushThinking();

  if (messages.length === 0) return null;

  if (opts?.since && createdAt < opts.since && updatedAt < opts.since) {
    return null;
  }

  if (!title) {
    const frum = findFirstRealUserMessage(
      messages.map(m => ({ role: m.role, content: m.content })),
    );
    if (frum) title = formatFrumTitle(frum);
  }

  return {
    externalId,
    title,
    createdAt,
    updatedAt,
    model,
    workspace,
    messages,
    metadata: {
      file: filePath,
      skipsSubagents: true,
    },
  };
}

function extractUserText(event: WorkbuddyEvent): string {
  const parts: string[] = [];
  for (const block of event.content ?? []) {
    if (block.type === 'input_text' && block.text) {
      parts.push(block.text);
    }
  }
  const raw = parts.join('\n');
  return extractUserQuery(raw);
}

function extractAssistantText(event: WorkbuddyEvent): string {
  const parts: string[] = [];
  for (const block of event.content ?? []) {
    if (block.type === 'output_text' && block.text) {
      parts.push(block.text);
    }
  }
  return parts.join('\n');
}

function extractReasoningText(event: WorkbuddyEvent): string {
  const parts: string[] = [];
  for (const block of event.rawContent ?? event.content ?? []) {
    if (block.type === 'reasoning_text' && block.text) {
      parts.push(block.text);
    }
  }
  return parts.join('\n\n');
}

/** Pull text from <user_query> when WorkBuddy wraps system context around it. */
function extractUserQuery(content: string): string {
  const match = content.match(/<user_query>([\s\S]*?)<\/user_query>/i);
  if (match?.[1]) return match[1].trim();
  if (content.includes('<system-reminder')) {
    const afterReminder = content.replace(/<system-reminder[\s\S]*?<\/system-reminder>/gi, '').trim();
    if (afterReminder) return afterReminder;
  }
  return content;
}

function mapRole(role?: string): Message['role'] {
  switch ((role ?? '').toLowerCase()) {
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

function normalizeTimestamp(value?: number): number | undefined {
  if (value === undefined || value === null) return undefined;
  return Math.floor(value);
}

function safeParseJson(value?: string): unknown {
  if (!value) return undefined;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function cryptoRandom(prefix: string): string {
  if (typeof globalThis.crypto?.randomUUID === 'function') {
    return `${prefix}-${globalThis.crypto.randomUUID().replace(/-/g, '').slice(0, 12)}`;
  }
  return `${prefix}-${Date.now()}${Math.random().toString(16).slice(2, 8)}`;
}

function isSubagentPath(filePath: string): boolean {
  const normalized = filePath.replace(/\\/g, '/').toLowerCase();
  return normalized.includes('/subagents/');
}

async function findSessionFiles(
  path: string,
  opts: { shallowOnly: boolean },
): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    if (extname(path) === '.jsonl' && !isSubagentPath(path)) return [path];
    return [];
  }
  if (!stats.isDirectory()) return [];

  const files: string[] = [];
  await walkDir(path, files, opts.shallowOnly ? 2 : 8);
  files.sort();
  return files;
}

async function walkDir(dir: string, files: string[], maxDepth: number): Promise<void> {
  if (maxDepth <= 0) return;

  const entries = await readdir(dir, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name.toLowerCase() === 'subagents') continue;
      await walkDir(entryPath, files, maxDepth - 1);
      continue;
    }
    if (!entry.isFile()) continue;
    if (extname(entry.name) !== '.jsonl') continue;
    if (isSubagentPath(entryPath)) continue;
    files.push(entryPath);
  }
}

runAdapter(adapter);
