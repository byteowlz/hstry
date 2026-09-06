# Evidence-oriented search

Search returns evidence, not conversation dumps. Examples:

```sh
hstry search 'what password did we set for immich' --json
hstry search 'what did we decide about bb' --json
hstry search 'which session did we discuss gpui in' --json
hstry search 'admin@fixture.local' --mode exact --json
hstry search 'admin@fixture\.local' --mode needle --json
```

## Modes

- `auto`: identifier-shaped queries (paths, addresses, camelCase, host labels,
  assignments, error fragments, quoted strings) try literal matching first.
  Natural questions use FTS/BM25 with question filler removed.
- `exact`: **case-sensitive literal substring** over stored message text. No
  regex interpretation, stemming, prefix operators, or fallbacks.
- `needle`: explicit literal-first routing, including for multiword queries.
- `regex`: explicit Rust regex syntax, case-sensitive unless `(?i)` is supplied.
  Invalid expressions are errors. Backreferences/lookaround are unsupported.
- `natural`: explicit Porter-tokenized FTS; `code`: identifier-preserving FTS.
  Both keep explicit FTS input semantics (quoted tokens, trailing `*` prefixes).
- `recent`, or an empty query: recent conversation evidence without scoring.

In auto/needle mode, a failed literal pattern containing regex syntax is retried
as regex, then FTS. Every attempted mode is reported in `attempts`; an invalid
fallback regex is reported in `warnings`. Explicit exact mode never does this.

For natural recall, conversation context matters: the topic and the answer may
be in different messages. Search expands to matching passages across the top
five candidate conversations, or uses an OR-term discovery pass when no strict
candidate exists. It ranks conversation term coverage, then message evidence.
Query-related named field values receive an evidence boost, and snippets choose
term-dense passages rather than the first mention. This uses no embeddings or
LLM. Ranking considers a bounded 128-message candidate window; warnings disclose
that limit. Use a narrower query or exact mode for exhaustive literal discovery.
There is no claim of perfect semantic recall.

Source, workspace, date, role (including multiple roles), model, harness and tag
filters apply before matching and pagination. CLI `--include-system` includes
system-role messages; tool output is searched by default. `--no-tools` is an
explicit exclusion. Existing `--dedup` and `--compact` remain available.

## JSON / API / MCP contract

CLI JSON, HTTP `/search`, gRPC search, and MCP `search` share a projection:

- Default **3,000 characters total**, including JSON keys/escaping and the CLI
  newline; default snippet at most **300 characters**.
- `result.hits`: conversation UUID, readable ID, message index, role, title,
  timestamp, one evidence snippet, raw-text character match position, provenance.
- `truncated`, `omitted_hits`, `has_more`, `next_offset`, `provenance_truncated`:
  presentation/pagination limits are explicit. Metadata may be elided to preserve
  evidence. Tiny budgets can omit all hits; that is not a zero-match assertion.
- `attempts`, `warnings`, `filters`, `scope`, `stores`, `available_remotes`, and
  `absence_is_global: false` are present even when no evidence matches.

CLI: `--max-chars 3000 --snippet-chars 300 --offset 0`. HTTP/MCP use `max_chars`,
`snippet_chars`, and `offset`. Supported total budgets are 512–1,000,000 chars.
Snippets in the compact projection never exceed 300 chars even if a larger
snippet option is provided. Title/readable-ID display fields are capped at 120.

`--raw` (HTTP/MCP `raw=true`, gRPC `raw=true`) is an explicit, **unbudgeted**
lossless report with full message content. It is intended for internal consumers
and diagnostics, not routine agent recall. Stored messages are never shortened.

This changes JSON search `result` from an array to an object containing `hits`.
Upgrade service/API/remote binaries together. Older service responses are rejected
with an upgrade instruction rather than silently losing modes or filters.

Drill into a result in one call:

```sh
hstry show <conversation_id> --message-idx <message_idx> --json
```

## Bounded reads

`show` is bounded by default; `show --full` explicitly requests its legacy full
transcript. Use `read` for continuation controls:

