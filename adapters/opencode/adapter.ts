/**
 * OpenCode adapter for hstry
 * 
 * Parses OpenCode session history from ~/.local/share/opencode/project/
 * 
 * Directory structure:
 *   project/<project-name>/storage/session/info/<session-id>.json
 *   project/<project-name>/storage/session/message/<session-id>/<msg-id>.json
 *   project/<project-name>/storage/session/part/<session-id>/<msg-id>/<part-id>.json
 */

import { readdir, readFile, stat } from 'fs/promises';
import { join, basename } from 'path';
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

// OpenCode storage structures
interface SessionInfo {
  id: string;
  version?: string;
  title?: string;
  parentID?: string;
  directory?: string;
  projectID?: string;
  time: {
    created: number;
    updated: number;
  };
}

interface MessageInfo {
  id: string;
  sessionID: string;
  role: string;
  time: {
    created: number;
    completed?: number;
  };
  parentID?: string;
  modelID?: string;
  providerID?: string;
  agent?: string;
  summary?: {
    title?: string;
  };
  tokens?: {
    input?: number;
    output?: number;
    reasoning?: number;
  };
  cost?: number;
}

interface PartInfo {
  id: string;
  messageID: string;
  sessionID: string;
  type: string;
  text?: string;
  tool?: string;
  state?: {
    status?: string;
    input?: unknown;
    output?: string;
    title?: string;
  };
}

const DEFAULT_OPENCODE_PATH = join(homedir(), '.local/share/opencode');

const adapter: Adapter = {
  info(): AdapterInfo {
    return {
      name: 'opencode',
      displayName: 'OpenCode',
      version: '1.0.0',
      defaultPaths: [DEFAULT_OPENCODE_PATH],
    };
  },

  async detect(path: string): Promise<number | null> {
    try {
      // Check for project directory structure
      const projectDir = join(path, 'project');
      const stats = await stat(projectDir);
      if (stats.isDirectory()) {
        // Check if there are project directories with sessions
        const projects = await readdir(projectDir);
        for (const project of projects) {
          const sessionInfoDir = join(projectDir, project, 'storage/session/info');
          try {
            const infoStats = await stat(sessionInfoDir);
            if (infoStats.isDirectory()) {
              const sessions = await readdir(sessionInfoDir);
              if (sessions.some(s => s.startsWith('ses_') && s.endsWith('.json'))) {
                return 0.95; // High confidence
              }
            }
          } catch {
            continue;
          }
        }
      }
      return null;
    } catch {
      return null;
    }
  },

  async parse(path: string, opts?: ParseOptions): Promise<Conversation[]> {
    const projectDir = join(path, 'project');
    const conversations: Conversation[] = [];

    try {
      // Iterate over project directories
      const projects = await readdir(projectDir);

      for (const projectName of projects) {
        const projectPath = join(projectDir, projectName);
        const projectStats = await stat(projectPath);
        if (!projectStats.isDirectory()) continue;

        const sessionInfoDir = join(projectPath, 'storage/session/info');
        const sessionMessageDir = join(projectPath, 'storage/session/message');
        const sessionPartDir = join(projectPath, 'storage/session/part');

        // Extract workspace from project name (e.g., "home-wismut-byteowlz-kittenx" -> "/home/wismut/byteowlz/kittenx")
        const workspace = projectNameToPath(projectName);

        try {
          // Read session info files
          const sessionFiles = await readdir(sessionInfoDir);

          for (const sessionFile of sessionFiles) {
            if (!sessionFile.startsWith('ses_') || !sessionFile.endsWith('.json')) {
              continue;
            }

            const sessionPath = join(sessionInfoDir, sessionFile);
            const sessionContent = await readFile(sessionPath, 'utf-8');
            const session: SessionInfo = JSON.parse(sessionContent);

            // Apply filters
            if (opts?.since && session.time.created < opts.since) {
              continue;
            }

            // Load messages for this session
            const messages = await loadMessages(
              session.id,
              sessionMessageDir,
              sessionPartDir,
              opts
            );

            const conv: Conversation = {
              externalId: session.id,
              title: session.title || undefined,
              createdAt: session.time.created,
              updatedAt: session.time.updated,
              workspace: session.directory || workspace,
              messages,
              metadata: {
                version: session.version,
                parentId: session.parentID,
                projectId: projectName,
              },
            };

            // Calculate totals from messages
            let tokensIn = 0;
            let tokensOut = 0;
            let cost = 0;
            let model: string | undefined;

            for (const msg of messages) {
              if (msg.tokens) {
                if (msg.role === 'user') tokensIn += msg.tokens;
                else tokensOut += msg.tokens;
              }
              if (msg.costUsd) cost += msg.costUsd;
              if (msg.model && !model) model = msg.model;
            }

            if (tokensIn > 0) conv.tokensIn = tokensIn;
            if (tokensOut > 0) conv.tokensOut = tokensOut;
            if (cost > 0) conv.costUsd = cost;
            if (model) conv.model = model;

            conversations.push(conv);

            // Check limit
            if (opts?.limit && conversations.length >= opts.limit) {
              break;
            }
          }
        } catch (err) {
          // Skip projects without session info
          if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
            console.error(`Error reading project ${projectName}:`, err);
          }
          continue;
        }

        if (opts?.limit && conversations.length >= opts.limit) {
          break;
        }
      }
    } catch (err) {
      // If project directory doesn't exist, return empty
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
        return [];
      }
      throw err;
    }

    // Sort by created_at descending (newest first)
    conversations.sort((a, b) => b.createdAt - a.createdAt);

    return conversations;
  },

  supportsIncremental: true,

  async parseSince(path: string, since: number): Promise<Conversation[]> {
    return this.parse(path, { since });
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

    if (opts.format !== 'opencode') {
      throw new Error(`Unsupported export format: ${opts.format}`);
    }

    const files = buildOpenCodeFiles(conversations);
    return {
      format: 'opencode',
      files,
      mimeType: 'application/json',
      metadata: {
        root: 'project/',
      },
    };
  },
};

