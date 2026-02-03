# Logbook Timeline - Simple Chronological Events

## Concept

A one-page, line-based timeline of important facts and decisions. Chronological order, concise, human and AI readable.

## Format

### Human-Readable Text

```
=== Project Logbook: my-project ===
Generated: 2026-02-02 13:11:53 UTC
Source Range: 2026-01-15 → 2026-02-02 (47 conversations)

2026-01-15 10:30 | DECISION | Use Rust with Tokio for async runtime
  → Rationale: Need async operations without blocking threads
  → Source: conv:a1b2c3d4

2026-01-16 14:20 | DECISION | PostgreSQL chosen as primary database
  → Rationale: ACID transactions and JSONB support
  → Source: conv:c3d4e5f6

2026-01-18 09:00 | FACT | Authentication uses JWT with RS256 signing
  → Source: conv:d4e5f6a7

2026-01-20 11:30 | DECISION | Adopt async/await with Result<T, E> error handling
  → Rationale: Explicit error handling, better than panics
  → Alternatives: Callbacks, Option for errors
  → Code: `async fn fetch_user(id: i32) -> Result<User>`
  → Source: conv:e5f6a7b8

2026-01-28 09:45 | BUG | Memory leak in connection pool when not released
  → Fix: Ensure `.await` is called for all pool operations
  → Source: conv:f6a7b8c9
```

### JSON (AI-consumable)

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
      "type": "decision",
      "content": "Use Rust with Tokio for async runtime",
      "rationale": "Need async operations without blocking threads",
      "source": { "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
    },
    {
      "timestamp": "2026-01-16T14:20:00Z",
      "type": "decision",
      "content": "PostgreSQL chosen as primary database",
      "rationale": "ACID transactions and JSONB support",
      "alternatives": ["MySQL", "MongoDB"],
      "source": { "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012" }
    },
    {
      "timestamp": "2026-01-18T09:00:00Z",
      "type": "fact",
      "content": "Authentication uses JWT with RS256 signing",
      "source": { "conversation_id": "d4e5f6a7-b8c9-0123-def0-123456789012" }
    },
    {
      "timestamp": "2026-01-20T11:30:00Z",
      "type": "decision",
      "content": "Adopt async/await with Result<T, E> error handling",
      "rationale": "Explicit error handling, better than panics",
      "alternatives": ["Callbacks", "Option for errors"],
      "code": "async fn fetch_user(id: i32) -> Result<User>",
      "source": { "conversation_id": "e5f6a7b8-c9d0-1234-ef01-234567890123" }
    },
    {
      "timestamp": "2026-01-28T09:45:00Z",
      "type": "bug",
      "content": "Memory leak in connection pool when not released",
      "resolution": "Fix: Ensure `.await` is called for all pool operations",
      "source": { "conversation_id": "f6a7b8c9-d0e1-2345-f012-345678901234" }
    }
  ]
}
```

## JSON Schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "properties": {
    "workspace": { "type": "string" },
    "generated_at": { "type": "string", "format": "date-time" },
    "source_range": {
      "type": "object",
      "properties": {
        "from": { "type": "string", "format": "date-time" },
        "to": { "type": "string", "format": "date-time" }
      },
      "required": ["from", "to"]
    },
    "timeline": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "timestamp": { "type": "string", "format": "date-time" },
          "type": {
            "type": "string",
            "enum": ["decision", "fact", "bug"]
          },
          "content": {
            "type": "string",
            "maxLength": 300
          },
          "rationale": {
            "type": "string",
            "maxLength": 500
          },
          "alternatives": {
            "type": "array",
            "items": { "type": "string", "maxLength": 200 }
          },
          "code": {
            "type": "string",
            "maxLength": 500
          },
          "resolution": {
            "type": "string",
            "maxLength": 500
          },
          "source": {
            "type": "object",
            "properties": {
              "conversation_id": { "type": "string", "format": "uuid" },
              "message_id": { "type": "string", "format": "uuid" }
            }
          }
        },
        "required": ["timestamp", "type", "content", "source"]
      }
    }
  },
  "required": ["workspace", "generated_at", "source_range", "timeline"]
}
```

## Types

```ts
type EventType = 'decision' | 'fact' | 'bug';

interface TimelineEntry {
  timestamp: string;           // ISO8601, sorted chronologically
  type: EventType;
  content: string;             // What happened (1 sentence max)
  rationale?: string;          // For decisions: why
  alternatives?: string[];      // For decisions: what else was considered
  code?: string;               // Brief code snippet (optional)
  resolution?: string;         // For bugs: how it was fixed
  source: {
    conversation_id: string;    // Link back to history
    message_id?: string;
  };
}

interface Logbook {
  workspace: string;
  generated_at: string;
  source_range: {
    from: string;
    to: string;
  };
  timeline: TimelineEntry[];
}
```

