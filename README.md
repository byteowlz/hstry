# Rust Workspace Template

A batteries-included Rust workspace template with CLI, TUI, MCP server, and HTTP API crates sharing a common core library.

## Quick Start

Install the latest stable Rust toolchain (`rustup default stable`), then:

```bash
cargo build
cargo test
```

Run individual binaries:

```bash
cargo run -p hstry-cli -- run
cargo run -p hstry-tui
cargo run -p hstry-api -- --port 3000
cargo run -p hstry-mcp
```

Scaffold a new project:

```bash
scripts/new-cli.sh my-app
```

```powershell
pwsh scripts/new-cli.ps1 my-app
```

This creates a new workspace with all crates renamed (e.g., `my-app-core`, `my-app-cli`, etc.).

## Workspace Structure

```
crates/
  hstry-core/    # Shared library: config, paths, error types
  hstry-cli/     # Command-line interface
  hstry-tui/     # Terminal user interface (ratatui)
  hstry-mcp/     # Model Context Protocol server
  hstry-api/     # HTTP API server (axum)
examples/
  config.toml   # Example configuration
scripts/
  new-cli.sh    # Unix scaffolding script
  new-cli.ps1   # PowerShell scaffolding script
```

## Crates

### hstry-core

Shared library providing:
- `AppConfig` - Configuration loading via `config` crate
- `AppPaths` - XDG-compliant path resolution
- Error types and common utilities

### hstry-cli

Command-line interface with:
- Subcommands: `run`, `init`, `config`, `completions`
- Global flags: `-q`, `-v`, `--debug`, `--trace`, `--json`, `--yaml`, `--no-color`, `--dry-run`, `--yes`
- Shell completion generation

```bash
cargo run -p hstry-cli -- --help
cargo run -p hstry-cli -- completions bash > target/hstry-cli.bash
```

### hstry-tui

Terminal UI built with ratatui featuring:
- Three-pane layout (navigation, list, details)
- Vim-style navigation (j/k/h/l)
- Modal help system

```bash
cargo run -p hstry-tui
```

### hstry-mcp

MCP (Model Context Protocol) server exposing tools:
- `get_profile` - Current configuration profile
- `echo` - Echo messages
- `get_runtime_config` - Runtime configuration

```bash
cargo run -p hstry-mcp
```

### hstry-api

HTTP API server (axum) with endpoints:
- `GET /` - Service info
- `GET /health` - Health check
- `GET /config` - Current configuration

```bash
cargo run -p hstry-api -- --port 3000
curl http://localhost:3000/health
```

## Configuration

Default config path: `$XDG_CONFIG_HOME/rust-workspace/config.toml`

Override with `--config <path>` or environment variables using the `RUST_WORKSPACE__` prefix:

```bash
RUST_WORKSPACE__LOGGING__LEVEL=debug cargo run -p hstry-cli -- run
```

See `examples/config.toml` for all options.

## Development

```bash
cargo fmt                                    # Format code
cargo clippy --all-targets --all-features   # Lint
cargo test                                   # Run tests
cargo build --release                        # Release build
```

## Scaffolding

The `scripts/new-cli.sh` (Unix) and `scripts/new-cli.ps1` (PowerShell) scripts create a new project from this template:

```bash
scripts/new-cli.sh my-app --path ~/projects/my-app
```

This will:
1. Copy the template to the destination
2. Rename all crates from `rust-*` to `my-app-*`
3. Update all references in Cargo.toml, source files, and documentation
4. Rename crate directories accordingly

Requirements: `python3` for the shell script, PowerShell 7 for the Windows script.
