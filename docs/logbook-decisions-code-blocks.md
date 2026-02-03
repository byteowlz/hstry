# Code Blocks in Decisions - Design

## Why Decisions Need Different Treatment

Decisions often have **code as evidence** or **code as the decision itself**:

- "Use `async fn` with `.await` for async operations" - code IS the decision
- "Implement pagination with `LIMIT/OFFSET`" - shows the pattern chosen
- "Error handling via `Result<T, E>` throughout" - the decision is about code style
- "Cache using `Redis::get()` / `Redis::set_ex()`" - shows implementation approach

These code examples are often **more important** than in facts, because:
1. They demonstrate the exact pattern chosen
2. They show the concrete outcome of the decision
3. They're reference for future developers
4. They distinguish between similar alternatives (e.g., `Option` vs `Result`)

---

## Recommended Approach: Code Evidence Field

Add an optional `code_evidence` field to decisions:

```json
{
  "decisions": [{
    "description": "Use async/await pattern for all database operations",
    "rationale": "Allows concurrent operations without blocking threads",
    "alternatives_considered": ["Callbacks", "Futures directly"],
    "impact": "high",
    "category": "technical",
    "code_evidence": {
      "snippet": "async fn fetch_user(id: i32) -> Result<User> {\n    sqlx::query_as!(User, \"SELECT * FROM users WHERE id = ?\", id)\n        .fetch_one(&pool).await\n}",
      "language": "rust",
      "purpose": "Example of the chosen async pattern"
    },
    "source_timestamp": "2026-02-01T10:00:00Z"
  }]
}
```

## When to Include Code Evidence

### Always include (by default):
- **Pattern decisions** (async/sync, error handling styles, state management)
- **API signature decisions** (function shapes, return types)
- **Configuration decisions** (env vars, config file format)
- **Library usage decisions** (which methods to use from a chosen library)

