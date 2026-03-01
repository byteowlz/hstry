# Issues

## Open

### [trx-5mf1] Add DeleteConversation gRPC RPC to remove conversations and their messages (P1, feature)

### [trx-6yb5] Add UpdateConversation gRPC RPC for partial metadata updates (title, workspace, model, provider, metadata) (P1, feature)

### [trx-tatn] Add message event read API + summary cache (P1, task)
Expose ReadService.GetMessageEvents for incremental history reads and add conversation_summary_cache to speed ListConversations. Update message ingestion to keep cache in sync.

### [trx-vg8v] Schema: store host_id/hostname on sessions + migrations (P1, task)

### [trx-rs72] Remote history sync over SSH (hstry) (P1, epic)

### [trx-en2q] Canonical part-based chat schema for Octo + hstry (P1, epic)

### [trx-smd7] Add tests for parallel sync correctness (P2, task)

### [trx-sq9n] Add sync performance instrumentation and per-source timings (P2, task)

### [trx-g91t] Update hstry adapters to populate sender field (P2, task)
Update hstry adapters to populate the new sender field on messages.

The Sender type is now available in hstry-core::parts (see trx-7fh3, completed).

Adapter updates needed:
...


### [trx-t8dh] Pi adapter scan all sessions + service port fallback (P2, task)

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

- [trx-kdcw] Pin adapters to hstry version and enforce manifest compatibility (closed 2026-03-01)
- [trx-0jah] Document sync parallelism and performance changes (closed 2026-02-27)
- [trx-4x3e] Optimize FTS integrity-check overhead during sync (closed 2026-02-27)
- [trx-ekqp] Parallelize sync across sources with bounded concurrency (closed 2026-02-27)
- [trx-01fn] Update octo-protocol to re-export Sender from hstry-core instead of defining its own (closed 2026-02-05)
- [trx-7fh3] Add Sender type to hstry-core and update Message model (closed 2026-02-05)
- [trx-z2zy] Migration 006: Add sender_json and provider columns to messages table (closed 2026-02-05)
- [trx-qtb6] Add gRPC write API for external writers (Octo) (closed 2026-02-01)
- [trx-ex6v] Add typed Part enum to hstry-core (closed 2026-02-01)
- [trx-cbkm] Use Unix domain socket for gRPC service security (closed 2026-02-01)
- [trx-0r08] Adjust clippy print rules for CLI crates (closed 2026-01-28)
- [trx-ca4j] Core: remote search/show helpers + host field (closed 2026-01-28)
- [trx-cmew] CLI/TUI: service-first search + remote SSH search (closed 2026-01-28)
- [trx-pbj8] Service: add gRPC search endpoint for warm queries (closed 2026-01-28)
- [trx-fddq] Docs: add API search usage notes (closed 2026-01-28)
- [trx-t23b] CLI: use hstry-api for search when available (closed 2026-01-28)
- [trx-whve] API: serve Tantivy-backed search endpoint (closed 2026-01-28)
- [trx-4w8c] Docs: document search index config and command (closed 2026-01-28)
- [trx-h51w] CLI: add hstry index command for rebuild (closed 2026-01-28)
- [trx-stz0] Search: make Tantivy default and add background indexing (closed 2026-01-28)
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
