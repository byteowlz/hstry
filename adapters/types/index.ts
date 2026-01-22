/**
 * @hstry/types - TypeScript types for hstry adapters
 */

/** Message roles across different sources */
export type MessageRole = 'user' | 'assistant' | 'system' | 'tool';

/** Tool call status */
export type ToolStatus = 'pending' | 'success' | 'error';

/** Attachment types */
export type AttachmentType = 'file' | 'image' | 'code';

/** A conversation from any source, normalized to a common format */
export interface Conversation {
  externalId?: string;
  title?: string;
  createdAt: number;      // Unix timestamp (ms)
  updatedAt?: number;
  model?: string;
  workspace?: string;
  tokensIn?: number;
  tokensOut?: number;
  costUsd?: number;
  messages: Message[];
  metadata?: Record<string, unknown>;
}

/** A message within a conversation */
export interface Message {
  role: MessageRole;
  content: string;
  createdAt?: number;     // Unix timestamp (ms)
  model?: string;
  tokens?: number;
  costUsd?: number;
  toolCalls?: ToolCall[];
  attachments?: Attachment[];
  metadata?: Record<string, unknown>;
}

/** A tool call within a message */
export interface ToolCall {
  toolName: string;
  input?: unknown;
  output?: string;
  status?: ToolStatus;
  durationMs?: number;
}

/** An attachment to a message */
export interface Attachment {
  type: AttachmentType;
  name?: string;
  mimeType?: string;
  content?: string;       // Base64 for binary, plain text for code
  path?: string;
  language?: string;      // For code blocks
  metadata?: Record<string, unknown>;
}

/** Options for parsing conversations */
export interface ParseOptions {
  since?: number;         // Only parse after this timestamp (ms)
  limit?: number;         // Max conversations to parse
  includeTools?: boolean;
  includeAttachments?: boolean;
}

/** Export formats supported by adapters */
export type ExportFormat =
  | 'markdown'
  | 'json'
  | 'opencode'
  | 'codex'
  | 'claude-code'
  | 'claude-web'
  | 'chatgpt'
  | 'gemini'
  | 'aider'
  | 'pi';

/** Options for exporting conversations */
export interface ExportOptions {
  format: ExportFormat;
  pretty?: boolean;
  includeTools?: boolean;
  includeAttachments?: boolean;
}

export interface ExportFile {
  path: string;
  content: string;
  encoding?: 'utf8' | 'base64';
}

export interface ExportResult {
  format: ExportFormat;
  content?: string;
  files?: ExportFile[];
  mimeType?: string;
  metadata?: Record<string, unknown>;
}

/** Adapter metadata */
export interface AdapterInfo {
  name: string;
  displayName: string;
  version: string;
  defaultPaths: string[];
}

/** Adapter interface that all adapters must implement */
export interface Adapter {
  /** Get adapter metadata */
  info(): AdapterInfo;

  /** Detect if path contains data for this adapter (0-1 confidence) */
  detect(path: string): Promise<number | null>;

  /** Parse conversations from path */
  parse(path: string, opts?: ParseOptions): Promise<Conversation[]>;

  /** Optional: supports incremental sync */
  supportsIncremental?: boolean;
  
  /** Optional: parse only new conversations since timestamp */
  parseSince?(path: string, since: number): Promise<Conversation[]>;

  /** Export conversations to a specific format */
  export?(conversations: Conversation[], opts: ExportOptions): Promise<ExportResult>;
}

/** Request from hstry runtime */
export type AdapterRequest =
  | { method: 'info' }
  | { method: 'detect'; params: { path: string } }
  | { method: 'parse'; params: { path: string; opts?: ParseOptions } }
  | { method: 'export'; params: { conversations: Conversation[]; opts: ExportOptions } };

/** Response to hstry runtime */
export type AdapterResponse =
  | AdapterInfo
  | number | null
  | Conversation[]
  | ExportResult
  | { error: string };

/** Read all data from stdin */
async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/** 
 * Main entry point for adapters.
 * Handles the request/response protocol with the Rust runtime.
 */
export function runAdapter(adapter: Adapter): void {
  const useStdin = process.env.HSTRY_REQUEST_STDIN === '1' || 
                   Bun?.env?.HSTRY_REQUEST_STDIN === '1' ||
                   (typeof Deno !== 'undefined' && Deno?.env?.get?.('HSTRY_REQUEST_STDIN') === '1');

  (async () => {
    try {
      let requestJson: string;

      if (useStdin) {
        requestJson = await readStdin();
      } else {
        requestJson = process.env.HSTRY_REQUEST || 
                      Bun?.env?.HSTRY_REQUEST || 
                      (typeof Deno !== 'undefined' ? Deno?.env?.get?.('HSTRY_REQUEST') : undefined) || 
                      '';
      }

      if (!requestJson) {
        console.error(JSON.stringify({ error: 'HSTRY_REQUEST not provided' }));
        process.exit(1);
      }

      const request: AdapterRequest = JSON.parse(requestJson);
      let response: AdapterResponse;

      switch (request.method) {
        case 'info':
          response = adapter.info();
          break;
        case 'detect':
          response = await adapter.detect(request.params.path);
          break;
        case 'parse':
          response = await adapter.parse(request.params.path, request.params.opts);
          break;
        case 'export':
          if (!adapter.export) {
            response = { error: 'Adapter does not support export' };
            break;
          }
          response = await adapter.export(request.params.conversations, request.params.opts);
          break;
        default:
          response = { error: `Unknown method: ${(request as any).method}` };
      }

      console.log(JSON.stringify(response));
    } catch (err) {
      console.log(JSON.stringify({ error: String(err) }));
      process.exit(1);
    }
  })();
}

// Type declarations for different runtimes
declare const Bun: { env?: Record<string, string> } | undefined;
declare const Deno: { env?: { get?(key: string): string | undefined } } | undefined;
