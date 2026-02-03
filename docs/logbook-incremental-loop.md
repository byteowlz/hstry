# Incremental Logbook Processing Loop

## Core Insight

**The logbook IS the context.** Keep the entire logbook in memory and pass it along with each chunk. The agent only needs to decide: "Does this chunk add anything new?"

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Processing Loop                       │
│  (oldest conversation → newest conversation)                │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  State:                                                   │
│    - logbook_so_far: LogbookTimeline (in context)          │
│    - processed_conversations: Set<UUID>                    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  For each conversation (in chronological order):           │
│                                                              │
│  1. Get all messages for conversation                        │
│  2. Format as "Current Chunk"                                │
│  3. Call Agent with:                                         │
│     - Current Logbook (full)                                  │
│     - Current Chunk                                          │
│     - Task: "Append meaningful events"                        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Agent Response:                                            │
│    - New events to append (chronologically ordered)            │
│    - Or empty array (nothing to add)                         │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Merge:                                                     │
│    - logbook_so_far.timeline.append(new_events)             │
│    - Sort chronologically (newest first)                    │
│    - Truncate if over max_events (optional)                 │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
                        Repeat for all conversations
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Final Output:                                              │
│    - logbook_so_far (chronologically sorted)                  │
└─────────────────────────────────────────────────────────────┘
```

## Agent Prompt Template

```
You are building a project logbook by reviewing conversation history.

CURRENT LOGBOOK:
{logbook_text}

CURRENT CONVERSATION CHUNK:
{chunk_text}

TASK:
Append any meaningful events from this chunk to the logbook.

Rules:
1. Only add events NOT already captured in the current logbook
2. Event types: "decision" (choices made), "fact" (important information), "bug" (issues found)
3. Keep content concise (1 sentence, under 100 chars preferred)
4. Include rationale for decisions
5. List alternatives discussed for decisions
6. Include brief code ONLY if critical to understanding (1 line max)
7. Match format of existing logbook entries
8. Preserve timestamps from chunk messages
9. Skip trivial details, chitchat, or non-technical items

OUTPUT:
Return ONLY JSON in this exact format:
{
  "events": [
    {
      "timestamp": "2026-01-15T10:30:00Z",
      "type": "decision|fact|bug",
      "content": "Brief one-sentence description",
      "rationale": "Why (for decisions)",
      "alternatives": ["option1", "option2"],
      "code": "brief_code_snippet",
      "source": {"conversation_id": "uuid", "message_id": "uuid"}
    }
  ]
}

If this chunk adds nothing meaningful, return: {"events": []}
```

## Context Window Management

### Why This Works

| Factor | Token Count | Notes |
|--------|-------------|-------|
| Logbook (one-page) | ~1,000-2,000 | 20-30 events × ~50-70 tokens each |
| Chunk (conversation) | ~1,000-3,000 | Varies by conversation length |
| Instructions | ~300 | Fixed |
| Total | ~2,300-5,300 | Fits easily in 32k-128k context windows |

### Logbook Size Guarantees

```rust
struct LogbookConfig {
    max_events: usize,           // Default: 100 events
    max_event_tokens: usize,     // Default: 100 tokens per event
}

fn estimate_logbook_tokens(logbook: &LogbookTimeline) -> usize {
    // Header: ~100 tokens
    // Per event: ~50-70 tokens on average
    100 + (logbook.timeline.len() * 60)
}

