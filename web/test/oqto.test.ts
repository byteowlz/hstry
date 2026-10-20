// The in-Oqto data path end to end, minus Oqto's runner: the App SDK's test
// host (real MessageChannel protocol) -> OqtoSource -> the pinned operations
// table -> operations/hstry-bridge -> the real hstry binary -> a fixture
// database. Proves operation output stays under the 1 MiB runtime limit.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createTestHost } from "@byteowlz/oqto-app-sdk/testing";
import { OqtoSource } from "../src/sources";
import { messagesOf } from "../src/types";

const repo = join(import.meta.dir, "../..");
const app = join(repo, "oqto/hstry.oqtoapp");
const bridge = join(app, "operations/hstry-bridge");
const hstryBin = process.env.HSTRY_BIN ?? join(repo, "target/debug/hstry");
const MAX_OUTPUT = 1_048_576; // oqto-apps MAX_OPERATION_OUTPUT_BYTES

let dir = "";
let env: Record<string, string> = {};
const sizes: Record<string, number> = {};

/** Operation id -> bridge subcommand, read from the pinned table. */
function operationTable(): Map<string, string> {
	const text = readFileSync(join(app, "operations/table.toml"), "utf8");
	const map = new Map<string, string>();
	for (const block of text.split("[[operation]]").slice(1)) {
		const id = /id = "([^"]+)"/.exec(block)?.[1];
		const sub = /exec = \["operations\/hstry-bridge", "([^"]+)"\]/.exec(block)?.[1];
		if (id && sub) map.set(id, sub);
	}
	return map;
}

function runBridge(sub: string, input: unknown) {
	const result = Bun.spawnSync([bridge, sub], {
		stdin: new TextEncoder().encode(JSON.stringify(input ?? {})),
		env,
		cwd: dir,
	});
	const stdout = result.stdout.toString();
	sizes[sub] = Math.max(sizes[sub] ?? 0, result.stdout.length);
	if (result.stdout.length > MAX_OUTPUT) throw new Error(`${sub} output ${result.stdout.length} bytes exceeds 1 MiB`);
	const json = JSON.parse(stdout || "null");
	// The runner reports a non-zero exit as a failed operation with the
	// bridge's error message (OqtoOperationFailure).
	if (!result.success) {
		return { ok: false as const, reason: "failed" as const, code: `exit-${result.exitCode}`, message: json?.error?.message ?? "bridge failed" };
	}
	return { ok: true as const, output: json };
}

