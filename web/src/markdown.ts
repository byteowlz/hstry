// Minimal markdown -> DOM. Never builds HTML strings, so transcript content
// can't inject markup: every piece of text becomes a text node.

type Inline = Node;

export interface RenderOptions {
	/** Query terms to mark in text. */
	highlight?: string[];
}

export function renderMarkdown(source: string, options: RenderOptions = {}): DocumentFragment {
	const out = document.createDocumentFragment();
	const lines = source.replace(/\r\n?/g, "\n").split("\n");
	let i = 0;
	let paragraph: string[] = [];

	const flushParagraph = () => {
		if (paragraph.length === 0) return;
		const p = document.createElement("p");
		appendInline(p, paragraph.join("\n"), options);
		out.append(p);
		paragraph = [];
	};

	while (i < lines.length) {
		const line = lines[i]!;
		const fence = /^\s*(```+|~~~+)\s*([\w+#.-]*)\s*$/.exec(line);
		if (fence) {
			flushParagraph();
			const marker = fence[1]!;
			const lang = fence[2] ?? "";
			const body: string[] = [];
			i++;
			while (i < lines.length && !lines[i]!.trim().startsWith(marker)) {
				body.push(lines[i]!);
				i++;
			}
			i++; // closing fence
			out.append(codeBlock(body.join("\n"), lang, options));
			continue;
		}

		const heading = /^(#{1,6})\s+(.*)$/.exec(line);
		if (heading) {
			flushParagraph();
			const level = Math.min(heading[1]!.length + 2, 6);
			const h = document.createElement(`h${level}`);
			appendInline(h, heading[2]!, options);
			out.append(h);
			i++;
			continue;
		}

		if (/^\s*([-*+]|\d+[.)])\s+/.test(line)) {
			flushParagraph();
			const ordered = /^\s*\d+[.)]\s+/.test(line);
			const list = document.createElement(ordered ? "ol" : "ul");
			while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i]!)) {
				const li = document.createElement("li");
				appendInline(li, lines[i]!.replace(/^\s*([-*+]|\d+[.)])\s+/, ""), options);
				list.append(li);
				i++;
			}
			out.append(list);
			continue;
		}

		if (/^\s*>/.test(line)) {
			flushParagraph();
			const quote = document.createElement("blockquote");
			const body: string[] = [];
			while (i < lines.length && /^\s*>/.test(lines[i]!)) {
				body.push(lines[i]!.replace(/^\s*>\s?/, ""));
				i++;
			}
			appendInline(quote, body.join("\n"), options);
			out.append(quote);
			continue;
		}

		if (/^\s*\|.*\|\s*$/.test(line)) {
			// Tables stay aligned as monospace rows rather than being reflowed.
			flushParagraph();
			const body: string[] = [];
			while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i]!)) {
				body.push(lines[i]!);
				i++;
			}
			out.append(codeBlock(body.join("\n"), "", options, "table"));
			continue;
		}

		if (/^\s*(---+|\*\*\*+)\s*$/.test(line)) {
			flushParagraph();
			out.append(document.createElement("hr"));
			i++;
			continue;
		}

		if (line.trim() === "") {
			flushParagraph();
		} else {
			paragraph.push(line);
		}
		i++;
	}
	flushParagraph();
	return out;
}

export function codeBlock(code: string, lang: string, options: RenderOptions = {}, kind = "code"): HTMLElement {
	const figure = document.createElement("figure");
	figure.className = `code-block ${kind}`;
	if (lang) {
		const label = document.createElement("figcaption");
		label.textContent = lang;
		figure.append(label);
	}
	const pre = document.createElement("pre");
	const codeEl = document.createElement("code");
	appendText(codeEl, code, options.highlight);
	pre.append(codeEl);
	figure.append(pre);
	return figure;
}

const INLINE = /(`+)([^`]+?)\1|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*?)\*|\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g;

function appendInline(parent: HTMLElement, text: string, options: RenderOptions): void {
	let last = 0;
	for (const m of text.matchAll(INLINE)) {
		const index = m.index ?? 0;
		if (index > last) appendText(parent, text.slice(last, index), options.highlight);
		let node: Inline;
		if (m[2] !== undefined) {
			const code = document.createElement("code");
			appendText(code, m[2], options.highlight);
			node = code;
		} else if (m[3] !== undefined || m[4] !== undefined) {
			const strong = document.createElement("strong");
			appendText(strong, m[3] ?? m[4] ?? "", options.highlight);
			node = strong;
		} else if (m[5] !== undefined) {
			const em = document.createElement("em");
			appendText(em, m[5], options.highlight);
			node = em;
		} else {
			const a = document.createElement("a");
			a.href = m[7]!;
			a.target = "_blank";
			a.rel = "noopener noreferrer";
			appendText(a, m[6] ?? "", options.highlight);
			node = a;
		}
		parent.append(node);
		last = index + m[0].length;
	}
	if (last < text.length) appendText(parent, text.slice(last), options.highlight);
}

/** Append text, wrapping case-insensitive occurrences of `terms` in <mark>. */
export function appendText(parent: HTMLElement, text: string, terms?: string[]): void {
	const needles = (terms ?? []).map((t) => t.toLowerCase()).filter((t) => t.length > 1);
	if (needles.length === 0) {
		parent.append(document.createTextNode(text));
		return;
	}
	const lower = text.toLowerCase();
	let pos = 0;
	while (pos < text.length) {
		let next = -1;
		let len = 0;
		for (const n of needles) {
			const at = lower.indexOf(n, pos);
			if (at >= 0 && (next < 0 || at < next)) {
				next = at;
				len = n.length;
			}
		}
		if (next < 0) {
			parent.append(document.createTextNode(text.slice(pos)));
			return;
		}
		if (next > pos) parent.append(document.createTextNode(text.slice(pos, next)));
		const mark = document.createElement("mark");
		mark.textContent = text.slice(next, next + len);
		parent.append(mark);
		pos = next + len;
	}
}
