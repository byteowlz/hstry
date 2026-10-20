// The hstry web UI: find a session, read it, act on it.

import { appendText } from "./markdown";
import { matchQuery } from "./fuzzy";
import {
	adapterOf,
	agentColor,
	excerpt,
	lastActive,
	relativeTime,
	repoName,
	resumeCommand,
	slug,
	titleOf,
	transcriptMarkdown,
} from "./format";
import { renderTranscript } from "./transcript";
import {
	type Agents,
	type DataSource,
	type Message,
	type SearchHit,
	type SessionDetail,
	type SessionPreview,
	type Source,
	messagesOf,
} from "./types";

type Scope = "sessions" | "content";

interface Visible {
	session: SessionPreview;
	titlePositions: number[];
}

interface HitGroup {
	id: string;
	hits: SearchHit[];
}

interface Loaded {
	detail: SessionDetail;
	messages: Message[];
}

const SESSION_ROW = 58;
const HIT_ROW = 82;
const CONTENT_DEBOUNCE_MS = 220;
const NARROW = 820;

const el = <K extends keyof HTMLElementTagNameMap>(
	tag: K,
	className?: string,
	text?: string,
): HTMLElementTagNameMap[K] => {
	const node = document.createElement(tag);
	if (className) node.className = className;
	if (text !== undefined) node.textContent = text;
	return node;
};

export class HstryApp {
	private sessions: SessionPreview[] = [];
	private byId = new Map<string, SessionPreview>();
	private sources = new Map<string, Source>();
	private agents: Agents = { default: "pi", agents: [] };
	private secondary = new Map<string, string>();

	private query = "";
	private scope: Scope = "sessions";
	private visible: Visible[] = [];
	private groups: HitGroup[] = [];
	private contentQuery: string | null = null;
	private searchTimer: ReturnType<typeof setTimeout> | null = null;
	private searchSeq = 0;
	private searching = false;

	private selected: string | null = null;
	private loaded = new Map<string, Loaded>();
	private loadingId: string | null = null;
	private pane: "list" | "detail" = "list";

	// DOM
	private readonly root: HTMLElement;
	private readonly input: HTMLInputElement;
	private readonly scopeButtons: Record<Scope, HTMLButtonElement>;
	private readonly count: HTMLElement;
	private readonly listViewport: HTMLElement;
	private readonly listSpacer: HTMLElement;
	private readonly listRows: HTMLElement;
	private readonly listTitle: HTMLElement;
	private readonly detail: HTMLElement;
	private readonly toast: HTMLElement;
	private toastTimer: ReturnType<typeof setTimeout> | null = null;
	private menu: HTMLElement | null = null;

	constructor(
		mount: HTMLElement,
		private readonly source: DataSource,
	) {
		this.root = el("div", "app");
		this.root.dataset.host = source.kind;

		// Top bar: brand, the one query field, scope switch, counts.
		const top = el("header", "topbar");
		const brand = el("div", "brand", "hstry");
		const search = el("label", "search");
		search.append(searchIcon());
		this.input = el("input");
		this.input.type = "search";
		this.input.placeholder = "Find a session — title, repo, agent, first prompt";
		this.input.setAttribute("aria-label", "Find sessions");
		this.input.autocomplete = "off";
		this.input.spellcheck = false;
		search.append(this.input);
		const keyHint = el("kbd", "key-hint", "/");
		search.append(keyHint);
		const scope = el("div", "scope");
		scope.setAttribute("role", "group");
		scope.setAttribute("aria-label", "Search in");
		this.scopeButtons = {
			sessions: el("button", "", "Sessions"),
			content: el("button", "", "Messages"),
		};
		for (const [key, button] of Object.entries(this.scopeButtons) as [Scope, HTMLButtonElement][]) {
			button.type = "button";
			button.title = key === "sessions" ? "Match titles, repos, agents and first prompts" : "Search inside every message (Tab)";
			button.addEventListener("click", () => {
				this.setScope(key);
				this.input.focus();
			});
			scope.append(button);
		}
		this.count = el("div", "count");
		top.append(brand, search, scope, this.count);

		// Body: list + detail.
		const body = el("main", "body");
		const listPane = el("nav", "list-pane");
		listPane.setAttribute("aria-label", "Sessions");
		this.listTitle = el("div", "pane-title", "Sessions");
		this.listViewport = el("div", "list-viewport");
		this.listViewport.setAttribute("role", "listbox");
		this.listViewport.setAttribute("aria-label", "Sessions");
		this.listViewport.tabIndex = 0;
		this.listSpacer = el("div", "list-spacer");
		this.listRows = el("div", "list-rows");
		this.listViewport.append(this.listSpacer, this.listRows);
		listPane.append(this.listTitle, this.listViewport);
		this.detail = el("section", "detail-pane");
		this.detail.setAttribute("aria-live", "polite");
		body.append(listPane, this.detail);

		this.toast = el("div", "toast");
		this.toast.setAttribute("role", "status");

		this.root.append(top, body, this.toast);
		mount.replaceChildren(this.root);

		this.input.addEventListener("input", () => this.onQuery());
		this.input.addEventListener("keydown", (e) => this.onInputKey(e));
		this.listViewport.addEventListener("scroll", () => this.renderRows());
		window.addEventListener("resize", () => {
			this.applyPane();
			this.renderRows();
		});
		document.addEventListener("keydown", (e) => this.onKey(e));
		document.addEventListener("click", (e) => {
			if (this.menu && !this.menu.contains(e.target as Node)) this.closeMenu();
		});
		this.updateScopeButtons();
		this.applyPane();
	}

