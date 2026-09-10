# hstry

One searchable SQLite database for your AI chat history. Imports conversations from coding agents (Claude Code, Codex, Pi, Cursor, Aider, ...) and web apps (ChatGPT, Claude, Gemini, Perplexity), with full-text search, export, and cross-agent session resume.

## Installation

```bash
# Homebrew (macOS and Linux)
brew tap byteowlz/tap
brew install hstry

# Arch Linux
yay -S hstry

# Cargo
cargo install --path crates/hstry-cli

# Optional binaries
cargo install --path .        # all binaries (CLI, TUI, MCP, API)
cargo install --path crates/hstry-tui
```

Pre-built binaries for Linux (x86_64/ARM64) and macOS (Intel/Apple Silicon) are on the [Releases](https://github.com/byteowlz/hstry/releases) page.

## Quick start

```bash
hstry quickstart                      # scan known paths, add sources, sync

hstry source add ~/.codex/sessions    # add a source (adapter auto-detected)
hstry sync --parallel 2               # import from all sources
hstry import ~/Downloads/chatgpt-export  # one-off import

hstry search "how to parse JSON"      # full-text search
hstry list --limit 10                 # recent conversations
hstry show <conversation-id>

hstry resume --search "JSON parser" --agent pi   # reopen a session in any agent
```

## Commands

| Command | Description |
|---------|-------------|
| `quickstart` | Scan known paths, add sources, sync |
| `sync` | Import from all configured sources in parallel |
| `import <path>` | One-off import with auto-detected adapter |
| `scan` | Detect chat history sources on this system |
| `search <query>` | Full-text search across messages |
| `index` | Build or refresh the search index |
| `list` | List conversations with filters |
| `show <id>` / `read` / `peek` | Display a conversation (bounded pages) |
| `remove <id>` | Remove a conversation and related data |
| `export` | Export to markdown/json or an adapter format |
| `resume` | Resume a session in a coding agent |
| `dedup` | Deduplicate conversations |
| `reseed` / `verify` | Rebuild a source from scratch / check DB vs disk |
| `source add/list/remove` | Manage import sources |
| `adapters list/add/repo/update` | Manage adapters and adapter repositories |
| `remote add/test/fetch/sync` | Sync and search remote hosts over SSH |
| `web install/login/sync/status` | Playwright-based web automation |
| `service enable/start/status` | Background sync service + local search API |
| `config show/path/edit` | Configuration management |
| `stats` | Database statistics |
| `skill install/status/update` | Install the bundled agent retrieval skill |
| `mmry extract` | Export memories to mmry |

Adapter installs are version-pinned to the hstry binary. Run `hstry adapters update` after upgrading; sync refuses to run if adapter manifests do not match.

## Browser extension

`extension/` contains **hstry sync**, a Chrome MV3 extension that background-syncs conversations from ChatGPT, Claude, Gemini, and Perplexity into your local database. It POSTs new conversations to a running `hstry-api` instance (`http://127.0.0.1:3000/ingest`, token-authenticated).

```bash
hstry-api --port 3000   # start the API, optionally with --token <secret>
```

Load it from `chrome://extensions` with Developer mode enabled (Load unpacked, select `extension/`). Provider toggles, port, and token are configured on the extension's options page. The `hstry web` Playwright commands are the headless alternative to the extension.

## Search

The query type is auto-detected: natural-language queries use porter stemming, code queries preserve underscores, dots, and path separators. Force with `--mode natural|code`.

Useful flags: `--scope local|remote|all`, `--remote <name>`, `--source`, `--workspace`, `--role`, `--no-tools`, `--dedup`, `--json`.

## Session resume

`resume` reopens a past session in any supported agent, converting formats when needed (a Codex session can resume in Claude Code and vice versa).

```bash
hstry resume <conversation-id>
hstry resume --search "async runtime refactor" --agent claude-code
hstry resume --after "2 days ago" --workspace myproject
hstry resume --limit 10                # browse and pick interactively
```

Time filters accept ISO dates, relative dates (`yesterday`, `last week`), and durations (`3 weeks ago`). Defaults come from config:

```toml
[resume]
default_agent = "pi"

[resume.agents.pi]
format = "pi"
command = "pi --session {session_path}"
session_dir = "~/.pi/agent/sessions"
```

Placeholders: `{session_path}`, `{session_id}`, `{workspace}`.

## Remote sync

Sync and search other machines' databases over SSH. Remotes need `hstry` installed.

```bash
hstry remote add laptop user@laptop
hstry remote test laptop
hstry remote fetch --remote laptop
hstry search "auth error" --scope remote --remote laptop
hstry remote sync --remote laptop --direction pull
```

See [docs/remote-sync.md](docs/remote-sync.md) for device namespaces and concurrency guidance.

## Service and API

`hstry service` runs a daemon that keeps the search index warm and exposes a local-only gRPC search endpoint; the CLI prefers it when running. `hstry-api` serves a local HTTP API (default `127.0.0.1:3000`) for external integrations, including the browser extension.

Environment overrides: `HSTRY_NO_SERVICE=1`, `HSTRY_API_URL`, `HSTRY_NO_API=1`.

## Supported sources

### Local agents

| Adapter | Default path |
|---------|--------------|
| `claude-code` | `~/.claude/projects` |
| `codex` | `~/.codex/sessions` |
| `cursor` | Cursor `workspaceStorage` (state.vscdb) |
| `opencode` | `~/.local/share/opencode` |
| `pi` | `~/.pi/agent/sessions` |
| `gemini-cli` | `~/.gemini/tmp` |
| `workbuddy` | `~/.workbuddy/projects` |
| `aider` | `.aider.chat.history.md` in project directories |
| `goose` | `~/.local/share/goose/sessions` |
| `jan` | `~/jan/threads` |
| `lmstudio` | `~/.cache/lm-studio/conversations` |
| `openwebui` | `~/.open-webui/data` |

### Web

- **Browser extension** (`extension/`): ChatGPT, Claude, Gemini, Perplexity
- **Manual exports**: `chatgpt` (Settings > Data controls > Export), `claude-web` (Settings > Export data), `gemini` (Google Takeout)

## Adapters

Adapters are TypeScript modules that implement `detect(path)` and `parse(path, options)`. They run via Bun/Deno/Node and are loaded from `adapter_paths`. Repositories (git, archive, local) can be managed with `hstry adapters repo`.

```bash
hstry adapters repo add-git community https://example.com/adapters.git
hstry adapters update
```

## Configuration

XDG paths: config in `~/.config/hstry/`, data in `~/.local/share/hstry/`, state in `~/.local/state/hstry/` (overridable via `XDG_*`). Default config is `~/.config/hstry/config.toml`:

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

[resume]
default_agent = "pi"
```

See `examples/config.toml` for all options.

## Development

```bash
just check-all        # format, lint, test
just update-adapters  # copy adapters to ~/.config/hstry/adapters
```

Workspace layout: `hstry-core` (database, config, models), `hstry-runtime` (adapter execution), `hstry-cli`, `hstry-tui` (ratatui), `hstry-mcp`, `hstry-api` (axum). Releases are automated via GitHub Actions; see [docs/RELEASE.md](docs/RELEASE.md) and [CHANGELOG.md](CHANGELOG.md).

## Attribution

Inspired by [cross-agent-session-search (cass)](https://github.com/Dicklesworthstone/coding_agent_session_search) by Jeffrey Emanuel (MIT).

## License

MIT
