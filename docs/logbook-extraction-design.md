# Logbook Extraction AI Loop Design

## Overview

Extract concise facts and decisions from hstry conversation history using LLM-based chunked processing with embedding-based deduplication.

## Architecture

```
┌─────────────┐
│  Workspace  │
└──────┬──────┘
       │
       ▼
┌──────────────────┐
│ List Conversations│ (newest → oldest)
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  For each conv: │
│  Get Messages    │ (ordered)
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  Chunk Messages  │ (configurable size)
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  LLM Extraction  │ (with JSON schema)
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  Deduplicate     │ (embedding similarity)
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  Merge Logbook   │
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│  Output Logbook  │ (JSON/Markdown)
└──────────────────┘
```

## JSON Schema for Extraction

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Logbook Extraction",
  "type": "object",
  "properties": {
    "facts": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "statement": {
            "type": "string",
            "description": "Concise factual statement extracted from conversation"
          },
          "category": {
            "type": "string",
            "enum": ["architecture", "bug", "feature", "config", "performance", "api", "database", "security", "testing", "deployment", "other"],
            "description": "Category of the fact"
          },
          "confidence": {
            "type": "number",
            "minimum": 0,
            "maximum": 1,
            "description": "Confidence in this being a factual statement (0-1)"
          },
          "related_entities": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Files, components, or systems mentioned"
          },
          "context": {
            "type": "string",
            "description": "Brief context for the fact"
          },
          "source_timestamp": {
            "type": "string",
            "format": "date-time",
            "description": "ISO8601 timestamp from the conversation"
          }
        },
        "required": ["statement", "category", "confidence", "source_timestamp"]
      }
    },
    "decisions": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "description": {
            "type": "string",
            "description": "What was decided"
          },
          "rationale": {
            "type": "string",
            "description": "Why this decision was made"
          },
          "alternatives_considered": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Other options that were discussed"
          },
          "impact": {
            "type": "string",
            "enum": ["high", "medium", "low"],
            "description": "Impact level of this decision"
          },
          "category": {
            "type": "string",
            "enum": ["technical", "product", "process", "infrastructure", "team", "other"],
            "description": "Type of decision"
          },
          "source_timestamp": {
            "type": "string",
            "format": "date-time",
            "description": "ISO8601 timestamp from the conversation"
          }
        },
        "required": ["description", "rationale", "impact", "category", "source_timestamp"]
      }
    }
  }
}
```

## Extraction Prompt Template

```
You are analyzing a chat history to extract a concise logbook of facts and decisions.

Analyze the following conversation messages and extract:
1. **Facts**: Concrete information learned, discoveries, or clarifications
2. **Decisions**: explicit choices made with rationale

Guidelines:
- Only extract high-value, actionable facts (not trivial details)
- Focus on technical decisions, architecture choices, bugs discovered, key learnings
- Keep statements concise (1-2 sentences max)
- Assign confidence scores: 1.0 for certain facts, 0.7-0.9 for inferred but likely
- For decisions, capture the rationale and any alternatives discussed
- Use appropriate categories
- Preserve the source_timestamp for temporal ordering

Output ONLY valid JSON matching the provided schema.

Conversation messages:
{messages}

Extracted logbook:
```

## Deduplication Strategy

### Embedding Generation

For each extracted fact/decision:
1. Create embedding of `statement` (facts) or `description + rationale` (decisions)
2. Store with the extracted item

### Similarity Check

```rust
struct DedupState {
    facts: Vec<FactWithEmbedding>,
    decisions: Vec<DecisionWithEmbedding>,
}

impl DedupState {
    fn should_add_fact(&mut self, fact: ExtractedFact, embedding: Vec<f32>) -> bool {
        let threshold = 0.85; // Cosine similarity threshold

        for existing in &self.facts {
            let similarity = cosine_similarity(&embedding, &existing.embedding);
            if similarity > threshold {
                // Duplicate detected - skip or merge
                return false;
            }
        }

        self.facts.push(FactWithEmbedding {
            fact,
            embedding,
        });
        true
    }

