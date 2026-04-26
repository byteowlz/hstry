# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.16] - 2026-04-26

### Changed

- **TUI left pane**: Group source entries by adapter instead of individual source ID. One entry per adapter (e.g., one `pi`, one `claude-code`) even when multiple source IDs exist for the same adapter.
- **`hstry list`**: Automatically deduplicate conversations across sources by `external_id`/`readable_id`/`platform_id`. Canonical source IDs (e.g. `pi`) take precedence over `import-pi` style IDs.
- **`hstry import`**: Reuse existing source when importing to a path that already has one, or default to canonical adapter ID instead of `import-<adapter>`. Prevents duplicate source creation.
- **Startup performance**: Replace `COUNT(*)` scans on FTS5 tables with `SELECT 1 LIMIT 1` probes during DB init. Eliminates multi-second startup latency on large databases.

## [0.5.15] - 2026-01-28

### Added

- (previous releases)
