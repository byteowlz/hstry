// Pure presentation helpers shared by the list, transcript and actions.

import type { Message, Part, SessionPreview, Source } from "./types";

export function adapterOf(session: SessionPreview, sources: Map<string, Source>): string {
	return sources.get(session.source_id)?.adapter ?? session.harness ?? session.source_id;
}

export function titleOf(session: SessionPreview): string {
	const title = session.title?.trim();
	if (title) return title;
	const first = session.first_user_message?.trim().replace(/\s+/g, " ");
	return first ? first.slice(0, 120) : "(untitled)";
}

export function repoName(workspace: string | null | undefined): string | null {
	if (!workspace) return null;
	const parts = workspace.split(/[\\/]/).filter(Boolean);
	return parts[parts.length - 1] ?? workspace;
}

export function lastActive(session: SessionPreview): Date {
	return new Date(session.updated_at ?? session.created_at);
}

/** Compact relative time: "now", "5m ago", "3h ago", "2d ago", "Oct 4", "2025-12-01". */
export function relativeTime(date: Date, now = new Date()): string {
	const seconds = (now.getTime() - date.getTime()) / 1000;
	if (seconds < 60) return "now";
	if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
	if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h ago`;
	if (seconds < 7 * 86_400) return `${Math.floor(seconds / 86_400)}d ago`;
	if (date.getFullYear() === now.getFullYear()) {
		return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
	}
	return date.toISOString().slice(0, 10);
}

/**
 * Stable agent color from the active scheme's base24 hue slots, so it follows
 * light/dark and host re-themes. Falls back to role tokens.
 */
export function agentColor(adapter: string): string {
	const a = adapter.toLowerCase();
	if (a.includes("claude")) return "var(--base09, var(--warning))";
	if (a === "pi" || a.includes("pi-")) return "var(--base0E, var(--info))";
	if (a.includes("codex")) return "var(--base0C, var(--info))";
	if (a.includes("opencode")) return "var(--base0B, var(--success))";
	if (a.includes("gemini")) return "var(--base0D, var(--info))";
	if (a.includes("chatgpt")) return "var(--base0B, var(--success))";
	const slots = ["08", "09", "0A", "0B", "0C", "0D", "0E"];
	let hash = 0;
	for (const ch of a) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
	return `var(--base${slots[hash % slots.length]}, var(--primary))`;
}

/** Word-aligned one-line excerpt around the first query term. */
export function excerpt(text: string, query: string, width = 160): string {
	const flat = text
		.split(/\s+/)
		.filter((w) => w && !w.startsWith("```") && !/^#+$/.test(w))
		.join(" ");
	const lower = flat.toLowerCase();
	const hits = query
		.split(/\s+/)
		.filter(Boolean)
		.map((t) => lower.indexOf(t.toLowerCase()))
		.filter((i) => i >= 0);
	if (hits.length === 0) return truncate(flat, width);
	let start = Math.max(0, Math.min(...hits) - Math.floor(width / 4));
	if (start > 0) {
		const space = flat.lastIndexOf(" ", start);
		start = space >= 0 ? space + 1 : 0;
	}
	return truncate((start > 0 ? "…" : "") + flat.slice(start), width);
}

export function truncate(s: string, max: number): string {
	const chars = Array.from(s);
	return chars.length <= max ? s : `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

export function partsOf(message: Message): Part[] {
	const parts = Array.isArray(message.parts_json) ? message.parts_json : [];
	if (parts.length > 0) return parts;
	const text = message.content?.trim() ?? "";
	if (!text) return [];
	return message.role === "tool"
		? [{ type: "tool_result", output: text }]
		: [{ type: "text", text }];
}

/** The most recognizable argument of a tool call, on one line. */
export function toolHint(input: unknown): string {
	if (input == null) return "";
	if (typeof input === "string") return oneLine(input);
	if (typeof input === "object" && !Array.isArray(input)) {
		const obj = input as Record<string, unknown>;
		for (const key of ["command", "cmd", "file_path", "path", "filePath", "pattern", "query", "url", "code", "prompt", "description"]) {
			const v = obj[key];
			if (typeof v === "string" && v.trim()) return oneLine(v);
			if (Array.isArray(v) && v.length) return oneLine(v.map(String).join(" "));
		}
	}
	return oneLine(JSON.stringify(input));
}

export function valueText(value: unknown): string {
	if (value == null) return "";
	if (typeof value === "string") return value;
	if (Array.isArray(value)) {
		return value
			.map((item) =>
				item && typeof item === "object" && typeof (item as { text?: unknown }).text === "string"
					? (item as { text: string }).text
					: JSON.stringify(item),
			)
			.join("\n");
	}
	return JSON.stringify(value, null, 2);
}

export function oneLine(s: string): string {
	return s.split(/\s+/).filter(Boolean).join(" ");
}

/** A transcript as markdown, matching the TUI's copy format. */
export function transcriptMarkdown(session: SessionPreview, messages: Message[], adapter: string): string {
	const lines: string[] = [`# ${titleOf(session)}`, ""];
	lines.push(`- Session: ${session.id}`);
	lines.push(`- Agent: ${adapter}`);
	if (session.workspace) lines.push(`- Workspace: ${session.workspace}`);
	lines.push(`- Created: ${new Date(session.created_at).toISOString()}`);
	if (session.model) lines.push(`- Model: ${session.model}`);
	lines.push(`- Messages: ${messages.length}`, "");
	for (const message of messages) {
		lines.push("---", "", `## ${roleLabel(message.role)}`, "");
		for (const part of partsOf(message)) {
			if (part.type === "text") lines.push(String((part as { text: string }).text).trim(), "");
			else if (part.type === "thinking") {
				lines.push("> " + String((part as { text: string }).text).trim().replace(/\n/g, "\n> "), "");
			} else if (part.type === "tool_call") {
				const call = part as { name?: string; input?: unknown };
				lines.push(`**${call.name ?? "tool"}** \`${toolHint(call.input)}\``, "");
			} else if (part.type === "tool_result") {
				const result = part as { output?: unknown; isError?: boolean };
				lines.push(result.isError ? "Error:" : "Output:", "```", valueText(result.output).trimEnd(), "```", "");
			}
		}
	}
	return `${lines.join("\n").trimEnd()}\n`;
}

function roleLabel(role: string): string {
	switch (role) {
		case "user":
			return "User";
		case "assistant":
			return "Assistant";
		case "tool":
			return "Tool";
		case "system":
			return "System";
		default:
			return role;
	}
}

export function slug(title: string): string {
	const s = title
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "")
		.slice(0, 40)
		.replace(/-+$/g, "");
	return s || "session";
}

/** The command a user runs to continue a session in an agent. */
export function resumeCommand(id: string, agent: string | null): string {
	return agent ? `hstry resume ${id} --agent ${agent}` : `hstry resume ${id}`;
}