	async start(): Promise<void> {
		this.renderDetailMessage("Loading sessions…");
		try {
			const [sessions, sources, agents] = await Promise.all([
				this.source.sessions(),
				this.source.sources().catch(() => [] as Source[]),
				this.source.agents(),
			]);
			this.setData(sessions, sources, agents);
		} catch (error) {
			this.renderDetailMessage(`Could not load sessions: ${(error as Error).message}`, () => this.start());
		}
	}

	private setData(sessions: SessionPreview[], sources: Source[], agents: Agents): void {
		this.sources = new Map(sources.map((s) => [s.id, s]));
		this.agents = agents;
		this.sessions = [...sessions].sort((a, b) => lastActive(b).getTime() - lastActive(a).getTime());
		this.byId = new Map(this.sessions.map((s) => [s.id, s]));
		this.secondary = new Map(
			this.sessions.map((s) => [
				s.id,
				[s.workspace ?? "", adapterOf(s, this.sources), (s.first_user_message ?? "").slice(0, 400)].join(" "),
			]),
		);
		this.refilter();
		if (!this.selected && this.visible[0]) this.select(this.visible[0].session.id);
		if (this.sessions.length === 0) {
			this.renderDetailMessage("No sessions yet. Run `hstry sync` to import history from Claude Code, Codex, Pi and more.");
		}
	}

	// ---------------------------------------------------------------- query

	private onQuery(): void {
		this.query = this.input.value;
		if (this.scope === "sessions") {
			this.refilter();
			this.selectFirst();
			return;
		}
		if (this.searchTimer) clearTimeout(this.searchTimer);
		if (!this.query.trim()) {
			this.groups = [];
			this.contentQuery = null;
			this.refilter();
			this.selectFirst();
			return;
		}
		this.searching = true;
		this.renderCount();
		this.searchTimer = setTimeout(() => void this.runContentSearch(), CONTENT_DEBOUNCE_MS);
	}

	private async runContentSearch(): Promise<void> {
		const q = this.query.trim();
		if (!q) return;
		const seq = ++this.searchSeq;
		this.searching = true;
		this.renderCount();
		try {
			const report = await this.source.search(q);
			if (seq !== this.searchSeq) return; // a newer query won
			this.groups = groupHits(report.hits);
			this.contentQuery = q;
		} catch (error) {
			if (seq !== this.searchSeq) return;
			this.groups = [];
			this.contentQuery = q;
			this.showToast(`Search failed: ${(error as Error).message}`);
		} finally {
			if (seq === this.searchSeq) this.searching = false;
		}
		this.refilter();
		this.selectFirst();
	}

	private setScope(scope: Scope): void {
		if (this.scope === scope) return;
		this.scope = scope;
		this.updateScopeButtons();
		this.groups = [];
		this.contentQuery = null;
		this.input.placeholder =
			scope === "sessions" ? "Find a session — title, repo, agent, first prompt" : "Search inside every message";
		if (scope === "content" && this.query.trim()) {
			void this.runContentSearch();
		} else {
			this.refilter();
			this.selectFirst();
		}
	}

