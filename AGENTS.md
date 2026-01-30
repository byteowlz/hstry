# AGENTS.md

Guidance for coding agents working on this Rust CLI template.

## Core Principles

- **Never publish** artifacts to public registries without explicit user approval.
- We favor clean refactors over backwards compatibility; update existing code in place (no `FooV2` suffixes).
- Target Windows 11, Linux, and macOS 14+ with the same behavior; no legacy OS shims.
- Keep file headers minimal—no author or timestamp banners.

## Rust Workflow

- Follow Clippy best practices: collapse trivial `if`s, inline `format!` arguments, and prefer method references over redundant closures.
- When tests compare structures, assert on the full value instead of individual fields.
- Run `cargo fmt` after code changes and `cargo test` for the touched crate. Invoke broader test or lint commands only if the user asks.

## CLI Expectations

- Prefer subcommands for verbs and keep outputs quiet/verbose via standard flags (`-q`, chainable `-v`, `--debug`, `--trace`).
- Support machine-readable modes via `--json/--yaml` and honor NO_COLOR/FORCE_COLOR.
- Offer `--dry-run`, `--yes/--force`, `--no-progress`, `--timeout`, and `--parallel` when operations warrant them.
- Generate help quickly (`-h/--help`) and provide shell completions off the same Clap definitions.

## Configuration & Storage

- Use XDG directories when available: config at `$XDG_CONFIG_HOME/<app>/config.toml`, data at `$XDG_DATA_HOME/<app>`, state at `$XDG_STATE_HOME/<app>` with sensible fallbacks (e.g., `~/.config`).
- Expand `~` and environment variables in config paths.
- Ship a commented example under `examples/`, create a default config on first run, and load overrides via the `config` crate.

## House Rules

- Do exactly what the user asks—no unsolicited files or docs.
- Keep README updates concise, emoji-free, and only when requested.
- Never commit secrets or sensitive paths; scrub logs before surfacing them.

## Justfile Commands

This project uses [just](https://github.com/casey/just) as a command runner. Run `just` to see available commands.

**Core commands:**
```bash
just              # Show available commands
just install-all  # Install all binaries
just install-crate CRATE # Install specific crate
just build        # Debug build (all crates)
just build-release # Release build (all crates)
just test         # Run all tests
just fmt          # Format all code
just clippy       # Run linter on all crates
just check-all    # Format + lint + test
```

**Workspace navigation:**
```bash
just list         # List all crates
just list-bins    # List binary crates
just list-libs    # List library crates
just build-crate CRATE  # Build specific crate
just test-crate CRATE   # Test specific crate
just clippy-crate CRATE # Lint specific crate
```

**Development workflow:**
```bash
just check        # Fast compile check
just fix          # Auto-fix clippy warnings
just docs         # Generate documentation
just update       # Update dependencies
```

Always run `just check-all` before committing significant changes.

## Issue Tracking (trx)

Use `trx` for all issue tracking. Do NOT use markdown TODOs or external trackers.

```bash
trx ready                              # Find unblocked work
trx create "Title" -t task -p 2        # Create issue (types: bug/feature/task/epic/chore)
trx update <id> --status in_progress   # Claim task
trx close <id> -r "Done"               # Complete work
trx sync                               # Commit .trx/ changes
```

Priorities: 0=critical, 1=high, 2=medium (default), 3=low, 4=backlog

Always commit `.trx/` with code changes.

## Memory System (byt/mmry)

Use `byt memory` to store and retrieve project knowledge. Memories auto-detect the current repo.

**Adding memories:**
```bash
byt memory add "Important decision or learning"              # Auto-detects current repo
byt memory add "Cross-repo architecture decision" --govnr    # Force govnr store
byt memory add "Specific insight" -c "architecture" -i 8     # With category and importance
```

**Searching memories:**
```bash
byt memory search "query"           # Search current repo's memories
byt memory search "query" --govnr   # Search cross-repo memories
byt memory search "query" --all     # Search ALL projects
```

**When to add memories:**
- Architecture decisions and their rationale
- Non-obvious solutions to tricky problems
- Integration patterns with other byteowlz repos
- Performance findings or benchmarks
- API contracts or breaking changes

**When to search memories:**
- Before starting work on a feature (check for prior decisions)
- When encountering unfamiliar code patterns
- When integrating with other repos (`byt memory search "query" --all`)

## Adapters

Adapters are TypeScript modules in `adapters/<name>/adapter.ts` that parse conversation data from various sources. They run via Bun and communicate with the Rust runtime via JSON.

**Adapter deployment:**
```bash
just update-adapters    # Copy adapters to ~/.config/hstry/adapters
```

After modifying any adapter, always run `just update-adapters` before testing. The CLI loads adapters from the config directory, not the source tree.

**Testing adapters directly:**
```bash
# Test detection
HSTRY_REQUEST='{"method":"detect","params":{"path":"/path/to/file.json"}}' \
  bun run ~/.config/hstry/adapters/<name>/adapter.ts

# Test parsing (with limit)
HSTRY_REQUEST='{"method":"parse","params":{"path":"/path/to/file.json","opts":{"limit":1}}}' \
  bun run ~/.config/hstry/adapters/<name>/adapter.ts
```

**Type contract with Rust runtime:**
- Timestamps must be integers (milliseconds). Use `Math.floor()` when converting from float seconds.
- The Rust `ParsedConversation` struct uses `created_at: i64` - floats will cause deserialization errors.
- Response types are defined in `adapters/types/index.ts` and must match `crates/hstry-runtime/src/runner.rs`.

**Common adapter issues:**
- "Could not detect format" - The `detect()` method returned null. Check file path patterns and content detection logic.
- "data did not match any variant" - Usually a type mismatch. Check that timestamps are integers, not floats.
- "No conversations found" - The `parse()` method returned empty array. Check file content parsing logic.

## Releases & Distribution

This project uses GitHub Actions for automated releases. See `.github/workflows/release.yml`.

**Creating a release:**
```bash
# Tag-based (automatic trigger)
git tag v1.0.0 && git push --tags

# Manual trigger via CLI
gh workflow run release.yml -f tag=v1.0.0
```

**What the workflow builds:**
- Linux x86_64 (ubuntu-latest)
- macOS x86_64 (cross-compiled from macos-14 ARM64)
- macOS ARM64 (macos-14)
- Windows x86_64 (if enabled)

**Disabled by default (uncomment in workflow if needed):**
- Linux ARM64: Requires `Cross.toml` with OpenSSL configuration
- Windows: May have C runtime mismatch issues with some crates

**Platform notes:**
- `macos-13` runner is retired - always use `macos-14`
- For protobuf projects: uncomment the protoc installation steps
- For ML projects: uncomment `--features coreml` for Apple Silicon
- Workspace builds package all binaries matching `hstry*`

**Required secrets for package managers:**
- `TAP_GITHUB_TOKEN` - PAT with repo access to byteowlz/homebrew-tap
- `AUR_SSH_PRIVATE_KEY` - SSH key registered with AUR
- `AUR_EMAIL` - Email for AUR commits

Use `byt secrets setup <repo>` to configure secrets.

**Installation methods (once published):**
```bash
# Homebrew (macOS/Linux)
brew install byteowlz/tap/<binary-name>

# AUR (Arch Linux)
yay -S <binary-name>

# Scoop (Windows)
scoop bucket add byteowlz https://github.com/byteowlz/scoop-bucket
scoop install <binary-name>
```
