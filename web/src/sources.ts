// Two ways to reach hstry, one contract.

import type { OqtoHost } from "@byteowlz/oqto-app-sdk";
import {
	type Agents,
	type DataSource,
	type SearchReport,
	type SessionDetail,
	type SessionPreview,
	type Source,
	unwrap,
} from "./types";

const SEARCH_LIMIT = 300;
const SNIPPET_CHARS = 240;
const SEARCH_BUDGET = 200_000;
/** Messages per transcript page inside Oqto (keeps each page under 1 MiB). */
const SHOW_PAGE = 150;

/**
 * Oqto caps operation output at 10 000 JSON nodes, so the bridge sends lists
 * as "json-rows" (one JSON-encoded string per row). Decode back to the CLI's
 * objects; plain objects pass through.
 */
export function decodeRows<T>(rows: unknown): T[] {
	if (!Array.isArray(rows)) return [];
	return rows.map((row) => (typeof row === "string" ? (JSON.parse(row) as T) : (row as T)));
}

/** Fallback when the host cannot report configured resume agents. */
export const DEFAULT_AGENTS: Agents = {
	default: "pi",
	agents: ["claude-code", "codex", "opencode", "pi"],
};

/** Standalone: the page is served by hstry-api and talks back to it. */
export class HttpSource implements DataSource {
	readonly kind = "http" as const;
	constructor(private readonly base = "") {}

	private async get<T>(path: string, what: string): Promise<T> {
		const response = await fetch(`${this.base}${path}`, { headers: { accept: "application/json" } });
		let body: unknown;
		try {
			body = await response.json();
		} catch {
			throw new Error(`${what}: HTTP ${response.status}`);
		}
		if (!response.ok) {
			const message = (body as { error?: string } | null)?.error;
			throw new Error(message ?? `${what}: HTTP ${response.status}`);
		}
		return unwrap<T>(body, what);
	}

	sessions(): Promise<SessionPreview[]> {
		return this.get("/sessions?limit=5000", "Loading sessions");
	}

	session(id: string): Promise<SessionDetail> {
		return this.get(`/sessions/${encodeURIComponent(id)}`, "Loading session");
	}

	search(query: string): Promise<SearchReport> {
		const params = new URLSearchParams({
			query,
			limit: String(SEARCH_LIMIT),
			max_chars: String(SEARCH_BUDGET),
			snippet_chars: String(SNIPPET_CHARS),
		});
		return this.get(`/search?${params}`, "Search");
	}

	sources(): Promise<Source[]> {
		return this.get("/sources", "Loading sources");
	}

	async agents(): Promise<Agents> {
		try {
			return await this.get<Agents>("/agents", "Loading agents");
		} catch {
			return DEFAULT_AGENTS;
		}
	}
}

/**
 * Inside Oqto: data comes only through the App's pinned operations, which
 * run the hstry CLI runner-side (see oqto/hstry.oqtoapp/operations).
 */
export class OqtoSource implements DataSource {
	readonly kind = "oqto" as const;
	constructor(private readonly host: OqtoHost) {}

	private async invoke<T>(id: string, input: Record<string, unknown>, what: string): Promise<T> {
		const ops = this.host.operations;
		if (!ops) throw new Error("This App has not been granted its hstry operations yet.");
		const result = await ops.invoke(id, input as never);
		if (!result.ok) throw new Error(result.message || `${what} failed`);
		return unwrap<T>(result.output, what);
	}

	async sessions(): Promise<SessionPreview[]> {
		const rows = await this.invoke<unknown[]>("hstry.sessions.list", { limit: 1500 }, "Loading sessions");
		return decodeRows<SessionPreview>(rows);
	}

	/**
	 * Operation output is capped at 1 MiB, so the bridge pages transcripts;
	 * fetch every page and join them.
	 */
	async session(id: string): Promise<SessionDetail> {
		let offset = 0;
		let detail: SessionDetail | null = null;
		for (let guard = 0; guard < 200; guard++) {
			const page = await this.invoke<SessionDetail & { page?: { has_more?: boolean; next_offset?: number } }>(
				"hstry.sessions.show",
				{ id, offset, limit: SHOW_PAGE },
				"Loading session",
			);
			const messages = decodeRows<SessionDetail["messages"][number]>(page.messages);
			if (!detail) detail = { ...page, messages };
			else detail.messages.push(...messages);
			const next = page.page?.next_offset;
			if (!page.page?.has_more || next === undefined || next <= offset) break;
			offset = next;
		}
		if (!detail) throw new Error("Loading session failed");
		return detail;
	}

	async search(query: string): Promise<SearchReport> {
		const report = await this.invoke<SearchReport>("hstry.search", { query, limit: SEARCH_LIMIT }, "Search");
		return { ...report, hits: decodeRows(report.hits) };
	}

	sources(): Promise<Source[]> {
		return this.invoke("hstry.sources.list", {}, "Loading sources");
	}

	async agents(): Promise<Agents> {
		return DEFAULT_AGENTS;
	}
}