	private updateScopeButtons(): void {
		for (const [key, button] of Object.entries(this.scopeButtons) as [Scope, HTMLButtonElement][]) {
			button.setAttribute("aria-pressed", String(key === this.scope));
		}
	}

	private showingContent(): boolean {
		return this.scope === "content" && this.contentQuery !== null && this.query.trim() !== "";
	}

	private refilter(): void {
		const q = this.scope === "sessions" ? this.query.trim() : "";
		if (!q) {
			this.visible = this.sessions.map((session) => ({ session, titlePositions: [] }));
		} else {
			const scored: Array<{ score: number; v: Visible }> = [];
			for (const session of this.sessions) {
				const m = matchQuery(q, titleOf(session), this.secondary.get(session.id) ?? "");
				if (m) scored.push({ score: m.score, v: { session, titlePositions: m.titlePositions } });
			}
			// Relevance first, recency breaks ties (sessions are already newest-first).
			scored.sort((a, b) => b.score - a.score);
			this.visible = scored.map((s) => s.v);
		}
		this.renderCount();
		this.renderListTitle();
		this.listViewport.scrollTop = 0;
		this.renderRows();
	}

	private rowIds(): string[] {
		return this.showingContent() ? this.groups.map((g) => g.id) : this.visible.map((v) => v.session.id);
	}

	private selectFirst(): void {
		const first = this.rowIds()[0];
		if (first) this.select(first);
		else {
			this.selected = null;
			this.renderDetailMessage(
				this.sessions.length === 0
					? "No sessions yet. Run `hstry sync` to import your agent history."
					: this.searching
						? "Searching…"
						: `Nothing matches “${this.query.trim()}”. ${this.scope === "sessions" ? "Try Messages to search inside sessions." : "Try fewer words."}`,
			);
		}
	}

	// ---------------------------------------------------------------- list

	private renderCount(): void {
		const total = this.sessions.length;
		if (this.searching) this.count.textContent = "Searching…";
		else if (this.showingContent()) {
			const hits = this.groups.reduce((n, g) => n + g.hits.length, 0);
			this.count.textContent = `${hits} hits in ${this.groups.length} sessions`;
		} else if (this.scope === "sessions" && this.query.trim()) {
			this.count.textContent = `${this.visible.length} of ${total}`;
		} else {
			this.count.textContent = `${total} sessions`;
		}
	}

	private renderListTitle(): void {
		this.listTitle.textContent = this.showingContent()
			? `${this.groups.length} sessions mention “${this.contentQuery}”`
			: this.query.trim() && this.scope === "sessions"
				? `${this.visible.length} matching sessions`
				: "Recent sessions";
	}

	private rowHeight(): number {
		return this.showingContent() ? HIT_ROW : SESSION_ROW;
	}

	private renderRows(): void {
		const ids = this.rowIds();
		const height = this.rowHeight();
		this.listSpacer.style.height = `${ids.length * height}px`;
		const top = this.listViewport.scrollTop;
		const view = this.listViewport.clientHeight || 800;
		const start = Math.max(0, Math.floor(top / height) - 4);
		const end = Math.min(ids.length, Math.ceil((top + view) / height) + 4);
		const rows: HTMLElement[] = [];
		for (let i = start; i < end; i++) {
			const row = this.showingContent() ? this.hitRow(this.groups[i]!) : this.sessionRow(this.visible[i]!);
			row.style.transform = `translateY(${i * height}px)`;
			row.style.height = `${height}px`;
			rows.push(row);
		}
		this.listRows.replaceChildren(...rows);
		if (ids.length === 0) {
			const empty = el("div", "list-empty");
			empty.textContent = this.sessions.length === 0 ? "No sessions yet." : this.searching ? "Searching…" : "No matches.";
			this.listRows.replaceChildren(empty);
		}
	}

	private rowShell(id: string): HTMLElement {
		const row = el("div", "row");
		row.setAttribute("role", "option");
		row.dataset.id = id;
		const selected = id === this.selected;
		row.setAttribute("aria-selected", String(selected));
		if (selected) row.classList.add("selected");
		row.addEventListener("click", () => {
			this.select(id);
			if (this.isNarrow()) this.showPane("detail");
		});
		row.addEventListener("dblclick", () => this.openMenu("open"));
		return row;
	}

