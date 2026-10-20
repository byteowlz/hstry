// Transcript rendering from canonical message parts — same rules as the TUI:
// one header per speaker turn, thinking and long tool output collapsed, tool
// output attached to the turn that requested it (never labelled "you").

import { appendText, codeBlock, renderMarkdown } from "./markdown";
import { partsOf, toolHint, valueText } from "./format";
import type { Message } from "./types";

const OUTPUT_PREVIEW_LINES = 6;

type Speaker = "user" | "assistant";

export interface TranscriptOptions {
	highlight?: string[];
	/** Message index to scroll to and mark (from a content-search hit). */
	focusIdx?: number | null;
}

export function renderTranscript(messages: Message[], options: TranscriptOptions = {}): HTMLElement {
	const root = document.createElement("div");
	root.className = "transcript";
	let turn: HTMLElement | null = null;
	let speaker: Speaker | null = null;
	let lastModel: string | null = null;

	for (const message of messages) {
		if (message.role === "system") {
			const text = message.content.trim().replace(/\s+/g, " ");
			if (!text) continue;
			const note = document.createElement("div");
			note.className = "system-note";
			appendText(note, text, options.highlight);
			root.append(note);
			speaker = null;
			turn = null;
			continue;
		}

		const parts = partsOf(message);
		if (parts.length === 0) continue;
		const onlyToolOutput = parts.every((p) => p.type === "tool_result");
		const who: Speaker =
			onlyToolOutput && speaker ? speaker : message.role === "user" ? "user" : "assistant";

		if (who !== speaker || !turn) {
			turn = document.createElement("section");
			turn.className = `turn ${who}`;
			turn.append(turnHeader(who, message, who === "assistant" && message.model && message.model !== lastModel ? message.model : null));
			if (who === "assistant" && message.model) lastModel = message.model;
			root.append(turn);
			speaker = who;
		}

		const block = document.createElement("div");
		block.className = "message";
		block.dataset.idx = String(message.idx);
		if (options.focusIdx != null && message.idx === options.focusIdx) block.classList.add("focused");

		for (const part of parts) {
			switch (part.type) {
				case "text": {
					const prose = document.createElement("div");
					prose.className = "prose";
					prose.append(renderMarkdown(String((part as { text: string }).text), options));
					block.append(prose);
					break;
				}
				case "thinking":
					block.append(thinking(String((part as { text: string }).text), options));
					break;
				case "tool_call": {
					const call = part as { name?: string; input?: unknown };
					block.append(toolCall(call.name ?? "tool", toolHint(call.input), options));
					break;
				}
				case "tool_result": {
					const result = part as { output?: unknown; isError?: boolean };
					block.append(toolOutput(valueText(result.output), Boolean(result.isError), options));
					break;
				}
				default:
					break;
			}
		}
		if (block.childNodes.length > 0) turn.append(block);
	}

	if (!root.firstChild) {
		const empty = document.createElement("p");
		empty.className = "empty";
		empty.textContent = "This session has no readable messages.";
		root.append(empty);
	}
	return root;
}

function turnHeader(who: Speaker, message: Message, model: string | null): HTMLElement {
	const header = document.createElement("header");
	const name = document.createElement("span");
	name.className = "speaker";
	name.textContent = who === "user" ? "You" : "Assistant";
	header.append(name);
	if (message.created_at) {
		const time = document.createElement("time");
		const date = new Date(message.created_at);
		time.dateTime = date.toISOString();
		time.textContent = date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
		header.append(time);
	}
	if (model) {
		const m = document.createElement("span");
		m.className = "model";
		m.textContent = model;
		header.append(m);
	}
	return header;
}

function thinking(text: string, options: TranscriptOptions): HTMLElement {
	const details = document.createElement("details");
	details.className = "thinking";
	const summary = document.createElement("summary");
	const first = text.trim().split("\n")[0] ?? "";
	summary.append(document.createTextNode("Thinking · "));
	const preview = document.createElement("span");
	appendText(preview, first, options.highlight);
	summary.append(preview);
	details.append(summary);
	const body = document.createElement("div");
	body.className = "thinking-body";
	body.append(renderMarkdown(text, options));
	details.append(body);
	// Open when the search term is in the reasoning, so the hit is visible.
	if (options.highlight?.some((t) => t.length > 1 && text.toLowerCase().includes(t.toLowerCase()))) {
		details.open = true;
	}
	return details;
}

function toolCall(name: string, hint: string, options: TranscriptOptions): HTMLElement {
	const row = document.createElement("div");
	row.className = "tool-call";
	const label = document.createElement("span");
	label.className = "tool-name";
	label.textContent = name;
	row.append(label);
	if (hint) {
		const arg = document.createElement("code");
		arg.className = "tool-arg";
		appendText(arg, hint, options.highlight);
		row.append(arg);
	}
	return row;
}

function toolOutput(output: string, isError: boolean, options: TranscriptOptions): HTMLElement {
	const text = output.replace(/\s+$/, "");
	const lines = text ? text.split("\n") : [];
	const wrapper = document.createElement("div");
	wrapper.className = `tool-output${isError ? " error" : ""}`;
	if (lines.length === 0) {
		wrapper.textContent = isError ? "Error (no output)" : "(no output)";
		return wrapper;
	}
	if (lines.length <= OUTPUT_PREVIEW_LINES) {
		wrapper.append(codeBlock(text, isError ? "error" : "", options, "output"));
		return wrapper;
	}
	const details = document.createElement("details");
	const summary = document.createElement("summary");
	summary.textContent = `${isError ? "Error output" : "Output"} · ${lines.length} lines`;
	details.append(summary);
	const preview = codeBlock(lines.slice(0, OUTPUT_PREVIEW_LINES).join("\n"), "", options, "output preview");
	const full = codeBlock(text, "", options, "output full");
	details.append(full);
	details.addEventListener("toggle", () => {
		preview.hidden = details.open;
	});
	if (options.highlight?.some((t) => t.length > 1 && text.toLowerCase().includes(t.toLowerCase()))) {
		details.open = true;
		preview.hidden = true;
	}
	wrapper.append(preview, details);
	return wrapper;
}
