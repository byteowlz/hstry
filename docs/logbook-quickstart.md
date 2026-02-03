# Logbook: Quick Summary

## Decision: Build with pi agent-loop

The logbook generation will be built in **pi-mono** using **pi's agent-loop** infrastructure.

---

## Architecture

```
hstry (data)
  └─ Exports conversations via CLI
      ↓
pi agent-loop (processing)
  └─ Maintains logbook state in context
  └─ Calls LLM via @mariozechner/pi-ai
  └─ Tool: append_events()
      ↓
logbook.json/text/markdown
```

---

## Why pi agent-loop?

| Benefit | Explanation |
|---------|-------------|
| Separation | hstry = data, pi = processing |
| Infrastructure | Proven agent-loop, battle-tested |
| LLM integration | Uses @mariozechner/pi-ai |
| Context management | Built-in compaction, state tracking |
| Tool ecosystem | pi tools system |
| Testing | Excellent test utilities |

---

## Implementation

### hstry side
```bash
# Export conversations
hstry logbook export --workspace my-project
```

### pi agent side
```typescript
// pi-mono/packages/logbook-agent/
export async function generateLogbook(
    workspace: string,
    model: Model<any>
): Promise<TimelineEvent[]>
```

### User interface
```bash
hstry logbook --workspace my-project
```

---

## Timeline Format

```json
{
  "workspace": "my-project",
  "generated_at": "2026-02-02T13:11:53Z",
  "timeline": [
    {
      "timestamp": "2026-01-15T10:30:00Z",
      "type": "decision",
      "content": "Use PostgreSQL as primary database",
      "rationale": "ACID transactions and JSONB support",
      "alternatives": ["MySQL", "MongoDB"],
      "source": { "conversation_id": "uuid" }
    }
  ]
}
```

---

## Text Output

```
=== Project Logbook: my-project ===
Generated: 2026-02-02 13:11:53 UTC

2026-01-15 | DECISION | Use PostgreSQL as primary database
  → Rationale: ACID transactions and JSONB support
  → Alternatives: MySQL, MongoDB

2026-01-18 | FACT | Authentication uses JWT with RS256

2026-01-28 | BUG | Memory leak in connection pool
  → Resolution: Fixed in v0.2.1
```

---

## Processing Loop

```typescript
For each conversation (oldest → newest):
  Context = [current_logbook, conversation_chunk]
  → LLM: "Append meaningful events"
  → Agent updates logbook state
  → Continue
```

**Key:** Logbook travels as context, agent only adds what's new.

---

## Files Created

| File | Purpose |
|------|---------|
| `logbook-pi-agent-loop.md` | Full implementation details |
| `logbook-architecture-final-decision.md` | Decision analysis |
| `logbook-incremental-loop.md` | Processing algorithm |
| `logbook-timeline-design.md` | JSON schema |
| `schemas/logbook-timeline-v1.json` | JSON Schema |
| `schemas/logbook-timeline.types.ts` | TypeScript types |
| `examples/*.json/txt` | Example outputs |

---

## Next Steps

1. **hstry CLI**: Add `hstry logbook export` command
2. **pi-mono**: Create `packages/logbook-agent`
3. **Implement**: Generate logbook using agent-loop
4. **Test**: On hstry itself
5. **Integrate**: hstry CLI calls pi agent

---

## CLI Examples

```bash
# Basic
hstry logbook --workspace my-project

# JSON output
hstry logbook --workspace my-project --format json

# Incremental update
hstry logbook --workspace my-project --continue-from logbook.json

# Custom model
hstry logbook --workspace my-project --provider ollama --model llama3.1
```

---

## Key Principle

**The logbook IS context.** It travels with each chunk, so the agent always knows what's already captured. No separate deduplication needed - the agent simply won't add what's already there.

---

**Verdict:** Logbook is an agent task. Build it in pi-mono using agent-loop, call from hstry CLI.