	private metaLine(session: SessionPreview | undefined, fallbackAdapter?: string): HTMLElement {
		const meta = el("div", "row-meta");
		const adapter = session ? adapterOf(session, this.sources) : (fallbackAdapter ?? "unknown");
		const dot = el("span", "dot");
		dot.style.background = agentColor(adapter);
		meta.append(dot);
		const bits = [adapter];
		const repo = repoName(session?.workspace);
		if (repo) bits.push(repo);
		if (session) bits.push(relativeTime(lastActive(session)));
		if (session?.message_count) bits.push(`${session.message_count} msgs`);
		meta.append(el("span", "", bits.join(" · ")));
		return meta;
	}

	private sessionRow(v: Visible): HTMLElement {
		const row = this.rowShell(v.session.id);
		const title = el("div", "row-title");
		highlightChars(title, titleOf(v.session), v.titlePositions);
		row.append(title, this.metaLine(v.session));
		return row;
	}

	private hitRow(group: HitGroup): HTMLElement {
		const row = this.rowShell(group.id);
		const session = this.byId.get(group.id);
		const best = group.hits[0]!;
		const head = el("div", "row-head");
		const title = el("div", "row-title", session ? titleOf(session) : (best.title ?? "(untitled)"));
		const badge = el("span", "hit-count", group.hits.length === 1 ? "1 hit" : `${group.hits.length} hits`);
		head.append(title, badge);
		const snippet = el("div", "row-snippet");
		appendText(snippet, excerpt(best.snippet, this.contentQuery ?? "", 150), terms(this.contentQuery));
		row.append(head, this.metaLine(session, best.provenance?.source ?? undefined), snippet);
		return row;
	}

	private ensureVisible(id: string): void {
		const index = this.rowIds().indexOf(id);
		if (index < 0) return;
		const height = this.rowHeight();
		const top = index * height;
		const viewport = this.listViewport;
		if (top < viewport.scrollTop) viewport.scrollTop = top;
		else if (top + height > viewport.scrollTop + viewport.clientHeight) {
			viewport.scrollTop = top + height - viewport.clientHeight;
		}
	}

	private move(delta: number): void {
		const ids = this.rowIds();
		if (ids.length === 0) return;
		const current = this.selected ? ids.indexOf(this.selected) : -1;
		const next = Math.max(0, Math.min(ids.length - 1, (current < 0 ? 0 : current) + delta));
		this.select(ids[next]!);
	}

	// ---------------------------------------------------------------- detail

	private select(id: string): void {
		this.selected = id;
		this.ensureVisible(id);
		this.renderRows();
		void this.loadDetail(id);
	}

	private async loadDetail(id: string): Promise<void> {
		const cached = this.loaded.get(id);
		if (cached) {
			this.renderDetail(id, cached);
			return;
		}
		this.loadingId = id;
		this.renderDetailMessage("Loading…");
		try {
			const detail = await this.source.session(id);
			const loaded = { detail, messages: messagesOf(detail) };
			this.loaded.set(id, loaded);
			if (this.selected === id) this.renderDetail(id, loaded);
		} catch (error) {
			if (this.selected === id) {
				this.renderDetailMessage(`Could not load this session: ${(error as Error).message}`, () => {
					this.loaded.delete(id);
					void this.loadDetail(id);
				});
			}
		} finally {
			if (this.loadingId === id) this.loadingId = null;
		}
	}

	private renderDetailMessage(text: string, retry?: () => void): void {
		const box = el("div", "detail-empty");
		box.append(el("p", "", text));
		if (retry) {
			const button = el("button", "button", "Try again");
			button.type = "button";
			button.addEventListener("click", retry);
			box.append(button);
		}
		this.detail.replaceChildren(box);
	}