// With default max_events=100:
// Max tokens: 100 + (100 * 60) = ~6,100 tokens
```

### Truncation Strategy (if needed)

```rust
fn truncate_logbook(logbook: &mut LogbookTimeline, max_events: usize) {
    if logbook.timeline.len() > max_events {
        // Keep most recent N events
        logbook.timeline.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        logbook.timeline.truncate(max_events);
    }
}
```

**Fallback modes:**
- If logbook grows too large, drop oldest events
- Alternatively, summarize older events into "Historical summary"
- Or split logbook by time periods (e.g., monthly logbooks)

## Implementation

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct AgentRequest {
    logbook_text: String,
    chunk_text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentResponse {
    events: Vec<TimelineEvent>,
}

async fn process_workspace(
    db: &Database,
    workspace: &str,
    llm_client: &LlmClient,
    config: &LogbookConfig,
) -> Result<LogbookTimeline> {
    // 1. Get conversations in chronological order
    let conversations = db
        .list_conversations(ListConversationsOptions {
            workspace: Some(workspace.to_string()),
            ..Default::default()
        })
        .await?;

    // Sort by created_at (oldest first for building chronologically)
    let mut conversations: Vec<_> = conversations
        .into_iter()
        .filter(|c| c.workspace.as_deref() == Some(workspace))
        .collect();
    conversations.sort_by_key(|c| c.created_at);

    // 2. Initialize empty logbook
    let mut logbook = LogbookTimeline {
        workspace: workspace.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        source_range: SourceRange {
            from: conversations.first().map(|c| c.created_at).unwrap_or_else(Utc::now),
            to: conversations.last().map(|c| c.created_at).unwrap_or_else(Utc::now),
        },
        timeline: Vec::new(),
    };

    // 3. Process each conversation
    for conversation in &conversations {
        let messages = db.get_messages(conversation.id).await?;

        if messages.is_empty() {
            continue;
        }

        // Format chunk
        let chunk_text = format_chunk(&conversation, &messages);

        // Format current logbook as text for context
        let logbook_text = format_logbook_text(&logbook);

        // Call agent
        let response = llm_client
            .call::<AgentResponse>("logbook_agent", &AgentRequest {
                logbook_text,
                chunk_text,
            })
            .await?;

        // Merge new events
        for event in response.events {
            logbook.timeline.push(event);
        }

        // Periodic truncation
        if logbook.timeline.len() > config.max_events {
            truncate_logbook(&mut logbook, config.max_events);
        }
    }

    // 4. Sort chronologically (newest first for display)
    logbook.timeline.sort_by_key(|e| std::cmp::Reverse(e.timestamp));

    Ok(logbook)
}

fn format_chunk(conversation: &Conversation, messages: &[Message]) -> String {
    let mut text = String::new();

    text.push_str(&format!("Conversation: {}\n", conversation.id));
    text.push_str(&format!("Title: {:?}\n", conversation.title));
    text.push_str(&format!("Created: {}\n", conversation.created_at.to_rfc3339()));
    text.push_str("\n---\n");

    for msg in messages {
        text.push_str(&format!("{}: {}\n", msg.role, msg.content));

        if let Some(created) = msg.created_at {
            text.push_str(&format!("(Time: {})\n", created.to_rfc3339()));
        }

        text.push('\n');
    }

    text
}

fn format_logbook_text(logbook: &LogbookTimeline) -> String {
    let mut text = String::new();

    text.push_str(&format!("=== Logbook: {} ===\n", logbook.workspace));
    text.push_str(&format!(
        "Generated: {}\n\n",
        logbook.generated_at.replace("T", " ").split('.').next().unwrap_or(&logbook.generated_at)
    ));

    // Sort chronologically (oldest first for context)
    let mut sorted_timeline = logbook.timeline.clone();
    sorted_timeline.sort_by_key(|e| e.timestamp);

    for event in &sorted_timeline {
        text.push_str(&format!(
            "{} | {} | {}\n",
            event.timestamp.split('T').next().unwrap_or("???"),
            event.type.to_uppercase(),
            event.content
        ));

        if let Some(rationale) = &event.rationale {
            text.push_str(&format!("  → Rationale: {}\n", rationale));
        }
        if let Some(code) = &event.code {
            text.push_str(&format!("  → Code: {}\n", code));
        }
        text.push('\n');
    }

    text
}
```

## Incremental Updates

To update an existing logbook:

```rust
async fn update_logbook(
    db: &Database,
    workspace: &str,
    existing_logbook: &LogbookTimeline,
    llm_client: &LlmClient,
    config: &LogbookConfig,
) -> Result<LogbookTimeline> {
    // Get conversations newer than existing logbook
    let conversations = db
        .list_conversations(ListConversationsOptions {
            workspace: Some(workspace.to_string()),
            after: Some(existing_logbook.source_range.to),
            ..Default::default()
        })
        .await?;

    let mut logbook = existing_logbook.clone();

    // Update source range
    logbook.source_range.to = conversations
        .last()
        .map(|c| c.created_at)
        .unwrap_or_else(Utc::now);

    // Process only new conversations
    for conversation in conversations {
        let messages = db.get_messages(conversation.id).await?;
        let chunk_text = format_chunk(&conversation, &messages);
        let logbook_text = format_logbook_text(&logbook);

        let response = llm_client
            .call::<AgentResponse>("logbook_agent", &AgentRequest {
                logbook_text,
                chunk_text,
            })
            .await?;

        logbook.timeline.extend(response.events);
    }

    // Sort and dedup
    logbook.timeline.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    logbook.timeline = deduplicate_timeline(logbook.timeline, 0.85);

    Ok(logbook)
}
```