```sh
hstry read CONVERSATION --message-idx 12 --before 2 --after 2 --json
hstry read CONVERSATION --message-id UUID --expand-interactions --json
hstry read CONVERSATION --message-idx 12 --field content --offset-chars 400 --json
```

CLI `read --input -` and HTTP `POST /read` accept
`{"id":"UUID","options":{"message_idx":12,"max_chars":3000}}`.
HTTP additionally accepts `remote`; CLI uses `--remote NAME`. MCP `expand` exposes
the same options, with `page_offset` for record paging and `offset` for character
continuation. All use the same core reader and `result.records` envelope.

Reads default to 3,000 serialized characters (valid range 1,000–1,000,000), 50
field records, and no context. Anchored pages prioritize the anchor over neighbors;
unanchored pages are chronological. Direct tool expansion follows nonempty
canonical tool-call IDs in this conversation, not recursive ancestry. Fields are
`content`, `parts/N/input`, and `parts/N/output`; null/missing payloads are omitted.

`records[].next_offset_chars` continues one field using its message and field
anchor. `result.next_offset` advances field-record pages; it does **not** recover
omitted field text. Offsets count Unicode characters, not bytes or JSONL lines.
Pass `--conversation-version` (JSON `options.version`) to reject stale cursors.
Each read uses one SQLite snapshot; output budgets do not truncate the archive.
The implementation still scans message headers and materializes selected fields.

SSH performs the bounded read on the source and includes the coordinator's machine
label inside that budget. Incompatible, oversized, or failed peer responses are
errors, never a fallback to fetching the whole remote transcript. Legacy full
conversation endpoints remain explicit alternatives, not evidence-read fallbacks.

## Agent guidance and diagnostics

`hstry skill install|status|update --target shared|claude` manages the bundled
`hstry-search` skill. `--path` selects an explicit skill directory. Updates preserve
locally edited/unmanaged copies unless `update --force` is explicitly requested.
Search warns on differing installed copies without changing them.

`search --trace-file NEW_FILE` writes opt-in timing, attempt, count, and rank/score
metadata, excluding queries, IDs, paths, and message text. Existing files are not
overwritten. The shared benchmark also records top-three session diversity.

Resume JSON is a non-executing plan with `argv`, a new converted target identity,
provenance, and explicit native-verification status. Conversion writes refuse
unsafe paths, symlinks, duplicates, and overwrites. Launching an unverified
conversion requires `--allow-unverified`. Export support is not native continuation
verification; the external `bench-resume` matrix reports those stages separately.

## Completeness

Local searches cover the **local database snapshot**, including already-synced
sources. Provenance records source, known machine, last sync, and `full`, `partial`
or `unknown` completeness. Missing evidence about completeness stays `unknown`;
a recent timestamp alone never proves completeness.

New remote database merges record copied snapshot time and source message count.
Hits report whether that snapshot is fully represented locally, explicitly marked
`basis: copied_snapshot_only`. Even a full copied snapshot is not proof that the
remote source itself is complete or current. Older copies retain unknown status.

```sh
hstry search 'admin@fixture.local' --remote workstation --json
hstry search 'registry placement' --scope all --json
```

`--remote NAME` implies remote search when scope is otherwise local. Unknown,
disabled or unreachable remotes fail explicitly; they do not become empty results.
Remote search runs against the source machine's hstry database, not unsynced raw
agent files. Sync there first if necessary. Paginate local searches or one named
remote separately; multi-store discovery intentionally does not offer a global
offset that could skip unseen evidence. The RPC search endpoint itself covers
its server's local snapshot; CLI/MCP handle named remote dispatch.

## Tests and benchmarks

Contract regression tests run in `cargo test -p hstry-core -p hstry-cli -p hstry-api
-p hstry-mcp`. The benchmark corpus, runner, baselines and results live exclusively
in `~/byteowlz/bench`:

```sh
cargo build -p hstry-cli
cd ../bench
HSTRY_BIN=../hstry/target/debug/hstry just bench-hstry
HSTRY_BIN=../hstry/target/debug/hstry just bench-hstry-internal
```

The external benchmark grades content independently of retrieval, verifies actual
hstry conversation/message anchors, and records output characters and latency.
Internal summaries contain metrics only—no queries, private identifiers or values.