    fn merge_decision(&mut self, decision: ExtractedDecision, embedding: Vec<f32>) {
        // Find similar existing decisions
        let mut similar = Vec::new();

        for existing in &self.decisions {
            let similarity = cosine_similarity(&embedding, &existing.embedding);
            if similarity > 0.85 {
                similar.push(existing);
            }
        }

        if similar.is_empty() {
            // No duplicates - add as-is
            self.decisions.push(DecisionWithEmbedding {
                decision,
                embedding,
            });
        } else if similar.len() == 1 {
            // Merge with single similar decision
            let existing = similar[0];
            if decision.source_timestamp > existing.decision.source_timestamp {
                // Keep the more recent one, but combine rationales if different
                if decision.rationale != existing.decision.rationale {
                    existing.decision.rationale = format!(
                        "{}; Later: {}",
                        existing.decision.rationale, decision.rationale
                    );
                }
                // Update alternatives
                for alt in decision.alternatives_considered {
                    if !existing.decision.alternatives_considered.contains(&alt) {
                        existing.decision.alternatives_considered.push(alt);
                    }
                }
            }
        } else {
            // Multiple matches - merge into the first one
            let primary = &mut self.decisions[0];
            for other in similar.iter().skip(1) {
                if !primary.decision.alternatives_considered.contains(&other.decision.description) {
                    primary.decision.alternatives_considered.push(other.decision.description);
                }
            }
        }
    }
}
```

## Chunking Strategies

### Option 1: Per-Conversation Chunking
```rust
// Process each conversation as a chunk
for conversation in conversations {
    let messages = db.get_messages(conversation.id).await?;
    let chunk = Chunk {
        conversation_id: conversation.id,
        timestamp: conversation.updated_at.unwrap_or(conversation.created_at),
        messages,
    };
    process_chunk(chunk, &mut dedup_state, &llm_client).await?;
}
```

### Option 2: Fixed-Size Message Chunking
```rust
// Chunk messages into groups of N (e.g., 20 messages)
let chunk_size = 20;
let mut current_chunk = Vec::new();
let mut chunk_timestamp = None;

for conversation in conversations {
    let messages = db.get_messages(conversation.id).await?;

    for message in messages {
        current_chunk.push(message);
        chunk_timestamp = chunk_timestamp.max(message.created_at);

        if current_chunk.len() >= chunk_size {
            process_chunk(
                Chunk {
                    conversation_id: conversation.id,
                    timestamp: chunk_timestamp.unwrap(),
                    messages: current_chunk,
                },
                &mut dedup_state,
                &llm_client,
            ).await?;
            current_chunk = Vec::new();
            chunk_timestamp = None;
        }
    }
}
```

### Option 3: Token-Based Chunking
```rust
// Chunk based on approximate token count
let max_tokens = 4000;
let mut current_chunk = Vec::new();
let mut current_tokens = 0;

for conversation in conversations {
    let messages = db.get_messages(conversation.id).await?;

    for message in messages {
        let message_tokens = estimate_tokens(&message.content);

        if current_tokens + message_tokens > max_tokens && !current_chunk.is_empty() {
            process_chunk(
                Chunk { /* ... */ },
                &mut dedup_state,
                &llm_client,
            ).await?;
            current_chunk = Vec::new();
            current_tokens = 0;
        }

        current_chunk.push(message);
        current_tokens += message_tokens;
    }
}
```

## Integration with hstry

### New CLI Command

```rust
enum LogbookCommand {
    /// Generate a logbook from conversation history
    Generate {
        /// Workspace to generate logbook for
        #[arg(long)]
        workspace: String,

        /// Output format (json, markdown)
        #[arg(long, default_value = "json")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<PathBuf>,

        /// Chunking strategy (conversation, messages, tokens)
        #[arg(long, default_value = "conversation")]
        chunk_strategy: String,

        /// Chunk size (for messages/tokens strategies)
        #[arg(long, default_value = "20")]
        chunk_size: usize,

        /// Deduplication threshold (0.0-1.0)
        #[arg(long, default_value = "0.85")]
        similarity_threshold: f32,

        /// LLM model to use for extraction
        #[arg(long, default_value = "claude-3-5-sonnet-20241022")]
        model: String,

        /// Continue from previous logbook (incremental updates)
        #[arg(long)]
        continue_from: Option<PathBuf>,
    },
}
```

### Configuration

```toml
[logbook]
# LLM provider
provider = "anthropic"  # or "openai", "ollama", etc.

# Embedding provider for deduplication
embedding_provider = "openai"
embedding_model = "text-embedding-3-small"

# Default chunking
default_chunk_strategy = "conversation"
default_chunk_size = 20
max_tokens_per_chunk = 4000

# Deduplication
similarity_threshold = 0.85
enable_deduplication = true