	private renderDetail(id: string, loaded: Loaded): void {
		const session = this.byId.get(id) ?? loaded.detail;
		const adapter = adapterOf(session, this.sources);

		const header = el("header", "detail-header");
		if (this.isNarrow()) {
			const back = el("button", "button ghost back", "← Sessions");
			back.type = "button";
			back.addEventListener("click", () => this.showPane("list"));
			header.append(back);
		}
		const title = el("h1", "detail-title", titleOf(session));
		const meta = el("div", "detail-meta");
		const dot = el("span", "dot");
		dot.style.background = agentColor(adapter);
		meta.append(dot, el("span", "", adapter));
		for (const bit of [
			session.model,
			session.workspace,
			relativeTime(lastActive(session)),
			`${loaded.messages.length} messages`,
			session.readable_id,
		]) {
			if (bit) meta.append(el("span", "sep", "·"), el("span", "", bit));
		}

		const actions = el("div", "actions");
		actions.setAttribute("role", "toolbar");
		actions.setAttribute("aria-label", "Session actions");
		actions.append(
			this.actionButton("Open in…", "o", () => this.openMenu("open"), "primary"),
			this.actionButton("Copy transcript", "y", () => this.copyTranscript()),
			this.actionButton("Copy id", "Y", () => this.copy(session.id, "Session id")),
			this.actionButton("Copy path", "p", () => this.copyPath()),
			this.actionButton(this.source.kind === "oqto" ? "Copy as…" : "Export…", "e", () => this.openMenu("export")),
		);
		header.append(title, meta, actions);

		const hits = this.showingContent() ? this.groups.find((g) => g.id === id) : undefined;
		const transcript = renderTranscript(loaded.messages, {
			highlight: this.showingContent() ? terms(this.contentQuery) : undefined,
			focusIdx: hits?.hits[0]?.message_idx ?? null,
		});
		const scroller = el("div", "detail-scroll");
		scroller.append(transcript);
		this.detail.replaceChildren(header, scroller);
		const focused = transcript.querySelector<HTMLElement>(".message.focused");
		if (focused) requestAnimationFrame(() => focused.scrollIntoView({ block: "center" }));
	}

	private actionButton(label: string, key: string, run: () => void, variant = ""): HTMLButtonElement {
		const button = el("button", `button ${variant}`.trim());
		button.type = "button";
		button.dataset.action = key;
		button.append(document.createTextNode(label));
		button.append(el("kbd", "", key));
		button.addEventListener("click", (e) => {
			e.stopPropagation();
			run();
		});
		return button;
	}

	// ---------------------------------------------------------------- actions

	private current(): { session: SessionPreview; loaded: Loaded | undefined; adapter: string } | null {
		if (!this.selected) return null;
		const session = this.byId.get(this.selected);
		if (!session) return null;
		return { session, loaded: this.loaded.get(this.selected), adapter: adapterOf(session, this.sources) };
	}

	private async copyTranscript(): Promise<void> {
		const c = this.current();
		if (!c?.loaded) return this.showToast("Open a session first");
		await this.copy(transcriptMarkdown(c.session, c.loaded.messages, c.adapter), `Transcript (${c.loaded.messages.length} messages)`);
	}

	private async copyPath(): Promise<void> {
		const c = this.current();
		if (!c) return;
		const file = typeof c.session.metadata?.file === "string" ? (c.session.metadata.file as string) : null;
		const path = file ?? this.sources.get(c.session.source_id)?.path ?? null;
		if (!path) return this.showToast("No on-disk path recorded for this session");
		await this.copy(path, "Path");
	}

	private async copy(text: string, what: string): Promise<void> {
		if (await writeClipboard(text)) {
			this.showToast(`${what} copied`);
		} else {
			this.showTextDialog(what, text);
		}
	}

	private exportAs(format: "md" | "json"): void {
		const c = this.current();
		if (!c?.loaded) return this.showToast("Open a session first");
		const body =
			format === "md"
				? transcriptMarkdown(c.session, c.loaded.messages, c.adapter)
				: JSON.stringify(c.loaded.detail, null, 2);
		if (this.source.kind === "oqto") {
			void this.copy(body, format === "md" ? "Markdown" : "JSON");
			return;
		}
		const blob = new Blob([body], { type: format === "md" ? "text/markdown" : "application/json" });
		const url = URL.createObjectURL(blob);
		const a = el("a");
		a.href = url;
		a.download = `hstry-${slug(titleOf(c.session))}-${c.session.id.slice(0, 8)}.${format}`;
		document.body.append(a);
		a.click();
		a.remove();
		setTimeout(() => URL.revokeObjectURL(url), 1000);
		this.showToast(`Downloaded ${a.download}`);
	}

