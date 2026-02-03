# Logbook Implementation: Recommendation

## Decision: Build in hstry

The logbook functionality should be implemented **in hstry itself**, not in mmry or pi-mono.

---

## Why hstry?

| Reason | Explanation |
|---------|-------------|
| **Data ownership** | Logbook is derived FROM hstry's conversation data |
| **Direct access** | Needs database access - hstry owns it |
| **Natural fit** | Logbook = summary of what hstry stores |
| **Simplicity** | One codebase, no external dependencies |
| **Performance** | Direct DB queries, no API overhead |

## Architecture

```
hstry (owns implementation)
  └─ hstry-logbook crate
      └─ CLI: hstry logbook --workspace X
          └─ Output: text/JSON/markdown

pi-mono (consumes logbook)
  └─ Agent tool: get_project_logbook()
      └─ Calls: hstry logbook CLI

AI agents (end users)
  └─ Use logbook for context
```

---

## Implementation Plan

### Phase 1: Core functionality
```rust
// New crate: crates/hstry-logbook/
hstry-logbook/
  └─ src/
      ├─ lib.rs           // Public API
      ├─ generator.rs     // Main generation loop
      ├─ llm.rs          // LLM client abstraction
      ├─ timeline.rs      // Timeline types
      └─ output.rs       // Format rendering
```

```bash
# CLI usage
hstry logbook --workspace my-project --format json
hstry logbook --workspace my-project --continue-from logbook.json
```

### Phase 2: LLM providers
- Anthropic (default)
- OpenAI
- Ollama (local)

### Phase 3: pi-mono integration
```typescript
// Agent tool
get_project_logbook({ workspace, format, max_events })
  → calls hstry CLI
  → returns logbook JSON
```

---

## Storage Strategy

**Primary:** File-based (simple, default)
```bash
hstry logbook --workspace my-project --output logbook.json
```

**Optional:** mmry integration
```bash
hstry logbook --workspace my-project | byt memory add -c logbook
```

**Future option:** hstry DB storage
- Queryable
- Single source of truth

---

## Files Created for Design

| File | Purpose |
|------|---------|
| `logbook-timeline-design.md` | JSON schema and format |
| `logbook-incremental-loop.md` | Processing algorithm |
| `logbook-architecture-decision.md` | Where to implement analysis |
| `logbook-code-blocks-decision.md` | Code handling details |
| `logbook-json-schema.md` | (old, complex) |
| `schemas/logbook-timeline-v1.json` | Simple JSON schema |
| `schemas/logbook-timeline.types.ts` | TypeScript types |
| `examples/logbook-timeline-example.*` | Example outputs |

---

## Next Steps

1. **Create `hstry-logbook` crate**
2. **Implement generation loop** with LLM client
3. **Add CLI command** to hstry-cli
4. **Test on hstry itself**
5. **Create pi-mono agent tool**
6. **Document for agent developers**

---

## Key Design Principles

1. **Logbook IS context** - Travels with each chunk during processing
2. **Incremental** - Update existing logbooks without full regeneration
3. **One-page** - Always fits in LLM context (~6,000 tokens max)
4. **Simple** - Agent just asks: "Does this chunk add anything?"
5. **Modular** - LLM providers are pluggable

---

## Quick Reference

```bash
# Generate logbook
hstry logbook --workspace my-project

# JSON for AI consumption
hstry logbook --workspace my-project --format json

# Update existing
hstry logbook --workspace my-project --continue-from logbook.json

# Limit events
hstry logbook --workspace my-project --max-events 30

# Agent tool (pi-mono)
agent.callTool('get_project_logbook', { workspace: 'my-project' })
```