### Include only if requested (`--code-blocks full`):
- **Large implementation chunks** (full function bodies > 10 lines)
- **Setup/boilerplate code** (imports, struct definitions)
- **Example usage** (unless it's the minimal demonstration)

### Never include (strip):
- **Contextual code** (unrelated to the decision)
- **Debug code** (print statements, temporary fixes)
- **Copied error messages** (unless relevant to the decision)

---

## Updated JSON Schema

```json
{
  "decisions": {
    "type": "array",
    "items": {
      "type": "object",
      "properties": {
        "description": {
          "type": "string",
          "description": "What was decided (concise)"
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
          "enum": ["technical", "product", "process", "infrastructure", "team", "other"]
        },
        "code_evidence": {
          "type": "object",
          "properties": {
            "snippet": {
              "type": "string",
              "description": "Code demonstrating the decision"
            },
            "language": {
              "type": "string",
              "description": "Programming language (rust, typescript, python, etc.)"
            },
            "purpose": {
              "type": "string",
              "description": "What this code demonstrates"
            },
            "context": {
              "type": "string",
              "description": "Where this code would be used (optional)"
            }
          },
          "required": ["snippet", "language", "purpose"]
        },
        "source_timestamp": {
          "format": "date-time",
          "type": "string"
        }
      },
      "required": ["description", "rationale", "impact", "category", "source_timestamp"]
    }
  }
}
```

---

## LLM Prompt for Decision Extraction

```
You are analyzing a chat history to extract decisions and their technical evidence.

For each decision extracted:
1. Provide a concise description of what was decided
2. Explain the rationale
3. List alternatives that were discussed
4. Include code evidence if the decision involves a specific pattern or implementation

**Including Code Evidence:**
Only include code_evidence when the decision directly involves code patterns:
- API signatures or function shapes decided
- Architectural patterns (async/sync, state management, error handling)
- Configuration approaches
- Specific library methods or patterns chosen

Keep code snippets:
- Minimal (3-10 lines max)
- Focused on the decision point
- Without boilerplate or context

**Excluding Code Evidence:**
Don't include code evidence for:
- Non-technical decisions (product, process, team)
- High-level choices without specific patterns
- Large implementation details
- Contextual/unrelated code

Output ONLY valid JSON matching the schema.

Conversation messages:
{messages}

Extracted decisions:
```

---

## Code Block Modes for Decisions

### Mode: `none` (Minimal)
Strip all code from decisions:

```json
{
  "description": "Use Result<T, E> for error handling",
  "rationale": "Explicit error handling, better than panics",
  "alternatives_considered": ["Panic on error", "Option for errors"],
  "impact": "high",
  "category": "technical"
}
```

### Mode: `context` (Default - Recommended)
Brief inline code in description, `code_evidence` for patterns:

```json
{
  "description": "Use `Result<T, E>` for all fallible operations",
  "rationale": "Explicit error handling, better than panics",
  "alternatives_considered": ["Panic on error", "Option for errors"],
  "impact": "high",
  "category": "technical",
  "code_evidence": {
    "snippet": "fn parse_config(path: &str) -> Result<Config, ConfigError>",
    "language": "rust",
    "purpose": "Example signature pattern for Result return type"
  }
}
```

### Mode: `summary`
LLM summarizes the code into description:

```json
{
  "description": "Use async/await pattern with `Result<T, E>` for database operations",
  "rationale": "Allows concurrent non-blocking operations with proper error handling",
  "code_evidence": null
}
```

### Mode: `full`
Include full code blocks (verbose):

```json
{
  "description": "Use async/await pattern for database operations",
  "rationale": "Allows concurrent non-blocking operations",
  "code_evidence": {
    "snippet": "async fn fetch_user(id: i32) -> Result<User> {\n    sqlx::query_as!(User, \"SELECT * FROM users WHERE id = ?\", id)\n        .fetch_one(&pool).await\n        .map_err(|e| anyhow!(\"Failed to fetch user {}: {}\", id, e))\n}\n\nasync fn create_user(user: &User) -> Result<i32> {\n    sqlx::query!(\"INSERT INTO users (name, email) VALUES (?, ?)\", user.name, user.email)\n        .execute(&pool).await\n        .map(|r| r.last_insert_rowid())\n        .map_err(|e| anyhow!(\"Failed to create user: {}\", e))\n}",
    "language": "rust",
    "purpose": "Full implementation examples of async/await pattern"
  }
}
```

---

## Configuration for Decision Code Evidence

```toml
[logbook]
code_blocks = "context"  # none, context, summary, full

# Decision-specific settings
[logbook.decisions]
max_code_lines = 10
include_code_evidence = ["technical", "infrastructure"]  # categories that get code
require_code_for = ["pattern", "api", "config"]  # decision types requiring code
```

---

## Implementation: Detecting Code-Related Decisions

```rust
enum DecisionType {
    Pattern,      // Code pattern decision (requires code)
    Api,          // API shape decision (requires code)
    Config,       // Configuration decision (requires code)
    Library,      // Library choice (code optional)
    Architecture, // Architecture choice (code optional)
    Product,      // Non-technical (no code)
    Process,      // Process/organizational (no code)
}

impl Decision {
    fn should_include_code_evidence(&self, config: &ExtractionConfig) -> bool {
        if config.code_blocks == CodeBlockHandling::None {
            return false;
        }

        let decision_type = self.classify_type();

        match config.code_blocks {
            CodeBlockHandling::Context | CodeBlockHandling::Summary => {
                // Include for technical decisions with patterns
                matches!(
                    decision_type,
                    DecisionType::Pattern | DecisionType::Api | DecisionType::Config
                )
            }
            CodeBlockHandling::Full => {
                // Include for all technical categories
                matches!(
                    decision_type,
                    DecisionType::Pattern
                        | DecisionType::Api
                        | DecisionType::Config
                        | DecisionType::Library
                        | DecisionType::Architecture
                )
            }
            CodeBlockHandling::None => false,
        }
    }

    fn classify_type(&self) -> DecisionType {
        let desc_lower = self.description.to_lowercase();
        let rationale_lower = self.rationale.to_lowercase();

        if desc_lower.contains("fn ") || desc_lower.contains("async ") || desc_lower.contains("struct ") {
            DecisionType::Pattern
        } else if desc_lower.contains("api") || desc_lower.contains("endpoint") || desc_lower.contains("signature") {
            DecisionType::Api
        } else if desc_lower.contains("config") || desc_lower.contains("env ") || desc_lower.contains("settings") {
            DecisionType::Config
        } else if desc_lower.contains("use ") || desc_lower.contains("adopt ") || desc_lower.contains("switch to ") {
            DecisionType::Library
        } else if self.category == "technical" || self.category == "infrastructure" {
            DecisionType::Architecture
        } else {
            DecisionType::Product
        }
    }
}
```

---

## Output Examples by Mode

### None Mode (Truly Minimal)
```markdown
# Decisions

## Technical

- **Use Result<T, E> for error handling**
  - Rationale: Explicit error handling, better than panics
  - Alternatives: Panic on error, Option for errors
  - Impact: High

- **Adopt PostgreSQL as database**
  - Rationale: Need ACID transactions
  - Alternatives: MySQL, MongoDB
  - Impact: High
```

### Context Mode (Default)
```markdown
# Decisions

## Technical

- **Use `Result<T, E>` for all fallible operations**
  - Rationale: Explicit error handling, better than panics
  - Alternatives: Panic on error, Option for errors
  - Impact: High
  - Code: `fn parse_config(path: &str) -> Result<Config, ConfigError>`

- **Adopt PostgreSQL with SQLx**
  - Rationale: Type-safe queries, compile-time checked
  - Alternatives: Diesel ORM, raw PQ library
  - Impact: High
  - Code: `sqlx::query_as!(User, "SELECT * FROM users WHERE id = ?", id)`

## Infrastructure

- **Deploy with Kubernetes on AWS EKS**
  - Rationale: Team familiarity, managed service
  - Impact: Medium
```

### Full Mode (Verbose)
```markdown
# Decisions

## Technical

- **Use async/await pattern for database operations**
  - Rationale: Allows concurrent non-blocking operations
  - Alternatives: Blocking calls, callback-based async
  - Impact: High

  ```rust
  async fn fetch_user(id: i32) -> Result<User> {
      sqlx::query_as!(User, "SELECT * FROM users WHERE id = ?", id)
          .fetch_one(&pool).await
          .map_err(|e| anyhow!("Failed to fetch user {}: {}", id, e))
  }
  ```
```

---

## Deduplication Considerations

When deduplicating decisions with code evidence:

1. **Primary comparison**: Use `description + rationale` text for embedding
2. **Code comparison**: If descriptions are similar but code differs, keep both
3. **Merging**: When merging similar decisions, combine code evidence if different

```rust
fn merge_decisions(existing: &mut Decision, new: Decision, similarity: f32) {
    // If descriptions are identical but code evidence differs, keep both code examples
    if existing.description == new.description && similarity > 0.95 {
        if let (Some(existing_code), Some(new_code)) = (&mut existing.code_evidence, new.code_evidence) {
            // Merge code snippets
            if !existing_code.snippet.contains(&new_code.snippet) {
                existing_code.snippet.push_str(&format!("\n\n// Alternative:\n{}", new_code.snippet));
            }
        }
    }
    // ... rest of merge logic
}
```

---

## Summary: Decision Code Handling

| Mode | Code Evidence | When to Use |
|------|---------------|-------------|
| `none` | No code at all | Minimal logbook, high-level overview |
| `context` | Brief snippets (3-10 lines) for pattern/API decisions | **Default** - balanced reference |
| `summary` | LLM summarizes code into description | When you want the pattern but not the code |
| `full` | Full code blocks | Detailed reference documentation |

**Recommendation:** Use `context` as default. It provides enough evidence to understand *what* was decided without cluttering the logbook with boilerplate. For decisions where code IS the decision (patterns, APIs), the brief snippet is essential context.