	private openMenu(kind: "open" | "export"): void {
		const c = this.current();
		if (!c) return;
		this.closeMenu();
		const anchor = this.detail.querySelector<HTMLElement>(`[data-action="${kind === "open" ? "o" : "e"}"]`);
		const menu = el("div", "menu");
		menu.setAttribute("role", "menu");
		const items: Array<{ label: string; detail?: string; run: () => void }> = [];
		if (kind === "open") {
			const agents = [...new Set([this.agents.default, ...this.agents.agents])].filter(Boolean);
			for (const agent of agents) {
				items.push({
					label: agent,
					detail: agent === this.agents.default ? "default" : undefined,
					run: () => void this.copy(resumeCommand(c.session.id, agent), `Command to open in ${agent}`),
				});
			}
			const note = el("div", "menu-note", "Copies the command that continues this session in the agent.");
			menu.append(note);
		} else {
			const verb = this.source.kind === "oqto" ? "Copy" : "Download";
			items.push({ label: `${verb} Markdown`, run: () => this.exportAs("md") });
			items.push({ label: `${verb} JSON`, run: () => this.exportAs("json") });
		}
		for (const item of items) {
			const button = el("button", "menu-item");
			button.type = "button";
			button.setAttribute("role", "menuitem");
			button.append(el("span", "", item.label));
			if (item.detail) button.append(el("span", "menu-detail", item.detail));
			button.addEventListener("click", (e) => {
				e.stopPropagation();
				this.closeMenu();
				item.run();
			});
			menu.append(button);
		}
		this.root.append(menu);
		const rect = (anchor ?? this.detail).getBoundingClientRect();
		menu.style.top = `${rect.bottom + 4}px`;
		menu.style.left = `${Math.max(8, Math.min(rect.left, window.innerWidth - 280))}px`;
		this.menu = menu;
		menu.querySelector<HTMLButtonElement>(".menu-item")?.focus();
	}

	private closeMenu(): void {
		this.menu?.remove();
		this.menu = null;
	}

	private showToast(text: string): void {
		this.toast.textContent = text;
		this.toast.classList.add("visible");
		if (this.toastTimer) clearTimeout(this.toastTimer);
		this.toastTimer = setTimeout(() => this.toast.classList.remove("visible"), 2600);
	}

	private showTextDialog(title: string, text: string): void {
		const dialog = el("dialog", "text-dialog");
		const heading = el("h2", "", `${title} — copy it from here`);
		const area = el("textarea");
		area.value = text;
		area.readOnly = true;
		area.rows = Math.min(20, Math.max(3, text.split("\n").length));
		const close = el("button", "button", "Close");
		close.type = "button";
		close.addEventListener("click", () => dialog.close());
		dialog.addEventListener("close", () => dialog.remove());
		dialog.append(heading, area, close);
		this.root.append(dialog);
		dialog.showModal();
		area.select();
	}

	// ---------------------------------------------------------------- keys & panes

	private onInputKey(e: KeyboardEvent): void {
		switch (e.key) {
			case "ArrowDown":
				e.preventDefault();
				this.move(1);
				break;
			case "ArrowUp":
				e.preventDefault();
				this.move(-1);
				break;
			case "Tab":
				if (!e.shiftKey || this.scope === "content") {
					e.preventDefault();
					this.setScope(this.scope === "sessions" ? "content" : "sessions");
				}
				break;
			case "Enter":
				e.preventDefault();
				if (this.scope === "content" && this.searchTimer) {
					clearTimeout(this.searchTimer);
					void this.runContentSearch();
				}
				this.input.blur();
				this.listViewport.focus();
				if (this.isNarrow() && this.selected) this.showPane("detail");
				break;
			case "Escape":
				e.preventDefault();
				if (this.input.value) {
					this.input.value = "";
					this.onQuery();
				} else {
					this.input.blur();
				}
				break;
		}
	}

