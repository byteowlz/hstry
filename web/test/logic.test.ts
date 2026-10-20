import { describe, expect, test } from "bun:test";
import { matchQuery } from "../src/fuzzy";
import { excerpt, relativeTime, resumeCommand, slug, toolHint, transcriptMarkdown } from "../src/format";
import type { Message, SessionPreview } from "../src/types";

// Same cases as crates/hstry-tui/src/fuzzy.rs so both UIs rank alike.
describe("matchQuery", () => {
	test("all terms must match", () => {
		expect(matchQuery("auth hstry", "Refactor auth middleware", "work/hstry claude-code")).not.toBeNull();
		expect(matchQuery("auth billing", "Refactor auth middleware", "work/hstry claude-code")).toBeNull();
	});
	test("contiguous beats scattered", () => {
		const contiguous = matchQuery("auth", "auth middleware", "")!;
		const scattered = matchQuery("rfctr", "refactor", "")!;
		expect(contiguous.score).toBeGreaterThan(scattered.score);
	});
	test("wide scatter is not a match", () => {
		expect(matchQuery("auth", "Add --json output to the list command", "")).toBeNull();
		expect(matchQuery("rfctr", "refactor the loader", "")).not.toBeNull();
	});
	test("metadata scatter stays in one word", () => {
		expect(matchQuery("auth", "Fix flaky retry test", "work/hstry claude-code The retry test")).toBeNull();
		expect(matchQuery("dstudio", "x", "work/design-studio")).not.toBeNull();
	});
	test("title hits are highlighted", () => {
		expect(matchQuery("retry", "Fix flaky retry test", "")!.titlePositions).toEqual([10, 11, 12, 13, 14]);
		expect(matchQuery("hstry", "Fix flaky retry test", "work/hstry")!.titlePositions).toEqual([]);
	});
});

describe("format", () => {
	test("excerpt is word aligned and drops fences", () => {
		const out = excerpt("Done.\n\n```rust\nfn backoff() {}\n```", "backoff", 60);
		expect(out).not.toContain("```");
		expect(out).toContain("fn backoff");
		const long = excerpt("The snippet() call re-tokenized huge tool blobs. Selecting the evidence window first", "evidence", 40);
		expect(long.startsWith("…")).toBe(true);
		expect(long).not.toMatch(/^…electing/);
	});
	test("relative time", () => {
		const now = new Date("2026-10-20T12:00:00Z");
		expect(relativeTime(new Date("2026-10-20T11:59:30Z"), now)).toBe("now");
		expect(relativeTime(new Date("2026-10-20T09:00:00Z"), now)).toBe("3h ago");
		expect(relativeTime(new Date("2024-01-02T00:00:00Z"), now)).toBe("2024-01-02");
	});
	test("slug and resume command", () => {
		expect(slug("Fix flaky retry test: in the sync loop!")).toBe("fix-flaky-retry-test-in-the-sync-loop");
		expect(slug("¿¿??")).toBe("session");
		expect(resumeCommand("abc", "pi")).toBe("hstry resume abc --agent pi");
	});
	test("tool hint picks the recognizable argument", () => {
		expect(toolHint({ command: ["cargo", "test"] })).toBe("cargo test");
		expect(toolHint({ file_path: "/a/b.rs", other: 1 })).toBe("/a/b.rs");
	});
	test("transcript markdown keeps tool calls and outputs", () => {
		const session = { id: "s1", source_id: "pi", title: "T", created_at: "2026-10-20T12:00:00Z" } as SessionPreview;
		const messages: Message[] = [
			{ id: "1", idx: 0, role: "user", content: "hi" },
			{ id: "2", idx: 1, role: "assistant", content: "", parts_json: [{ type: "tool_call", name: "bash", input: { command: "ls" } }] },
			{ id: "3", idx: 2, role: "tool", content: "a.txt", parts_json: [{ type: "tool_result", output: "a.txt", isError: true }] },
		];
		const md = transcriptMarkdown(session, messages, "pi");
		expect(md).toContain("# T");
		expect(md).toContain("**bash** `ls`");
		expect(md).toContain("Error:\n```\na.txt\n```");
	});
});
