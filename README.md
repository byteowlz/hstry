# hstry

Universal AI chat history database. Aggregates conversations from multiple AI tools (ChatGPT, Claude, Gemini, Cursor, Claude Code, etc.) into a single searchable SQLite database.

## Features

- Import chat history from multiple sources via pluggable TypeScript adapters
- One-off imports from files or directories with auto-detection
- Full-text search with separate indexes for natural language and code
- Filter by source, workspace, role, and local/remote scope
- Remote sync and search over SSH
- Background service for automatic syncing
- Optional terminal UI (`hstry-tui`) for interactive browsing
- Incremental adapter parsing with cursor-based batching
- Export conversations to adapter formats (markdown/json, pi, opencode, codex, claude-code, etc.)
- Deduplicate conversations and export memories to mmry
- JSON output for scripting and MCP integration

## Installation

```bash
cargo install --path crates/hstry-cli
```

Or build from source:

```bash
cargo build --release
```

## Quick Start

```bash
# Scan for supported chat history sources
hstry scan

# Add a source (auto-detects adapter)
hstry source add ~/.codex/sessions

# Sync all sources
hstry sync

# Import a one-off export directory
hstry import ~/Downloads/chatgpt-export

# Search your history
hstry search "how to parse JSON"

# List recent conversations
hstry list --limit 10

# View a specific conversation
hstry show <conversation-id>

# Export a conversation to markdown
hstry export --format markdown --conversations <conversation-id> --output ./conversation.md
```

## Commands

| Command | Description |
|---------|-------------|
| `scan` | Detect chat history sources on the system |
| `sync` | Import conversations from all configured sources (resets cursor if source is empty) |
| `import <path>` | One-off import with auto-detected adapter |
| `search <query>` | Full-text search across all messages |
| `index` | Build or refresh the search index |
| `list` | List conversations with optional filters (workspace uses substring match) |
| `show <id>` | Display a conversation with all messages |
| `export` | Export conversations to markdown/json or adapter format |
| `dedup` | Deduplicate conversations in the database |
| `source add/list/remove` | Manage import sources |
| `adapters list/add/enable/disable` | Manage adapters |
| `adapters repo ...` | Manage adapter repositories (git/archive/local) |
| `remote add/list/remove/test/fetch/sync/status` | Manage remote hosts and sync |
| `service enable/disable/start/run/restart/stop/status` | Control background sync service |
| `config show/path/edit` | Manage configuration |
| `stats` | Show database statistics |
| `mmry extract` | Export memories to mmry |

## Search Modes

The search command auto-detects query type:

- **Natural language**: Uses porter stemming for English text
- **Code**: Preserves underscores, dots, and path separators

Force a mode with `--mode natural` or `--mode code`.

Scope and filters:

- `--scope local|remote|all` (default: local)
- `--remote <name>` to target specific remotes
- `--source`, `--workspace`, `--role` filters
- `--no-tools` to exclude tool calls
- `--dedup` to collapse similar results
- `--include-system` to include system context (AGENTS.md, etc.)

## Configuration

hstry follows XDG Base Directory specifications:

| Directory | Default | Environment Override |
|-----------|---------|---------------------|
| Config | `~/.config/hstry/` | `$XDG_CONFIG_HOME/hstry/` |
| Data | `~/.local/share/hstry/` | `$XDG_DATA_HOME/hstry/` |
| State | `~/.local/state/hstry/` | `$XDG_STATE_HOME/hstry/` |

Default config: `~/.config/hstry/config.toml`

```toml
database = "~/.local/share/hstry/hstry.db"
adapter_paths = ["~/.config/hstry/adapters"]
js_runtime = "auto"  # bun, deno, or node

[[adapters]]
name = "codex"
enabled = true

[service]
enabled = false
poll_interval_secs = 30
search_api = true
# search_port = 3000

[search]
# index_path = "~/.local/share/hstry/search"
index_batch_size = 500
```

See `examples/config.toml` for all options. Use `hstry config show/path/edit` for config management.

## Service + API

`hstry service` runs a local daemon that keeps the search index warm and exposes a
local-only gRPC search endpoint. The CLI prefers the service when it is running.
Use `hstry service enable/disable/start/run/restart/stop/status` to manage it.

The optional `hstry-api` binary serves a local HTTP API (default `http://127.0.0.1:3000`)
for external integrations (e.g., Octo).

Override service usage with `HSTRY_NO_SERVICE=1`. Override the API URL with
`HSTRY_API_URL` or disable API usage with `HSTRY_NO_API=1`.

## Remote Sync

hstry can sync and search remote databases over SSH. Remotes require `hstry` to
be installed on the host.

```bash
# Add a remote host
hstry remote add laptop user@laptop

# Verify connectivity
hstry remote test laptop

# Fetch the remote database into the local cache
hstry remote fetch --remote laptop

# Search only remote results
hstry search "auth error" --scope remote --remote laptop

# Sync (merge) remote history into the local database
hstry remote sync --remote laptop --direction pull
```

## Terminal UI

Use the optional `hstry-tui` binary for an interactive, three-pane browser.

```bash
cargo install --path crates/hstry-tui
hstry-tui
```

## Supported Sources

### Local Agents & Apps (automatic local storage)

| Adapter | Default Path | Description |
|---------|--------------|-------------|
| `claude-code` | `~/.claude/projects` | Claude Code CLI |
| `codex` | `~/.codex/sessions` | OpenAI Codex CLI |
| `cursor` | `Cursor workspaceStorage` (platform-specific) | Cursor (state.vscdb) |
| `opencode` | `~/.local/share/opencode` | OpenCode |
| `pi` | `~/.pi/agent/sessions` | Pi coding agent |
| `aider` | Project directories | Aider (finds `.aider.chat.history.md`) |
| `goose` | `~/.local/share/goose/sessions` | Goose (SQLite/JSONL) |
| `jan` | `~/jan/threads` | Jan.ai |
| `lmstudio` | `~/.cache/lm-studio/conversations` | LM Studio |
| `openwebui` | `~/.open-webui/data` (or `/app/backend/data`) | Open WebUI |

### Web Exports (manual download)

| Adapter | Source | Export Location |
|---------|--------|-----------------|
| `chatgpt` | ChatGPT | Settings > Data controls > Export |
| `claude-web` | Claude.ai | Settings > Export data |
| `gemini` | Gemini | google.com/takeout > Gemini Apps |

Point these adapters at the extracted export directory (e.g., `~/Downloads/chatgpt-export`).

## Adapters

Adapters are TypeScript modules that parse chat history from specific tools. Each adapter implements:

- `detect(path)` - Check if a path contains valid data
- `parse(path, options)` - Extract conversations and messages

Add custom adapters by placing them in `adapter_paths`, or manage repositories with:

```bash
hstry adapters repo add-git community https://example.com/adapters.git
hstry adapters update
```

## Workspace Structure

```
crates/
  hstry-core/     # Database, config, models
  hstry-runtime/  # TypeScript adapter execution
  hstry-cli/      # Command-line interface
  hstry-tui/      # Terminal UI (ratatui)
  hstry-mcp/      # MCP server
  hstry-api/      # HTTP API (axum)
```

## Development

```bash
just check-all       # Format, lint, and test
just test            # Run tests only
just clippy          # Lint only
just update-adapters # Copy latest adapters to ~/.config/hstry/adapters
```

## Attribution

This project is inspired by and references ideas from **cross-agent-session-search (cass)** by Jeffrey Emanuel. Source: https://github.com/Dicklesworthstone/coding_agent_session_search (MIT License).

## License

MIT