/**
 * Convert OpenCode project name to filesystem path
 * e.g., "home-wismut-byteowlz-kittenx" -> "/home/wismut/byteowlz/kittenx"
 */
function projectNameToPath(projectName: string): string {
  if (projectName === 'global') return '~';
  return '/' + projectName.replace(/-/g, '/');
}

async function loadMessages(
  sessionId: string,
  messageDir: string,
  partDir: string,
  opts?: ParseOptions
): Promise<Message[]> {
  const sessionMessagePath = join(messageDir, sessionId);
  const sessionPartPath = join(partDir, sessionId);
  const messages: Message[] = [];

  try {
    const messageFiles = await readdir(sessionMessagePath);

    for (const messageFile of messageFiles) {
      if (!messageFile.startsWith('msg_') || !messageFile.endsWith('.json')) {
        continue;
      }

      const messagePath = join(sessionMessagePath, messageFile);
      const messageContent = await readFile(messagePath, 'utf-8');
      const msgInfo: MessageInfo = JSON.parse(messageContent);

      // Load parts for this message
      const parts = await loadParts(sessionId, msgInfo.id, sessionPartPath, opts);

      // Combine text parts into content
      const textParts = parts.filter(p => p.type === 'text' && p.text);
      const content = textParts.map(p => p.text).join('\n');

      // Extract tool calls if requested
      let toolCalls: ToolCall[] | undefined;
      if (opts?.includeTools !== false) {
        const toolParts = parts.filter(p => p.type === 'tool' && p.tool);
        if (toolParts.length > 0) {
          toolCalls = toolParts.map(p => ({
            toolName: p.tool!,
            input: p.state?.input,
            output: p.state?.output,
            status: p.state?.status as 'pending' | 'success' | 'error' | undefined,
          }));
        }
      }

      const tokens = (msgInfo.tokens?.input || 0) + (msgInfo.tokens?.output || 0) + (msgInfo.tokens?.reasoning || 0);

      const msg: Message = {
        role: mapRole(msgInfo.role),
        content,
        createdAt: msgInfo.time.created,
        model: msgInfo.modelID || undefined,
        tokens: tokens > 0 ? tokens : undefined,
        costUsd: msgInfo.cost || undefined,
        toolCalls,
        metadata: {
          id: msgInfo.id,
          parentId: msgInfo.parentID,
          provider: msgInfo.providerID,
          agent: msgInfo.agent,
          summaryTitle: msgInfo.summary?.title,
        },
      };

      messages.push(msg);
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  // Sort by created_at ascending (chronological)
  messages.sort((a, b) => (a.createdAt || 0) - (b.createdAt || 0));

  return messages;
}

async function loadParts(
  sessionId: string,
  messageId: string,
  sessionPartPath: string,
  opts?: ParseOptions
): Promise<PartInfo[]> {
  // Parts are in: part/<session-id>/<msg-id>/<part-id>.json
  const messagePartDir = join(sessionPartPath, messageId);
  const parts: PartInfo[] = [];

  try {
    const partFiles = await readdir(messagePartDir);

    for (const partFile of partFiles) {
      if (!partFile.startsWith('prt_') || !partFile.endsWith('.json')) {
        continue;
      }

      const partPath = join(messagePartDir, partFile);
      const partContent = await readFile(partPath, 'utf-8');
      const part: PartInfo = JSON.parse(partContent);
      parts.push(part);
    }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw err;
    }
  }

  // Sort by ID (roughly chronological)
  parts.sort((a, b) => a.id.localeCompare(b.id));

  return parts;
}

function mapRole(role: string): Message['role'] {
  switch (role.toLowerCase()) {
    case 'user':
    case 'human':
      return 'user';
    case 'assistant':
    case 'agent':
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

function buildOpenCodeFiles(conversations: Conversation[]): { path: string; content: string }[] {
  const files: { path: string; content: string }[] = [];

  for (const conv of conversations) {
    const sessionId = conv.externalId ?? randomId('ses_');
    const projectName = workspaceToProjectName(conv.workspace);
    const basePath = `project/${projectName}/storage/session`;
    const infoDir = `${basePath}/info`;
    const messageDir = `${basePath}/message/${sessionId}`;
    const partDir = `${basePath}/part/${sessionId}`;

    const sessionInfo = {
      id: sessionId,
      version: '1',
      title: conv.title ?? null,
      parentID: null,
      directory: conv.workspace ?? null,
      projectID: projectName,
      time: {
        created: conv.createdAt,
        updated: conv.updatedAt ?? conv.createdAt,
      },
    };
    files.push({
      path: `${infoDir}/${sessionId}.json`,
      content: JSON.stringify(sessionInfo, null, 2),
    });

    conv.messages.forEach((msg, idx) => {
      const msgId = `msg_${idx + 1}`;
      const created = msg.createdAt ?? conv.createdAt;
      const messageInfo = {
        id: msgId,
        sessionID: sessionId,
        role: msg.role,
        time: {
          created,
        },
        modelID: msg.model ?? null,
        providerID: null,
        agent: null,
        tokens: msg.tokens ? { input: msg.tokens, output: 0 } : undefined,
        cost: msg.costUsd ?? undefined,
      };
      files.push({
        path: `${messageDir}/${msgId}.json`,
        content: JSON.stringify(messageInfo, null, 2),
      });

      const partEntries: PartInfo[] = [];
      if (msg.content) {
        partEntries.push({
          id: `prt_${idx + 1}_text`,
          messageID: msgId,
          sessionID: sessionId,
          type: 'text',
          text: msg.content,
        });
      }

      if (msg.toolCalls) {
        msg.toolCalls.forEach((tool, toolIdx) => {
          partEntries.push({
            id: `prt_${idx + 1}_tool_${toolIdx + 1}`,
            messageID: msgId,
            sessionID: sessionId,
            type: 'tool',
            tool: tool.toolName,
            state: {
              status: tool.status,
              input: tool.input,
              output: tool.output,
            },
          });
        });
      }

      partEntries.forEach(part => {
        files.push({
          path: `${partDir}/${msgId}/${part.id}.json`,
          content: JSON.stringify(part, null, 2),
        });
      });
    });
  }

  return files;
}

function workspaceToProjectName(workspace?: string): string {
  if (!workspace) return 'global';
  return workspace.replace(/^\/+/, '').replace(/[\\/]/g, '-');
}

function randomId(prefix: string): string {
  const uuid =
    typeof globalThis.crypto?.randomUUID === 'function'
      ? globalThis.crypto.randomUUID()
      : `${Date.now()}${Math.random().toString(16).slice(2)}`;
  return `${prefix}${uuid.replace(/-/g, '').slice(0, 12)}`;
}

// Run the adapter
runAdapter(adapter);
