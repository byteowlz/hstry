# Logbook Design Summary

## Core Concept

A one-page chronological timeline of decisions, facts, and bugs that gives AI agents and humans a quick understanding of what happened in a project.

## Processing Loop

```
For each conversation (oldest → newest):
  1. Context = [current_logbook, current_chunk]
  2. Agent: "Append meaningful events from this chunk"
  3. Logbook += new_events
  4. Continue
```

**Key:** The logbook itself travels as context, so the agent always knows what's already captured. No separate deduplication step needed.

## JSON Structure

```json
{
  "workspace": "my-project",
  "generated_at": "2026-02-02T13:11:53Z",
  "source_range": {
    "from": "2026-01-15T10:30:00Z",
    "to": "2026-02-02T11:00:00Z"
  },
  "timeline": [
    {
      "timestamp": "2026-01-15T10:30:00Z",
      "type": "decision|fact|bug",
      "content": "Brief one-sentence description",
      "rationale": "Why (for decisions)",
      "alternatives": ["option1", "option2"],
      "code": "brief_code_snippet",      // Optional
      "resolution": "How fixed (for bugs)", // Optional
      "source": {"conversation_id": "uuid", "message_id": "uuid"}
    }
  ]
}
```

## Text Output

```
=== Project Logbook: my-project ===
Generated: 2026-02-02 13:11:53 UTC

2026-02-01 | DECISION | Deploy to Kubernetes on AWS EKS
  → Rationale: Team expertise and managed service benefits
  → Alternatives: Docker Compose on EC2, AWS ECS

2026-01-28 | BUG | Memory leak in connection pool
  → Resolution: Fixed in v0.2.1, ensure .await on all operations

2026-01-20 | DECISION | Adopt async/await with Result error handling
  → Rationale: Explicit error handling, better than panics
  → Code: async fn fetch_user(id: i32) -> Result<User>
```

## Agent Prompt

```
CURRENT LOGBOOK:
{logbook_text}

CURRENT CHUNK:
{chunk_text}

TASK: Append meaningful events from this chunk.

Rules:
- Only add events NOT already captured
- Event types: decision, fact, bug
- Keep content concise (1 sentence)
- Include rationale for decisions
- Brief code only if critical

OUTPUT: {"events": [...]}
```

## Context Management

| Component | Tokens |
|-----------|--------|
| Logbook (30 events) | ~2,000 |
| Chunk (conversation) | ~1,500 |
| Instructions | ~300 |
| **Total** | **~3,800** |

Fits easily in 32k+ context windows. Logbook self-limits to ~100 events = ~6,000 tokens max.

## CLI

```bash
# Generate logbook
hstry logbook --workspace my-project

# Update existing (incremental)
hstry logbook --workspace my-project --continue-from logbook.json

# Output formats
hstry logbook --workspace my-project --format text    # Human-readable
hstry logbook --workspace my-project --format json     # AI-consumable
```

## Files Created

- `docs/logbook-timeline-design.md` - Design concepts
- `docs/logbook-incremental-loop.md` - Processing loop details
- `docs/schemas/logbook-timeline-v1.json` - JSON Schema
- `docs/schemas/logbook-timeline.types.ts` - TypeScript types
- `docs/examples/logbook-timeline-example.json` - JSON example
- `docs/examples/logbook-timeline-example.txt` - Text example

## Key Design Decisions

1. **Logbook IS context** - Not something extracted from context
2. **Incremental append** - One conversation at a time
3. **Agent focuses on "what's new"** - Simple comparison task
4. **No separate deduplication** - Logbook context prevents duplicates
5. **One-page format** - Always fits in context window
