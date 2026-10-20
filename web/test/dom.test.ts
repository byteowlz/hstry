import { describe, expect, test } from "bun:test";
import { renderMarkdown } from "../src/markdown";
import { renderTranscript } from "../src/transcript";
import type { Message } from "../src/types";

const msg = (idx: number, role: Message["role"], content: string, parts?: Message["parts_json"]): Message => ({
	id: String(idx),
	idx,
	role,
	content,
	parts_json: parts ?? null,
});

describe("markdown", () => {
	test("content never becomes markup", () => {
		const host = document.createElement("div");
		host.append(renderMarkdown('<script>alert(1)</script> <img src=x onerror="alert(1)"> **bold** `x`'));
		expect(host.querySelector("script")).toBeNull();
		expect(host.querySelector("img")).toBeNull();
		expect(host.textContent).toContain("<script>alert(1)</script>");
		expect(host.querySelector("strong")?.textContent).toBe("bold");
		expect(host.querySelector("code")?.textContent).toBe("x");
	});
	test("only http(s) links become anchors", () => {
		const host = document.createElement("div");
		host.append(renderMarkdown("[ok](https://example.com) [bad](javascript:alert(1))"));
		const links = [...host.querySelectorAll("a")];
		expect(links.map((a) => a.getAttribute("href"))).toEqual(["https://example.com"]);
		expect(links[0]!.getAttribute("rel")).toContain("noopener");
	});
	test("fenced code keeps its language and text", () => {
		const host = document.createElement("div");
		host.append(renderMarkdown("```rust\nfn main() {}\n```"));
		expect(host.querySelector("figcaption")?.textContent).toBe("rust");
		expect(host.querySelector("pre code")?.textContent).toBe("fn main() {}");
	});
});

describe("transcript", () => {
	const messages: Message[] = [
		msg(0, "user", "find it", [{ type: "text", text: "find it" }]),
		msg(1, "assistant", "", [
			{ type: "thinking", text: "look first" },
			{ type: "tool_call", toolCallId: "t1", name: "Bash", input: { command: "rg retry" } },
		]),
		msg(2, "user", "src/a.rs:1", [{ type: "tool_result", toolCallId: "t1", output: "src/a.rs:1" }]),
		msg(3, "assistant", "Found it.", [{ type: "text", text: "Found it." }]),
	];

	test("tool output attaches to the assistant turn", () => {
		const root = renderTranscript(messages);
		const speakers = [...root.querySelectorAll(".speaker")].map((s) => s.textContent);
		expect(speakers).toEqual(["You", "Assistant"]);
		expect(root.querySelector(".tool-name")?.textContent).toBe("Bash");
		expect(root.querySelector(".tool-arg")?.textContent).toBe("rg retry");
		expect(root.querySelector(".tool-output")?.textContent).toContain("src/a.rs:1");
		expect(root.querySelector("details.thinking")?.hasAttribute("open")).toBe(false);
	});

	test("long output is collapsed and errors are marked", () => {
		const output = Array.from({ length: 20 }, (_, i) => `line ${i}`).join("\n");
		const root = renderTranscript([
			msg(0, "assistant", "", [{ type: "tool_call", name: "ls", input: {} }]),
			msg(1, "tool", output, [{ type: "tool_result", output, isError: true }]),
		]);
		const out = root.querySelector(".tool-output")!;
		expect(out.classList.contains("error")).toBe(true);
		expect(out.querySelector("summary")?.textContent).toContain("20 lines");
	});

	test("a search hit opens collapsed thinking and marks the focused message", () => {
		const root = renderTranscript(messages, { highlight: ["look"], focusIdx: 1 });
		expect(root.querySelector("details.thinking")?.hasAttribute("open")).toBe(true);
		expect(root.querySelector(".message.focused")?.getAttribute("data-idx")).toBe("1");
		expect(root.querySelectorAll("mark").length).toBeGreaterThan(0);
	});
});
