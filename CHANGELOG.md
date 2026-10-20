# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.28] - 2026-10-20

### Added

- Web UI: find a session as you type (title, repo, agent, first prompt), search inside every message with hits grouped per session, read the transcript with thinking, tool calls and tool output shown as such, and act on it: copy the command to continue it in an agent, copy the transcript, id or path, export Markdown or JSON. `hstry ui` opens it (reusing a running API or serving it until Ctrl-C); `hstry-api` serves it at `/ui` with a strict CSP. Styled only through the byteowlz/Oqto design tokens, light and dark, with a phone layout.
- Oqto App (`oqto/hstry.oqtoapp`): the same UI inside Oqto, read-only, fed by pinned operations that call the hstry CLI and stay within Oqto's operation output limits. Install with `just oqto-app-install <workdir>`.
- `hstry-api` read endpoints `GET /sessions`, `/sessions/{id}`, `/sources`, `/agents`, returning the same JSON as `hstry list`, `show --full` and `source list`.
- `hstry pick`: fuzzy session finder with copy (default), `--print`, `--id-only`, `--path`, `--open <agent>`, `--export <format>` and `--no-pick` for scripts.
- Pi Durable adapter and qualified Pi <-> Pi Durable conversion with explicit fidelity reports.
- Adapter conformance suite: every adapter ships sample sessions and a reviewed snapshot; CI gates all of them. A weekly CI job checks the adapters against current public Hugging Face agent traces.

### Changed

- TUI rework around finding and acting: one live query bar (Tab switches to full-text search), two-line rows that keep titles readable at 80 columns, a transcript built from message parts, a visible action bar (open in agent, copy transcript/id/path, export, delete), light-terminal detection, and a single-pane layout below 100 columns. Search hits are grouped per session instead of one row per hit.
- Release archives now include `hstry-api`, so `hstry ui` and `hstry api` work from an installed release. Oqto deployments that manage hstry must list `hstry-api` in `[binary_owners]`.
- Codex sessions without a thread title are titled by their first prompt instead of "repo - date - prompt".

### Fixed

Adapters, many of which silently imported nothing or the wrong content for current tool versions (reimport refreshes affected sessions in place):

- claude-code: `/rename` titles, redacted thinking, model fallback and attachment records; honors `CLAUDE_CONFIG_DIR`.
- codex: reasoning summaries and agent messages indexed; `.jsonl.zst` rollouts, `CODEX_HOME` and the newest `state_N.sqlite` supported.
- opencode 2.x (`session_v2`) and Qwen Code (`~/.qwen/projects/*.jsonl`) sessions are read; Hermes reads its canonical `state.db` and `hermes sessions export` files; Gemini reads AI Studio prompt files and Takeout `MyActivity.json`.
- chatgpt, chatgpt-web and openwebui keep only the visible branch instead of merging regenerated answers.
- aider, amp, antigravity, fx, github-copilot, omp and openwebui parsed nothing from real files; cline, cursor, droid, gemini-cli, goose, kiro, mastracode, claude-web and others lost tool calls, results, reasoning or timestamps.
- pi: one malformed session no longer aborts the whole import; system prompt sections, extension messages and usage entries are kept.
- Windows: the CLI no longer overflows the main-thread stack.

## [0.5.27] - 2026-10-06

### Changed

- Search falls back to the local database snapshot when the background service is unavailable.
- Installed adapters are stamped with the hstry version at install time.

## [0.5.26] - 2026-09-29

### Added

- `hstry api enable|disable|start|stop|restart|status|run` to manage the hstry-api HTTP ingest server, mirroring `hstry service`. `start` runs hstry-api as a detached background process (pid file + log under XDG state); no launchd/plist involved.
- `[api]` config section (`enabled`/`port`/`token`). The ingest token may be a literal, an `env:VAR` reference, or a `kyz:<secret>` secrets-store reference via `hstry_core::config::resolve_secret`, so it never needs to live as plaintext in config or the process args.

## [0.5.25] - 2026-09-06

### Added

- Exact/needle/regex recall with bounded evidence, stable anchors, and explicit scope/completeness reporting.
- Shared bounded reads across CLI, HTTP, MCP, and SSH, including field continuations and linked tool interactions.
- Installable retrieval skill and opt-in content-free search diagnostics.

### Changed

- Search JSON now returns a report object; upgrade service/API/remote binaries together. `show` is bounded by default; use `--full` for legacy output.
- Resume uses structured arguments, fresh conversion identities, origin sidecars, and no-overwrite placement. JSON returns a non-executing plan; converted launches require `--allow-unverified`.

### Fixed

- TUI compilation with remote search reports and OpenCode conversion layout/session IDs.

## [0.5.16] - 2026-04-26

### Changed

- **TUI left pane**: Group source entries by adapter instead of individual source ID. One entry per adapter (e.g., one `pi`, one `claude-code`) even when multiple source IDs exist for the same adapter.
- **`hstry list`**: Automatically deduplicate conversations across sources by `external_id`/`readable_id`/`platform_id`. Canonical source IDs (e.g. `pi`) take precedence over `import-pi` style IDs.
- **`hstry import`**: Reuse existing source when importing to a path that already has one, or default to canonical adapter ID instead of `import-<adapter>`. Prevents duplicate source creation.
- **Startup performance**: Replace `COUNT(*)` scans on FTS5 tables with `SELECT 1 LIMIT 1` probes during DB init. Eliminates multi-second startup latency on large databases.

## [0.5.15] - 2026-01-28

### Added

- (previous releases)