	private onKey(e: KeyboardEvent): void {
		if (e.defaultPrevented || e.target === this.input) return;
		const target = e.target as HTMLElement | null;
		if (target && (target.tagName === "TEXTAREA" || target.tagName === "INPUT" || target.isContentEditable)) return;
		if (e.metaKey || e.ctrlKey || e.altKey) {
			if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
				e.preventDefault();
				this.input.focus();
				this.input.select();
			}
			return;
		}
		if (this.menu && e.key === "Escape") {
			e.preventDefault();
			this.closeMenu();
			return;
		}
		switch (e.key) {
			case "/":
				e.preventDefault();
				this.input.focus();
				this.input.select();
				break;
			case "j":
			case "ArrowDown":
				if (this.pane === "list" || !this.isNarrow()) {
					e.preventDefault();
					this.move(1);
				}
				break;
			case "k":
			case "ArrowUp":
				if (this.pane === "list" || !this.isNarrow()) {
					e.preventDefault();
					this.move(-1);
				}
				break;
			case "Enter":
				if (this.isNarrow() && this.selected) this.showPane("detail");
				break;
			case "Escape":
				if (this.isNarrow() && this.pane === "detail") this.showPane("list");
				else if (this.input.value) {
					this.input.value = "";
					this.onQuery();
				}
				break;
			case "o":
				e.preventDefault();
				this.openMenu("open");
				break;
			case "y":
				void this.copyTranscript();
				break;
			case "Y":
				if (this.selected) void this.copy(this.selected, "Session id");
				break;
			case "p":
				void this.copyPath();
				break;
			case "e":
				e.preventDefault();
				this.openMenu("export");
				break;
			case "r":
				this.loaded.clear();
				void this.start();
				this.showToast("Reloaded");
				break;
		}
	}

	private isNarrow(): boolean {
		return window.innerWidth < NARROW;
	}

	private showPane(pane: "list" | "detail"): void {
		this.pane = pane;
		this.applyPane();
		if (pane === "list") requestAnimationFrame(() => this.renderRows());
		if (pane === "detail" && this.selected) {
			const loaded = this.loaded.get(this.selected);
			if (loaded) this.renderDetail(this.selected, loaded);
		}
	}

	private applyPane(): void {
		this.root.dataset.pane = this.isNarrow() ? this.pane : "both";
	}
}

function groupHits(hits: SearchHit[]): HitGroup[] {
	const order: string[] = [];
	const byId = new Map<string, SearchHit[]>();
	for (const hit of hits) {
		if (!byId.has(hit.conversation_id)) {
			order.push(hit.conversation_id);
			byId.set(hit.conversation_id, []);
		}
		byId.get(hit.conversation_id)!.push(hit);
	}
	return order.map((id) => ({ id, hits: byId.get(id)! }));
}

function terms(query: string | null): string[] {
	return (query ?? "").split(/\s+/).filter((t) => t.length > 1);
}

function highlightChars(parent: HTMLElement, text: string, positions: number[]): void {
	if (positions.length === 0) {
		parent.textContent = text;
		return;
	}
	const set = new Set(positions);
	let run = "";
	let runMatched = false;
	const flush = () => {
		if (!run) return;
		if (runMatched) {
			const mark = document.createElement("mark");
			mark.textContent = run;
			parent.append(mark);
		} else parent.append(document.createTextNode(run));
		run = "";
	};
	Array.from(text).forEach((ch, i) => {
		const matched = set.has(i);
		if (matched !== runMatched) flush();
		runMatched = matched;
		run += ch;
	});
	flush();
}

async function writeClipboard(text: string): Promise<boolean> {
	try {
		if (navigator.clipboard?.writeText) {
			await navigator.clipboard.writeText(text);
			return true;
		}
	} catch {
		// Sandboxed frames may deny the async clipboard; try the legacy path.
	}
	try {
		const area = document.createElement("textarea");
		area.value = text;
		area.setAttribute("readonly", "");
		area.style.position = "fixed";
		area.style.opacity = "0";
		document.body.append(area);
		area.select();
		const ok = document.execCommand("copy");
		area.remove();
		return ok;
	} catch {
		return false;
	}
}

function searchIcon(): SVGSVGElement {
	const ns = "http://www.w3.org/2000/svg";
	const svg = document.createElementNS(ns, "svg");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("aria-hidden", "true");
	svg.classList.add("icon");
	const circle = document.createElementNS(ns, "circle");
	circle.setAttribute("cx", "11");
	circle.setAttribute("cy", "11");
	circle.setAttribute("r", "7");
	const line = document.createElementNS(ns, "path");
	line.setAttribute("d", "M20 20l-4-4");
	svg.append(circle, line);
	return svg;
}
