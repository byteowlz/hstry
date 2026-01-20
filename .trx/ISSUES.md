# Issues

## Open

### [trx-g7af] Integration with mmry for memory extraction (P2, feature)
Notes: added hstry CLI mmry extraction command that maps messages to mmry JSON memories (with hstry metadata) and invokes the mmry add stdin flow with store/config options.

### [trx-j3xk] Implement source remove command (P2, task)

### [trx-dy7w] Add ChatGPT export adapter (P2, feature)

### [trx-qtxm] Add Claude Code adapter (P2, feature)

### [trx-k4ts] Add MCP server for LLM access to history (P3, feature)
Notes: added missing MCP deps, wired hstry_core::Config loading, updated MCP tool output to return service config, and aligned API deps/config loading so the workspace builds.

### [trx-wab4] Add TUI for browsing history (P3, feature)
Notes: added TUI deps, wired hstry_core::Config loading, and show database path in the details pane.

### [trx-2bmw] Add Aider adapter (P3, feature)

### [trx-s13m] Add Cursor adapter (P3, feature)

### [trx-4h8s] Add Gemini adapter (P3, feature)

## Closed

- [trx-36t2] Test OpenCode adapter with real session data (closed 2026-01-17)
- [trx-zp7t] Fix adapter path discovery in dev mode (closed 2026-01-17)
