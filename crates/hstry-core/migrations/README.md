# Database Migrations

This directory contains SQL migration files for the hstry database schema.

## Naming Convention

Migrations must follow the naming pattern: `NNN_description.sql`

- `NNN` - Three-digit zero-padded migration number (e.g., `001`, `002`, `003`)
- `description` - Short snake_case description of what the migration does
- `.sql` - File extension

Examples:
- `001_initial_schema.sql`
- `002_add_provider_column.sql`
- `003_add_provider_index.sql`

## Adding a New Migration

1. Create a new SQL file with the next available number
2. Write your migration SQL (use `CREATE TABLE IF NOT EXISTS`, `ALTER TABLE ADD COLUMN`, etc.)
3. Update the embedded migrations list in `src/db.rs`:

```rust
async fn run_embedded_migrations(&self) -> Result<()> {
    let migrations: &[(&str, &str)] = &[
        ("001_initial_schema.sql", include_str!("../migrations/001_initial_schema.sql")),
        ("002_add_provider_column.sql", include_str!("../migrations/002_add_provider_column.sql")),
        ("003_add_provider_index.sql", include_str!("../migrations/003_add_provider_index.sql")),
        ("004_your_new_migration.sql", include_str!("../migrations/004_your_new_migration.sql")), // <-- Add here
    ];
```

4. Test the migration locally by deleting your database and running hstry
5. Commit the migration file and the updated `src/db.rs`

## Migration Tracking

Applied migrations are tracked in the `schema_migrations` table:

```sql
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);
```

Each migration is only applied once. If you need to re-run a migration, you must manually delete its row from the `schema_migrations` table.

## Best Practices

- Use `IF NOT EXISTS` for table/index creation
- Use `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` (if supported) or check with `PRAGMA` first
- Keep migrations backward-compatible when possible
- Test migrations on a copy of production data
- Include comments explaining complex changes
- For breaking changes, consider using feature flags in the application code

## Runtime vs Migrations

Some schema checks are still performed at runtime via the `ensure_*` methods in `src/db.rs`. These are for:

1. **Data backfills** - Populating new columns with computed values (e.g., `readable_id`)
2. **FTS maintenance** - Checking and rebuilding full-text search indexes
3. **Graceful degradation** - Handling schema variations for backwards compatibility

These are not migrations because they're idempotent and may need to run on every startup.

## Troubleshooting

### "no such column" error

This usually means a migration hasn't run yet. Check the `schema_migrations` table:

```bash
sqlite3 ~/.local/state/hstry/hstry.db "SELECT * FROM schema_migrations ORDER BY version;"
```

### Migration already applied

If you need to re-run a migration:

```bash
sqlite3 ~/.local/state/hstry/hstry.db "DELETE FROM schema_migrations WHERE version = NNN;"
```

Then restart the application.

### Database location

The default database location is:
- `~/.local/state/hstry/hstry.db` (Linux)
- `~/Library/Application Support/hstry/hstry.db` (macOS)
- `%LOCALAPPDATA%\hstry\hstry.db` (Windows)
