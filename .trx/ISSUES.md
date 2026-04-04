# Issues

## Open

### [trx-8em1] hstry must not crash-loop on adapter version mismatch -- gracefully degrade or auto-update (P0, bug)
When hstry binary is updated (e.g. 0.5.5 -> 0.5.7) but adapters are not, hstry exits immediately with:
'Adapter version mismatch (expected hstry 0.5.7, found 0.5.5). Run hstry adapters update.'

With systemd Restart=always, this creates a crash-loop (1100+ restarts observed in production, 3 users affected on octo-azure, 21+ messages permanently lost from Pi JSONL).

...


### [trx-702h] Add parent_conversation_id column for session tree tracking (P1, feature)
hstry has no first-class parent/child relationship between conversations. Pi tracks parentSession in JSONL headers and the adapter puts it into the metadata JSON blob, but there is no parent_conversation_id column on the conversations table. This means you cannot query 'show all child sessions of X', 'show the full session tree', or navigate fork lineage. Needed for: Oqto thread dispatch (worker sessions link to orchestrator), session tree rendering in sidebar, unified search across session trees. Implementation: add parent_conversation_id TEXT column (nullable, self-referencing FK), migration 012, index on parent_conversation_id, update Pi adapter to populate it from header.parentSession, add list_children(conversation_id) and get_ancestors(conversation_id) to Database.

### [trx-z42c.9] Regression suite for incremental sync correctness and missed-event recovery (P1, task)
Add tests for watermark advancement, event miss recovery via safety audit, fingerprint invalidation, and idempotent outbox indexing.

### [trx-z42c.6] Dedicated index worker consuming outbox jobs (P1, task)
Build index worker loop with bounded batch size, retries, poison handling, and exactly-once semantics. Remove hot-path DB rescans for indexing.

### [trx-z42c.5] Indexer outbox table + enqueue on message upsert (P1, task)
Introduce index_jobs outbox with idempotency key (message_id+version). Enqueue jobs transactionally alongside message writes.

### [trx-z42c.4] File source fingerprint cache to skip unchanged artifact parsing (P1, task)
Store fast fingerprints (path, inode, size, mtime_ns) and optional content hash-on-change. Parse only changed files and update cache atomically.

### [trx-z42c.3] Incremental state model: persist watermarks/checkpoints per source (P1, task)
Add durable source sync state (cursor/change-token/fsevent checkpoint, last_success_at, next_sync_at). Include migration and compatibility with existing source config.

### [trx-z42c.2] Event pipeline: watcher debounce, coalescing, and source-targeted sync (P1, task)
Refine file/API event ingestion: debounce, coalesce, map changed paths to affected sources, and sync only impacted sources.

### [trx-z42c.1] Scheduler v2: per-source adaptive cadence + remove global 30s full sync (P1, task)
Implement per-source next_sync_at scheduling with adaptive backoff/jitter. Replace global tick-driven sync_all as primary path. Keep low-frequency safety audit timer configurable.

### [trx-z42c] SOTA sync architecture: event-driven incremental ingestion + outbox indexing (P1, epic)
Replace frequent full sync loops with an event-driven incremental pipeline to eliminate idle CPU spikes and improve scalability.

Problem:
- Service currently performs frequent broad sync checks across many sources, causing unnecessary parse/diff work when no data changed.
- Search indexing is incremental, but upstream sync scheduling still burns CPU/IO.
...


### [trx-h5q5] Pi adapter stores conversations with empty workspace field (P1, bug)
The Pi adapter sync sometimes creates conversations with empty workspace field despite valid cwd in JSONL header. Also decodeWorkspaceFromPath() at line 880 of adapter.ts is lossy - replaces ALL hyphens with slashes (content-creation becomes content/creation). Fixed 34+ sessions on octo-azure with direct SQL UPDATE.