## Parallel Processing (Advanced)

For large workspaces with many conversations:

```rust
use futures::stream::{self, StreamExt};

async fn process_workspace_parallel(
    db: &Database,
    workspace: &str,
    llm_client: &LlmClient,
    config: &LogbookConfig,
) -> Result<LogbookTimeline> {
    let conversations = db.list_conversations(ListConversationsOptions {
        workspace: Some(workspace.to_string()),
        ..Default::default()
    }).await?;

    // Process conversations in batches of 5
    let batch_size = 5;
    let mut logbook = LogbookTimeline {
        workspace: workspace.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        source_range: SourceRange {
            from: conversations.iter().map(|c| c.created_at).min().unwrap_or_else(Utc::now),
            to: conversations.iter().map(|c| c.created_at).max().unwrap_or_else(Utc::now),
        },
        timeline: Vec::new(),
    };

    let batches: Vec<_> = conversations
        .chunks(batch_size)
        .map(|batch| batch.to_vec())
        .collect();

    for batch in batches {
        let results = stream::iter(batch)
            .map(|conversation| {
                let db = db.clone();
                let llm_client = llm_client.clone();
                let current_logbook = logbook.clone();
                async move {
                    let messages = db.get_messages(conversation.id).await?;
                    let chunk_text = format_chunk(&conversation, &messages);
                    let logbook_text = format_logbook_text(&current_logbook);

                    let response = llm_client
                        .call::<AgentResponse>("logbook_agent", &AgentRequest {
                            logbook_text,
                            chunk_text,
                        })
                        .await?;

                    Ok::<_, anyhow::Error>(response.events)
                }
            })
            .buffer_unordered(batch_size)
            .collect::<Vec<_>>()
            .await;

        for result in results {
            if let Ok(events) = result {
                logbook.timeline.extend(events);
            }
        }
    }

    logbook.timeline.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    logbook.timeline = deduplicate_timeline(logbook.timeline, config.similarity_threshold);

    Ok(logbook)
}
```

## Error Handling

```rust
async fn process_workspace_with_retry(
    db: &Database,
    workspace: &str,
    llm_client: &LlmClient,
    config: &LogbookConfig,
) -> Result<LogbookTimeline> {
    let mut logbook = LogbookTimeline {
        workspace: workspace.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        source_range: SourceRange {
            from: Utc::now(),
            to: Utc::now(),
        },
        timeline: Vec::new(),
    };

    let conversations = db.list_conversations(ListConversationsOptions {
        workspace: Some(workspace.to_string()),
        ..Default::default()
    }).await?;

    let mut failed_conversations: Vec<Conversation> = Vec::new();

    for conversation in conversations {
        let messages = db.get_messages(conversation.id).await?;

        let result = llm_client
            .call::<AgentResponse>("logbook_agent", &AgentRequest {
                logbook_text: format_logbook_text(&logbook),
                chunk_text: format_chunk(&conversation, &messages),
            })
            .await;

        match result {
            Ok(response) => {
                logbook.timeline.extend(response.events);
            }
            Err(e) => {
                eprintln!("Failed to process conversation {}: {}", conversation.id, e);
                failed_conversations.push(conversation);

                // Continue with next conversation
            }
        }
    }

    if !failed_conversations.is_empty() {
        eprintln!("Warning: Failed to process {} conversations", failed_conversations.len());
    }

    logbook.timeline.sort_by_key(|e| std::cmp::Reverse(e.timestamp));

    Ok(logbook)
}
```

## CLI Interface

```bash
# Generate logbook (auto-manages context)
hstry logbook --workspace my-project

# Update existing logbook (incremental)
hstry logbook --workspace my-project --continue-from logbook.json

# Set max events
hstry logbook --workspace my-project --max-events 50

# Parallel processing (for large workspaces)
hstry logbook --workspace my-project --parallel

# Output formats
hstry logbook --workspace my-project --format text     # Human-readable
hstry logbook --workspace my-project --format json      # AI-consumable
hstry logbook --workspace my-project --format markdown  # Documentation
```

## Key Benefits

1. **Context is the logbook** - Agent always sees what's already captured
2. **Natural deduplication** - No separate deduplication step needed
3. **Simple prompt** - Agent just compares chunk to logbook
4. **Scalable** - Logbook stays small, chunks are swapped in/out
5. **Resilient** - If a chunk fails, continue with the rest
6. **Incremental** - Can update existing logbooks efficiently