# Categories to include
include_categories = ["architecture", "bug", "feature", "config", "performance", "api", "database", "security", "testing", "deployment", "other"]
exclude_categories = []

# Minimum confidence score
min_confidence = 0.7
```

## Output Formats

### JSON Output
```json
{
  "workspace": "my-project",
  "generated_at": "2026-02-02T13:11:53Z",
  "source_range": {
    "from": "2026-01-01T00:00:00Z",
    "to": "2026-02-02T13:11:53Z"
  },
  "facts": [
    {
      "statement": "The project uses Rust with Tokio for async runtime",
      "category": "architecture",
      "confidence": 1.0,
      "related_entities": ["Rust", "Tokio"],
      "context": "Initial project setup discussion",
      "source_timestamp": "2026-01-15T10:30:00Z"
    }
  ],
  "decisions": [
    {
      "description": "Use PostgreSQL as the primary database",
      "rationale": "Need ACID transactions and JSONB support",
      "alternatives_considered": ["MySQL", "MongoDB"],
      "impact": "high",
      "category": "technical",
      "source_timestamp": "2026-01-16T14:20:00Z"
    }
  ]
}
```

### Markdown Output
```markdown
# Project Logbook

**Workspace:** my-project
**Generated:** 2026-02-02 13:11:53 UTC
**Source Range:** 2026-01-01 → 2026-02-02

---

## Facts

### Architecture
- [2026-01-15] The project uses Rust with Tokio for async runtime
- [2026-01-20] Event-driven architecture with message queue (Redis Streams)

### Bugs
- [2026-01-28] Memory leak in connection pool (fixed in v0.2.1)

### Features
- [2026-01-22] Added WebSocket support for real-time updates

---

## Decisions

### Technical
- [2026-01-16] **Use PostgreSQL as the primary database**
  - Rationale: Need ACID transactions and JSONB support
  - Alternatives considered: MySQL, MongoDB
  - Impact: High

- [2026-01-25] **Deploy with Kubernetes on AWS EKS**
  - Rationale: Team familiarity and managed service benefits
  - Alternatives considered: Docker Compose, AWS ECS
  - Impact: Medium
```

## Implementation Plan

1. **Create `hstry-logbook` crate** with:
   - `extraction.rs` - LLM extraction logic
   - `dedup.rs` - Embedding-based deduplication
   - `chunk.rs` - Chunking strategies
   - `schema.rs` - JSON schemas
   - `output.rs` - Format rendering

2. **Integrate with CLI** via new `LogbookCommand`

3. **Add to `hstry-core`**:
   - Optional: Store generated logbooks in database
   - Query for logbook regeneration (incremental updates)

4. **Testing**:
   - Unit tests for chunking
   - Integration tests with mock LLM
   - End-to-end tests on sample conversations

## Example Usage

```bash
# Generate logbook for a workspace
hstry logbook generate --workspace my-project --format markdown --output logbook.md

# With custom chunking and deduplication
hstry logbook generate \
  --workspace my-project \
  --chunk-strategy messages \
  --chunk-size 30 \
  --similarity-threshold 0.9 \
  --model claude-3-5-sonnet-20241022

# Incremental update from previous logbook
hstry logbook generate \
  --workspace my-project \
  --continue-from logbook.json \
  --output logbook-updated.json

# Generate JSON for further processing
hstry logbook generate --workspace my-project --format json | jq '.facts | length'
```

## Performance Considerations

- **Parallel chunk processing**: Extract from multiple chunks concurrently
- **Caching**: Cache embeddings to avoid re-generating on reruns
- **Incremental updates**: Track processed conversations, only process new ones
- **Batch API calls**: Send multiple chunks in single request if API supports it
- **Streaming**: Stream results as they're processed for large workspaces

## Error Handling

- **LLM failures**: Log and skip chunk, continue with next
- **Invalid JSON**: Retry with simpler prompt or fallback to extraction
- **Rate limits**: Exponential backoff, retry failed chunks
- **Embedding failures**: Skip deduplication, keep all items with warning

## Future Enhancements

- **Summary generation**: Add overall project summary at logbook top
- **Trend analysis**: Track fact categories over time
- **Decision graph**: Visualize dependency between decisions
- **Action items**: Extract TODOs and follow-up tasks
- **Confidence filtering**: Filter output by minimum confidence
- **Custom schemas**: User-provided JSON schemas for custom extraction
