# hstry

Universal AI chat history database. Aggregates conversations from multiple AI tools (ChatGPT, Claude, Gemini, Cursor, Claude Code, etc.) into a single searchable SQLite database.

## Features

- Import chat history from multiple sources via pluggable TypeScript adapters
- Full-text search with separate indexes for natural language and code
- Filter by source, workspace, or time range
- Background service for automatic syncing
- Incremental adapter parsing with cursor-based batching
- Export memories to mmry for semantic search
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

# Search your history
hstry search "how to parse JSON"

# List recent conversations
hstry list --limit 10

# View a specific conversation
hstry show <conversation-id>
```

## Commands

| Command | Description |
|---------|-------------|
| `scan` | Detect chat history sources on the system |
| `sync` | Import conversations from all configured sources |
| `search <query>` | Full-text search across all messages |
| `list` | List conversations with optional filters |
| `show <id>` | Display a conversation with all messages |
| `source add/list/remove` | Manage import sources |
| `adapters` | List, enable, or disable adapters |
| `service start/stop/status` | Control background sync service |
| `stats` | Show database statistics |
| `mmry extract` | Export memories to mmry |

## Search Modes

The search command auto-detects query type:

- **Natural language**: Uses porter stemming for English text
- **Code**: Preserves underscores, dots, and path separators

Force a mode with `--mode natural` or `--mode code`.

## Configuration

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
```

See `examples/config.toml` for all options.

## Supported Sources

### Coding Agents (automatic local storage)

| Adapter | Default Path | Description |
|---------|--------------|-------------|
| `claude-code` | `~/.claude/projects` | Claude Code CLI |
| `codex` | `~/.codex/sessions` | OpenAI Codex CLI |
| `opencode` | `~/.local/share/opencode` | OpenCode |
| `pi` | `~/.pi/agent/sessions` | Pi coding agent |
| `aider` | Project directories | Aider (finds `.aider.chat.history.md`) |

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

Add custom adapters by placing them in `adapter_paths`.

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
just check-all  # Format, lint, and test
just test       # Run tests only
just clippy     # Lint only
```

## License

MIT