function piSession(i: number, title: string, messages: Array<{ role: string; text?: string; tool?: string; output?: string }>) {
	const id = `019a0000-0000-7000-8000-${String(i).padStart(12, "0")}`;
	const at = Date.parse("2026-10-01T00:00:00Z") + i * 60_000;
	const lines: unknown[] = [{ type: "session", version: 3, id, timestamp: new Date(at).toISOString(), cwd: `/work/repo-${i % 7}` }];
	let parent: string | null = null;
	messages.forEach((m, n) => {
		const eid = `e${n}`;
		const ts = new Date(at + n * 1000).toISOString();
		let message: unknown;
		if (m.role === "user") message = { role: "user", content: m.text, timestamp: at + n * 1000 };
		else if (m.tool) message = { role: "assistant", content: [{ type: "toolCall", id: `t${n}`, name: m.tool, arguments: { command: "run" } }], timestamp: at + n * 1000 };
		else if (m.output !== undefined) message = { role: "toolResult", toolCallId: `t${n - 1}`, toolName: "bash", content: [{ type: "text", text: m.output }], isError: false, timestamp: at + n * 1000 };
		else message = { role: "assistant", content: [{ type: "text", text: m.text }], timestamp: at + n * 1000 };
		lines.push({ type: "message", id: eid, parentId: parent, timestamp: ts, message });
		parent = eid;
	});
	lines.push({ type: "session_info", id: "info", parentId: parent, timestamp: new Date(at).toISOString(), name: title });
	const sessionsDir = join(dir, "home/.pi/agent/sessions/--work--");
	mkdirSync(sessionsDir, { recursive: true });
	writeFileSync(join(sessionsDir, `${String(i).padStart(6, "0")}_${id}.jsonl`), lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
}

beforeAll(() => {
	if (!existsSync(hstryBin)) throw new Error(`hstry binary not found at ${hstryBin}; run \`cargo build -p hstry-cli\` or set HSTRY_BIN`);
	dir = mkdtempSync(join(tmpdir(), "hstry-oqto-"));
	const longPrompt = "Please investigate the flaky retry test in the sync loop. ".repeat(40);
	for (let i = 0; i < 1600; i++) {
		piSession(i, `Session ${i}: ${"a long descriptive title ".repeat(12)}`, [
			{ role: "user", text: `${longPrompt} (#${i})` },
			{ role: "assistant", text: `Answer ${i} mentioning backoff.` },
		]);
	}
	// One long session: 400 messages and a 300 KB tool output.
	const long: Array<{ role: string; text?: string; tool?: string; output?: string }> = [{ role: "user", text: "Run the whole suite" }];
	for (let n = 0; n < 133; n++) {
		long.push({ role: "assistant", tool: "bash" });
		long.push({ role: "tool", output: n === 5 ? "x".repeat(300_000) : `ok ${n}` });
		long.push({ role: "assistant", text: `step ${n}` });
	}
	piSession(9999, "The long session", long);

	const config = join(dir, "xdg/hstry/config.toml");
	mkdirSync(join(dir, "xdg/hstry"), { recursive: true });
	writeFileSync(config, `database = "${join(dir, "hstry.db")}"\nadapter_paths = ["${join(repo, "adapters")}"]\n`);
	env = {
		PATH: `${join(hstryBin, "..")}:${process.env.PATH ?? ""}`,
		HOME: join(dir, "home"),
		XDG_CONFIG_HOME: join(dir, "xdg"),
		XDG_DATA_HOME: join(dir, "data"),
		XDG_STATE_HOME: join(dir, "state"),
		HSTRY_NO_SERVICE: "1",
		HSTRY_NO_API: "1",
	};
	const imported = Bun.spawnSync([hstryBin, "import", "-a", "pi", join(dir, "home/.pi/agent/sessions")], { env });
	if (!imported.success) throw new Error(`import failed: ${imported.stderr}`);
}, 120_000);

afterAll(() => {
	if (dir) rmSync(dir, { recursive: true, force: true });
});

function connect() {
	const table = operationTable();
	const host = createTestHost({
		capabilities: ["operations", "theme"],
		operations: [...table.entries()].map(([id, sub]) => ({ id, handler: (input) => runBridge(sub, input) })),
	});
	return { host, source: new OqtoSource(host.connect()) };
}

describe("hstry Oqto App data path", () => {
	test("the pinned table declares exactly the operations the manifest requests", () => {
		const manifest = readFileSync(join(app, "oqto-app.toml"), "utf8");
		const requested = [...manifest.matchAll(/"(hstry\.[a-z.]+)"/g)].map((m) => m[1] ?? "").sort();
		expect([...operationTable().keys()].sort()).toEqual(requested);
	});

	test("sessions: compact list of recent sessions under the output limit", async () => {
		const { source, host } = connect();
		const sessions = await source.sessions();
		// 1600 sessions with long titles and prompts: the bridge fills the
		// list by size and stops under the limit.
		expect(sessions.length).toBeGreaterThan(900);
		expect(sessions.length).toBeLessThanOrEqual(1500);
		expect(sizes.sessions!).toBeLessThan(MAX_OUTPUT);
		const first = sessions[0]!;
		expect(first.title?.length ?? 0).toBeLessThanOrEqual(301);
		expect(first.first_user_message?.length ?? 0).toBeLessThanOrEqual(201);
		expect(Object.keys(first.metadata ?? {})).toEqual(["file"]);
		expect(host.invocations.map((i) => i.id)).toEqual(["hstry.sessions.list"]);
		host.close();
	});

	test("show: pages a long transcript and marks cut output", async () => {
		const { source, host } = connect();
		const report = await source.search("whole suite");
		const id = report.hits.find((h) => h.title === "The long session")?.conversation_id;
		expect(id).toBeDefined();
		const detail = await source.session(id!);
		const messages = messagesOf(detail);
		expect(messages.length).toBe(400);
		expect(messages.map((m) => m.idx)).toEqual([...Array(400).keys()]);
		// Pages fill by size: the page with the 300 KB output still fits after
		// its cut, so 400 messages take at least 3 pages of at most 150.
		const shows = host.invocations.filter((i) => i.id === "hstry.sessions.show");
		expect(shows.length).toBeGreaterThanOrEqual(3);
		expect(sizes.show!).toBeLessThan(MAX_OUTPUT);
		const cut = JSON.stringify(messages).includes("more characters not shown here");
		expect(cut).toBe(true);
		host.close();
	}, 60_000);

	test("search and sources", async () => {
		const { source, host } = connect();
		const report = await source.search("backoff");
		expect(report.hits.length).toBeGreaterThan(0);
		expect(sizes.search!).toBeLessThan(MAX_OUTPUT);
		const sources = await source.sources();
		expect(sources.map((s) => s.adapter)).toEqual(["pi"]);
		host.close();
	});

	test("errors surface as messages, not crashes", async () => {
		const { source, host } = connect();
		await expect(source.session("does-not-exist")).rejects.toThrow(/No conversation found/);
		host.close();
	});
});
