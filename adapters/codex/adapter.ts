/**
 * Codex CLI adapter for hstry
 *
 * Parses Codex rollout JSONL files under ~/.codex/sessions
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
  ToolCall,
} from '../types/index.ts';
import { runAdapter } from '../types/index.ts';

const DEFAULT_CODEX_HOME = join(homedir(), '.codex');
const DEFAULT_PATHS = [
  join(DEFAULT_CODEX_HOME, 'sessions'),
  join(DEFAULT_CODEX_HOME, 'archived_sessions'),
];

interface CodexEvent {
  timestamp?: string;
  type?: string;
  payload?: unknown;
}

interface CodexSessionMeta {
  id?: string;
  forked_from_id?: string | null;
  timestamp?: string;
  cwd?: string;
  originator?: string;
  cli_version?: string;
  source?: string;
  model_provider?: string | null;
}

interface CodexSessionMetaLine {
  meta?: CodexSessionMeta;
  git?: {
    commit_hash?: string;
    branch?: string;
    repository_url?: string;
  };
}

interface CodexTurnContext {
  model?: string;
  approval_policy?: string;
  sandbox_policy?: { mode?: string } | null;
  cwd?: string;
}

interface CodexResponseItem {
  type?: string;
  role?: string;
  content?: Array<{ type?: string; text?: string; image_url?: string }>;
  name?: string;
  arguments?: string;
  call_id?: string;
  output?: unknown;
  status?: string;
  action?: unknown;
  input?: string;
}

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'codex',
      displayName: 'Codex CLI',
      version: '1.0.0',
      defaultPaths: DEFAULT_PATHS,
    };
  },

  async detect(path: string): Promise<number | null> {
    const files = await findRolloutFiles(path, { shallowOnly: true });
    return files.length > 0 ? 0.9 : null;
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const files = await findRolloutFiles(path, { shallowOnly: false });
    if (files.length === 0) return [];

    const conversations: Conversation[] = [];

    for (const filePath of files) {
      const conv = await parseRolloutFile(filePath, opts);
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

    if (opts.format !== 'codex') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = conversations.map(conv => {
      const jsonl = buildCodexRollout(conv);
      const name = `${conv.externalId ?? 'session'}-rollout.jsonl`;
      return { path: name, content: jsonl };
    });

    return {
      format: 'codex',
      files,
      mimeType: 'application/jsonl',
    };
  },
};

async function parseRolloutFile(
  filePath: string,
  opts?: ParseOptions
): Promise<Conversation | null> {
  const raw = await readFile(filePath, 'utf-8');
  const lines = raw.split(/\r?\n/).filter(line => line.trim().length > 0);
  if (lines.length === 0) return null;

  let sessionMeta: CodexSessionMeta | undefined;
  let gitInfo: CodexSessionMetaLine['git'] | undefined;
  let model: string | undefined;
  const pendingToolCalls = new Map<string, ToolCall>();
  const messages: Message[] = [];

  let firstUserMessage: string | undefined;
  let lastTimestamp: number | undefined;

  for (const line of lines) {
    let event: CodexEvent | null = null;
    try {
      event = JSON.parse(line) as CodexEvent;
    } catch {
      continue;
    }

    if (!event?.type) continue;

    const timestampMs = event.timestamp ? Date.parse(event.timestamp) : undefined;
    if (timestampMs && (!lastTimestamp || timestampMs > lastTimestamp)) {
      lastTimestamp = timestampMs;
    }

    switch (event.type) {
      case 'session_meta': {
        const payload = event.payload as CodexSessionMetaLine | CodexSessionMeta | undefined;
        if (!payload) break;
        if ('meta' in payload && payload.meta) {
          sessionMeta = payload.meta;
          gitInfo = payload.git;
        } else {
          sessionMeta = payload as CodexSessionMeta;
        }
        break;
      }
      case 'turn_context': {
        const payload = event.payload as CodexTurnContext | undefined;
        if (payload?.model) {
          model = model ?? payload.model;
        }
        break;
      }
      case 'response_item': {
        const payload = event.payload as CodexResponseItem | undefined;
        if (!payload?.type) break;

        if (payload.type === 'message') {
          const content = extractContent(payload.content);
          if (!content) break;

          if (!firstUserMessage && payload.role === 'user') {
            firstUserMessage = content;
          }

          messages.push({
            role: mapRole(payload.role ?? 'assistant'),
            content,
            createdAt: timestampMs,
            model,
          });
          break;
        }

        if (payload.type === 'compacted' && typeof (payload as any).message === 'string') {
          messages.push({
            role: 'assistant',
            content: (payload as any).message,
            createdAt: timestampMs,
            model,
          });
          break;
        }

        if (payload.type === 'function_call' || payload.type === 'custom_tool_call') {
          const callId = payload.call_id;
          if (callId && payload.name) {
            pendingToolCalls.set(callId, {
              toolName: payload.name,
              input: safeParseJson(payload.arguments ?? payload.input),
              status: 'pending',
            });
          }
          break;
        }

        if (payload.type === 'function_call_output' || payload.type === 'custom_tool_call_output') {
          const callId = payload.call_id;
          if (!callId) break;

          const toolCall = pendingToolCalls.get(callId) ?? {
            toolName: payload.type,
          };
          toolCall.output = stringifyOutput(payload.output);
          toolCall.status = 'success';
          toolCall.durationMs = undefined;

          messages.push({
            role: 'tool',
            content: toolCall.output ?? '',
            createdAt: timestampMs,
            toolCalls: [toolCall],
          });

          pendingToolCalls.delete(callId);
          break;
        }

        if (payload.type === 'local_shell_call' || payload.type === 'web_search_call') {
          const toolName = payload.type === 'local_shell_call' ? 'shell' : 'web_search';
          const toolCall: ToolCall = {
            toolName,
            input: payload.action,
            status: payload.status as ToolCall['status'],
          };
          messages.push({
            role: 'tool',
            content: stringifyOutput(payload.action),
            createdAt: timestampMs,
            toolCalls: [toolCall],
          });
          break;
        }

        break;
      }
      default:
        break;
    }
  }

  if (messages.length === 0) return null;

  const createdAt = sessionMeta?.timestamp
    ? Date.parse(sessionMeta.timestamp)
    : messages[0].createdAt ?? Date.now();

  // Check both created and updated time so modified sessions are re-imported
  if (opts?.since) {
    const lastModified = lastTimestamp ?? createdAt;
    if (createdAt < opts.since && lastModified < opts.since) {
      return null;
    }
  }

  const title = sessionMeta?.cwd
    ? buildTitle(sessionMeta.cwd, createdAt, firstUserMessage)
    : firstUserMessage?.slice(0, 80);

  return {
    externalId: sessionMeta?.id ?? basename(filePath),
    title: title || undefined,
    createdAt,
    updatedAt: lastTimestamp,
    model,
    workspace: sessionMeta?.cwd,
    messages,
    metadata: {
      file: filePath,
      forkedFromId: sessionMeta?.forked_from_id ?? undefined,
      originator: sessionMeta?.originator,
      cliVersion: sessionMeta?.cli_version,
      source: sessionMeta?.source,
      modelProvider: sessionMeta?.model_provider,
      git: gitInfo,
    },
  };
}

function extractContent(
  content?: Array<{ type?: string; text?: string; image_url?: string }>
): string {
  if (!content) return '';
  const parts: string[] = [];
  for (const item of content) {
    if (!item) continue;
    if (item.type === 'input_text' || item.type === 'output_text' || item.type === 'text') {
      if (item.text) parts.push(item.text);
    } else if (item.type === 'input_image' && item.image_url) {
      parts.push(`[image] ${item.image_url}`);
    }
  }
  return parts.join('\n').trim();
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

function buildCodexRollout(conv: Conversation): string {
  const lines: string[] = [];
  const sessionId = conv.externalId ?? randomId('session');
  const timestamp = new Date(conv.createdAt).toISOString();

  lines.push(
    JSON.stringify({
      timestamp,
      type: 'session_meta',
      payload: {
        meta: {
          id: sessionId,
          timestamp,
          cwd: conv.workspace ?? '',
          originator: 'hstry',
          cli_version: '0.0.0',
          source: 'cli',
          model_provider: null,
        },
      },
    })
  );

  for (const msg of conv.messages) {
    const msgTimestamp = new Date(msg.createdAt ?? conv.createdAt).toISOString();
    lines.push(
      JSON.stringify({
        timestamp: msgTimestamp,
        type: 'response_item',
        payload: {
          type: 'message',
          role: msg.role,
          content: [
            {
              type: msg.role === 'user' ? 'input_text' : 'output_text',
              text: msg.content,
            },
          ],
        },
      })
    );
  }

  return lines.join('\n') + '\n';
}

function randomId(prefix: string): string {
  const uuid =
    typeof globalThis.crypto?.randomUUID === 'function'
      ? globalThis.crypto.randomUUID()
      : `${Date.now()}${Math.random().toString(16).slice(2)}`;
  return `${prefix}-${uuid.replace(/-/g, '').slice(0, 12)}`;
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

function buildTitle(cwd: string, createdAt: number, firstUserMessage?: string): string {
  const project = cwd.split(/[\\/]/).filter(Boolean).pop() ?? cwd;
  const date = new Date(createdAt);
  const dateStr = date.toISOString().slice(0, 10);

  if (!firstUserMessage) {
    const time = date.toISOString().slice(11, 16);
    return `${project} - ${dateStr} - ${time}`;
  }

  let msg = firstUserMessage.replace(/\s+/g, ' ').trim();
  if (msg.length > 60) {
    msg = `${msg.slice(0, 60)}...`;
  }

  return `${project} - ${dateStr} - ${msg}`;
}

function safeParseJson(value?: string): unknown {
  if (!value) return undefined;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function stringifyOutput(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value === undefined || value === null) return '';
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

async function findRolloutFiles(
  path: string,
  opts: { shallowOnly: boolean }
): Promise<string[]> {
  const stats = await stat(path).catch(() => null);
  if (!stats) return [];

  if (stats.isFile()) {
    return extname(path) === '.jsonl' ? [path] : [];
  }

  const roots: string[] = [];
  if (stats.isDirectory()) {
    roots.push(path);
    const sessionsDir = join(path, 'sessions');
    const archivedDir = join(path, 'archived_sessions');
    if (await existsDir(sessionsDir)) roots.push(sessionsDir);
    if (await existsDir(archivedDir)) roots.push(archivedDir);
  }

  const files: string[] = [];
  for (const root of roots) {
    const found = await walkForJsonl(root, opts.shallowOnly ? 2 : 4);
    files.push(...found.filter(file => file.includes('rollout-') || file.endsWith('.jsonl')));
  }

  return files;
}

async function walkForJsonl(root: string, depth: number): Promise<string[]> {
  const results: string[] = [];
  if (depth < 0) return results;

  const entries = await readdir(root, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    const entryPath = join(root, entry.name);
    if (entry.isFile() && entry.name.endsWith('.jsonl')) {
      results.push(entryPath);
    } else if (entry.isDirectory()) {
      results.push(...(await walkForJsonl(entryPath, depth - 1)));
    }
  }
  return results;
}

async function existsDir(path: string): Promise<boolean> {
  const stats = await stat(path).catch(() => null);
  return !!stats?.isDirectory();
}

runAdapter(adapter);