## Extraction Prompt

```
Extract important events from this conversation as a chronological timeline.

For each event:
1. Assign a type: "decision", "fact", or "bug"
2. Write a concise content line (under 100 chars preferred)
3. Include rationale for decisions
4. List alternatives for decisions (if discussed)
5. Include brief code snippets only if critical to understanding
6. Link to the source conversation

Guidelines:
- Only extract high-value events (skip trivial details)
- Keep content to 1-2 sentences max
- Code snippets: only for decisions where code IS the decision (e.g., "use Result<T,E>")
- Output in chronological order (newest first or as specified)
- Format as JSON array of objects

Output ONLY JSON matching the schema.

Conversation:
{conversation}

Timeline:
```

## CLI Usage

```bash
# Generate timeline (human-readable by default)
hstry logbook --workspace my-project

# Output JSON for AI consumption
hstry logbook --workspace my-project --format json

# Include code snippets
hstry logbook --workspace my-project --include-code

# Filter by date range
hstry logbook --workspace my-project --from 2026-01-01 --to 2026-01-31

# Limit to N most recent events
hstry logbook --workspace my-project --limit 20

# Only decisions
hstry logbook --workspace my-project --type decision
```

## Deduplication Strategy

Since it's a flat timeline, deduplication is simpler:

1. **Exact content match**: Remove duplicates
2. **Similar content**: Merge with `→ Later: ...` suffix
3. **Same decision with new info**: Update rationale, add alternatives

```rust
fn deduplicate_timeline(events: Vec<TimelineEntry>, threshold: f32) -> Vec<TimelineEntry> {
    let mut deduped = Vec::new();

    for event in events {
        let mut duplicate_of = None;

        for existing in &deduped {
            let similarity = cosine_similarity(
                embed(&event.content),
                embed(&existing.content)
            );

            if similarity > threshold {
                duplicate_of = Some(existing);
                break;
            }
        }

        match duplicate_of {
            Some(existing) => {
                // Merge
                if event.timestamp > existing.timestamp {
                    existing.content = format!("{} → Later: {}", existing.content, event.content);
                    if let Some(rationale) = event.rationale {
                        existing.rationale = Some(format!("{}; Later: {}", existing.rationale.unwrap_or_default(), rationale));
                    }
                }
            }
            None => deduped.push(event),
        }
    }

    deduped.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    deduped
}
```

## Processing Pipeline

```
┌─────────────────────────────────────┐
│  List Conversations (newest first) │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│  For each conversation:             │
│  1. Get messages                    │
│  2. Chunk (1 conversation = 1 chunk)│
│  3. Extract events via LLM          │
│  4. Append to timeline              │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│  Sort all events chronologically    │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│  Deduplicate (embedding similarity) │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│  Output (text or JSON)              │
└─────────────────────────────────────┘
```

## Configuration

```toml
[logbook]
# Default output format
default_format = "text"  # text | json

# Include code snippets?
include_code = false     # Can be overridden with --include-code

# Deduplication threshold (0.0-1.0)
similarity_threshold = 0.85

# Maximum timeline length (0 = unlimited)
max_events = 0
```

## Incremental Updates

To update an existing logbook:

```bash
# Continue from existing logbook, only process newer conversations
hstry logbook --workspace my-project --continue-from logbook.json --output updated.json
```

Implementation:
- Parse existing logbook, get `source_range.to`
- Only query conversations with `updated_at > existing_range.to`
- Append new events to existing timeline
- Re-deduplicate entire timeline

## Example Output (Minimal, No Code)

```
=== Project Logbook: my-project ===
Generated: 2026-02-02 13:11:53 UTC
Source: 47 conversations, Jan 15 → Feb 2

2026-02-01 10:30 | DECISION | Deploy to Kubernetes on AWS EKS
2026-01-28 09:45 | BUG | Memory leak in connection pool (fixed in v0.2.1)
2026-01-25 16:00 | DECISION | Use GitHub Actions for CI/CD
2026-01-20 11:30 | DECISION | Adopt async/await with Result error handling
2026-01-18 09:00 | FACT | Authentication uses JWT with RS256
2026-01-16 14:20 | DECISION | PostgreSQL as primary database
2026-01-15 10:30 | DECISION | Rust with Tokio for async runtime
```

## Key Differences from Previous Design

| Aspect | Previous | New |
|--------|----------|-----|
| Structure | Separate facts/decisions arrays | Single timeline array |
| Metadata | Complex with stats | Simple: workspace, date range |
| Fields per entry | 10+ fields | 4-6 fields (type, content, rationale, code?, source) |
| Output | Full JSON | Human-readable text OR simple JSON |
| Purpose | Structured database | Quick reference for AI/humans |
| Size | 50+ lines for simple project | 10-20 lines |
