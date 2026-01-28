# Issues

## Open

### [trx-vg8v] Schema: store host_id/hostname on sessions + migrations (P1, task)

### [trx-rs72] Remote history sync over SSH (hstry) (P1, epic)

### [trx-en2q] Canonical part-based chat schema for Octo + hstry (P1, epic)

### [trx-fycz] Conflict policy: dedupe by message_id + session_id, prefer newest updated_at (P2, task)

### [trx-g7af] Integration with mmry for memory extraction (P2, feature)
Notes: added hstry CLI mmry extraction command that maps messages to mmry JSON memories (with hstry metadata) and invokes the mmry add stdin flow with store/config options.

### [trx-j3xk] Implement source remove command (P2, task)

### [trx-dy7w] Add ChatGPT export adapter (P2, feature)

### [trx-qtxm] Add Claude Code adapter (P2, feature)

### [trx-ctd3] Docs: update example config and usage for remote fetch/sync (P3, task)

### [trx-bj82] Tests: config parsing + remote fetch/sync happy paths (P3, task)

### [trx-k4ts] Add MCP server for LLM access to history (P3, feature)
Notes: added missing MCP deps, wired hstry_core::Config loading, updated MCP tool output to return service config, and aligned API deps/config loading so the workspace builds.

### [trx-wab4] Add TUI for browsing history (P3, feature)
Notes: added TUI deps, wired hstry_core::Config loading, and show database path in the details pane.

### [trx-2bmw] Add Aider adapter (P3, feature)

### [trx-s13m] Add Cursor adapter (P3, feature)

### [trx-4h8s] Add Gemini adapter (P3, feature)

## Closed

- [trx-gj5r] CLI: add hstry remote fetch/sync subcommands with filters (closed 2026-01-28)
- [trx-vd8h] Remote plumbing: SSH transport + temp files + atomic replace (closed 2026-01-28)
- [trx-bb8t] Remote sync: bidirectional merge between local and remote DBs (closed 2026-01-28)
- [trx-j8gj] Remote fetch: SSH pull of remote hstry DB into local cache (closed 2026-01-28)
- [trx-8g87] Config: define remote hosts (SSH) and optional paths (closed 2026-01-28)
- [trx-x2k5] Reduce noisy search projection from tool/file outputs (closed 2026-01-27)
- [trx-z5x5] Add readable_id adj-verb-noun IDs to conversations (closed 2026-01-27)
- [trx-qxba] Fix hstry-tui char boundary panic on box drawing (closed 2026-01-27)
- [trx-36t2] Test OpenCode adapter with real session data (closed 2026-01-17)
- [trx-zp7t] Fix adapter path discovery in dev mode (closed 2026-01-17)