### [trx-6124] Pi adapter sync broken: HSTRY_REQUEST not provided (P1, bug)
The pi adapter always fails with 'HSTRY_REQUEST not provided' during hstry sync. This means sessions that weren't persisted via gRPC during the live session are never recoverable. Observed on octo-azure for user oqto_usr_wismut. The adapter.ts at /usr/local/share/hstry/adapters/pi/ expects HSTRY_REQUEST env var but hstry CLI doesn't set it.

### [trx-5mf1] Add DeleteConversation gRPC RPC to remove conversations and their messages (P1, feature)

### [trx-6yb5] Add UpdateConversation gRPC RPC for partial metadata updates (title, workspace, model, provider, metadata) (P1, feature)

### [trx-tatn] Add message event read API + summary cache (P1, task)
Expose ReadService.GetMessageEvents for incremental history reads and add conversation_summary_cache to speed ListConversations. Update message ingestion to keep cache in sync.

### [trx-vg8v] Schema: store host_id/hostname on sessions + migrations (P1, task)

### [trx-rs72] Remote history sync over SSH (hstry) (P1, epic)

### [trx-en2q] Canonical part-based chat schema for Octo + hstry (P1, epic)

### [trx-5fqx] Standardize search JSON output for cross-tool integration (P2, feature)
For unified search across hstry/mmry/trx, search results need a common envelope format: { source, source_id, id, title, snippet, score, created_at, tags, metadata }. Ensure hstry search --json output matches this schema so agntz can merge results from all three tools.

### [trx-j4z6] Expose conversation tags in search and CLI (P2, feature)
The schema already has tags and conversation_tags tables (migration 001) but they are never populated or queryable through the API. Add: tag management (add/remove tags on conversations), tag filter on search and list_conversations, CLI commands for tagging. Tags are the cross-tool connector for unified search across hstry/mmry/trx.

### [trx-bwvy] Add role filter to search() (P2, feature)
Cannot restrict search to 'only user messages' or 'only assistant messages'. Add optional role field to SearchOptions, apply as WHERE m.role = ? clause. Useful for finding what users asked vs what agents answered.

### [trx-tj2t] Add date range filters (after/before) to search() (P2, feature)
list_conversations supports after/before filters but search() does not. You cannot say 'search for X from last 2 weeks'. Add after/before fields to SearchOptions and apply as WHERE clauses on m.created_at or c.created_at in both FTS and Tantivy search paths.

### [trx-z42c.8] Observability: per-source metrics, queue depth, and structured sync logs (P2, task)
Expose sync timings, changed/unchanged counters, error/backoff state, index queue lag/depth, and skipped-source reasons.

### [trx-z42c.7] Resource controls: bounded concurrency, CPU/time budgets, and QoS (P2, task)
Add global/per-source worker limits and budgeted execution to prevent service CPU monopolization on active machines.

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

### [trx-hbfm] Add model and harness filters to search() (P3, feature)
Cannot filter search by model (e.g. 'all Claude conversations') or harness (e.g. 'all Pi sessions'). Add optional model and harness fields to SearchOptions, apply as WHERE clauses joining through conversations table.

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

- [trx-krtd] hstry-core fabricated fake readable_ids instead of storing NULL (closed 2026-03-26)
- [trx-dyty] Pi adapter was not extracting readable_id from session title brackets (closed 2026-03-26)
- [trx-2wpe] Adapter protocol: add optional conversation version/message_count fields for import/export parity (closed 2026-03-19)
- [trx-bw1g] Add monotonic conversation version for deterministic message sync (closed 2026-03-19)
- [trx-7ghs] Add message_count and version fields to GetConversation and ListConversations responses (closed 2026-03-13)
- [trx-5ph6] append_messages should be idempotent by message idx -- prevent duplicate inserts (closed 2026-03-13)
- [trx-c077] Add monotonic version counter to conversations for sync protocol (closed 2026-03-13)
- [trx-hhnc] Add web login/sync for ChatGPT via Playwright (closed 2026-03-01)
- [trx-1qkv] Add quickstart command for auto scan+ingest (closed 2026-03-01)
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
