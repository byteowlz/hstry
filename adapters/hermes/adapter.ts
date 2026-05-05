/**
 * Hermes Agent adapter for hstry.
 *
 * Parses Hermes session files stored under ~/.hermes/sessions. The canonical
 * source is `session_<id>.json` (full transcript + metadata); the sibling
 * `<id>.jsonl` files only hold partial recent state and are ignored.
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
import { findFirstRealUserMessage, formatFrumTitle } from '../types/first-message.ts';

const DEFAULT_HERMES_PATH = join(homedir(), '.hermes', 'sessions');

interface HermesSessionFile {
  session_id?: string;
  model?: string;
  base_url?: string;
  platform?: string | null;
  session_start?: string;
  last_updated?: string;
  system_prompt?: string;
  message_count?: number;
  messages?: HermesMessage[];
}

interface HermesMessage {
  role?: string;
  content?: string;
  reasoning?: string;
  finish_reason?: string;
  timestamp?: string | number;
  tool_call_id?: string;
  tool_name?: string;
  tool_calls?: HermesToolCall[];
}

interface HermesToolCall {
  id?: string;
  call_id?: string;
  type?: string;
  function?: { name?: string; arguments?: string };
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'hermes',
      displayName: 'Hermes Agent',
      version: '1.0.0',
      defaultPaths: [DEFAULT_HERMES_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findSessionFiles(path);
    return files.length > 0 ? 0.9 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findSessionFiles(path);
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

  let session: HermesSessionFile;
  try {
    session = JSON.parse(raw) as HermesSessionFile;
  } catch {
    return null;
  }

  const sessionId = session.session_id ?? basename(filePath, '.json').replace(/^session_/, '');
  const createdAt = parseTimestamp(session.session_start) ?? Date.now();
  const updatedAt = parseTimestamp(session.last_updated);

  if (opts?.since) {
    const lastModified = updatedAt ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const messages: Message[] = [];
  const pendingToolCalls = new Map<string, ToolCall>();

  for (const raw of session.messages ?? []) {
    const role = mapRole(raw.role);
    const content = (raw.content ?? '').toString();
    const timestamp = parseTimestamp(raw.timestamp) ?? createdAt;
    const parts: CanonPart[] = [];

    if (raw.reasoning && raw.reasoning.trim()) {
      parts.push(thinkingPart(raw.reasoning));
    }
    if (content.trim()) {
      parts.push(textPart(content));
    }

    if (Array.isArray(raw.tool_calls) && raw.tool_calls.length > 0) {
      for (const tc of raw.tool_calls) {
        const callId = tc.id ?? tc.call_id ?? cryptoRandom('tc');
        const name = tc.function?.name ?? tc.type ?? 'tool';
        const input = safeParseJson(tc.function?.arguments);
        parts.push(toolCallPart(callId, name, input));
        pendingToolCalls.set(callId, {
          toolName: name,
          input,
          status: 'pending',
        });
      }
    }

    if (role === 'tool') {
      const callId = raw.tool_call_id ?? '';
      const name = raw.tool_name ?? pendingToolCalls.get(callId)?.toolName ?? 'tool';
      parts.push(toolResultPart(callId, content, { name }));
      pendingToolCalls.delete(callId);
    }

    if (parts.length === 0) {
      const fallback = textOnlyParts(content);
      messages.push({
        role,
        content,
        parts: fallback,
        createdAt: timestamp,
        model: session.model,
      });
      continue;
    }

    messages.push({
      role,
      content,
      parts,
      createdAt: timestamp,
      model: session.model,
    });
  }

  if (messages.length === 0) return null;

  const frum = findFirstRealUserMessage(
    messages.map(m => ({ role: m.role, content: m.content })),
  );
  const title = frum ? formatFrumTitle(frum) : undefined;

  return {
    externalId: sessionId,
    title,
    createdAt,
    updatedAt,
    model: session.model,
    provider: session.base_url ? deriveProvider(session.base_url) : undefined,
    messages,
    metadata: {
      file: filePath,
      platform: session.platform ?? undefined,
      baseUrl: session.base_url,
      messageCount: session.message_count,
    },
  };
}

function deriveProvider(baseUrl: string): string | undefined {
  try {
    const url = new URL(baseUrl);
    return url.host;
  } catch {
    return undefined;
  }
}

function safeParseJson(value?: string): unknown {
  if (!value) return undefined;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function parseTimestamp(value?: string | number): number | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value === 'number') {
    return value < 1e12 ? Math.floor(value * 1000) : Math.floor(value);
  }
  // Hermes writes naive ISO timestamps without timezone (e.g. "2026-04-18T04:53:25.274422").
  // Date.parse treats these as local time on Node; coerce to UTC by appending 'Z' if absent.
  const normalized = /[zZ]|[+-]\d{2}:?\d{2}$/.test(value) ? value : `${value}Z`;
  const parsed = Date.parse(normalized);
  return Number.isNaN(parsed) ? undefined : parsed;
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
    case 'function':
      return 'tool';
    default:
      return 'assistant';
  }
}

function cryptoRandom(prefix: string): string {
  if (typeof globalThis.crypto?.randomUUID === 'function') {
    return `${prefix}-${globalThis.crypto.randomUUID().replace(/-/g, '').slice(0, 12)}`;
  }
  return `${prefix}-${Date.now()}${Math.random().toString(16).slice(2, 8)}`;
}

async function findSessionFiles(path: string): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    return path.endsWith('.json') && basename(path).startsWith('session_') ? [path] : [];
  }
  if (!stats.isDirectory()) return [];

  const entries = await readdir(path, { withFileTypes: true }).catch(() => []);
  const files: string[] = [];
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    if (!entry.name.startsWith('session_')) continue;
    if (!entry.name.endsWith('.json')) continue;
    files.push(join(path, entry.name));
  }
  return files;
}

runAdapter(adapter);
