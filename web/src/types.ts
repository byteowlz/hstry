// The data contract: exactly what `hstry ... --json` prints (and what
// hstry-api's read endpoints return). Only fields the UI reads are typed.

export interface Envelope<T> {
	ok: boolean;
	result: T | null;
	error: string | null;
}

/** `hstry list --json` item (core `ConversationPreview`). */
export interface SessionPreview {
	id: string;
	source_id: string;
	external_id?: string | null;
	readable_id?: string | null;
	title?: string | null;
	created_at: string;
	updated_at?: string | null;
	model?: string | null;
	workspace?: string | null;
	metadata?: Record<string, unknown> | null;
	message_count?: number;
	harness?: string | null;
	first_user_message?: string | null;
}

/** `hstry source list --json` item. */
export interface Source {
	id: string;
	adapter: string;
	path?: string | null;
}

export type Part =
	| { type: "text"; text: string }
	| { type: "thinking"; text: string }
	| { type: "tool_call"; toolCallId?: string; name?: string; input?: unknown }
	| { type: "tool_result"; toolCallId?: string; output?: unknown; isError?: boolean }
	| { type: string; [key: string]: unknown };

export type Role = "user" | "assistant" | "system" | "tool" | "other";

export interface Message {
	id: string;
	idx: number;
	role: Role;
	content: string;
	parts_json?: Part[] | null;
	created_at?: string | null;
	model?: string | null;
}

/** `hstry show --full --json` result (core `ConversationWithMessages`). */
export interface SessionDetail extends SessionPreview {
	messages: Array<Message | { message: Message }>;
}

/** One hit of `hstry search --json` (projected evidence). */
export interface SearchHit {
	conversation_id: string;
	message_idx: number;
	role: Role;
	title?: string | null;
	snippet: string;
	match_position?: number | null;
	timestamp?: string | null;
	provenance?: { source?: string | null; machine?: string | null } | null;
}

export interface SearchReport {
	hits: SearchHit[];
	has_more?: boolean;
	truncated?: boolean;
}

export interface Agents {
	default: string;
	agents: string[];
}

/** Where the UI reads from: hstry-api over HTTP, or an Oqto host. */
export interface DataSource {
	readonly kind: "http" | "oqto";
	sessions(): Promise<SessionPreview[]>;
	session(id: string): Promise<SessionDetail>;
	search(query: string): Promise<SearchReport>;
	sources(): Promise<Source[]>;
	agents(): Promise<Agents>;
}

export function unwrap<T>(value: unknown, what: string): T {
	const env = value as Envelope<T> | null;
	if (env && typeof env === "object" && "ok" in env) {
		if (!env.ok || env.result == null) throw new Error(env.error ?? `${what} failed`);
		return env.result;
	}
	return value as T;
}

/** `show` wraps messages as `{message, tool_calls, attachments}`; flatten. */
export function messagesOf(detail: SessionDetail): Message[] {
	return detail.messages.map((m) => ("message" in m ? m.message : m));
}
