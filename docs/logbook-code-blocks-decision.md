# Code Blocks in Logbook - Design Decision

## Trade-offs

### Include Code Blocks

**Pros:**
- Provides concrete examples and implementation details
- Preserves actual code patterns used
- Helpful for understanding technical decisions (e.g., API signatures, configuration examples)
- Valuable reference when revisiting decisions later
- Shows "how" not just "what"

**Cons:**
- Makes the logbook much longer and harder to scan
- Code becomes outdated quickly as project evolves
- Increases LLM token usage and processing cost
- Clutters the concise summary nature of a logbook
- Can distract from high-level facts/decisions

### Exclude Code Blocks

**Pros:**
- More concise, skimmable logbook
- Focuses on facts/decisions, not implementation
- Less prone to becoming stale
- Cheaper/faster to process
- Better for high-level project history

**Cons:**
- Loses concrete examples and patterns
- Harder to reconstruct context without seeing code
- Might miss critical technical details
- Less useful as a reference guide

---

## Recommended Approaches

### Option 1: Configurable Inclusion (Recommended)

Make code block inclusion a user choice with smart defaults:

```toml
[logbook]
# How to handle code blocks in extraction
code_blocks = "context"  # options: "none", "context", "full", "summary"

# Maximum code block length to include (lines)
max_code_lines = 10

# Include code only in specific categories
code_in_categories = ["architecture", "api", "bug", "config"]
```

**Modes:**
- `none` - Strip all code blocks from input before extraction
- `context` - Include brief code snippets as "context" field for relevant facts (default)
- `full` - Include code blocks in extraction output (verbose)
- `summary` - LLM summarizes code blocks into 1-2 sentences instead of quoting

**CLI flags:**
```bash
# Strip all code
hstry logbook generate --code-blocks none

# Include brief code as context
hstry logbook generate --code-blocks context --max-code-lines 5

# Include full code blocks
hstry logbook generate --code-blocks full

# Summarize code instead of quoting
hstry logbook generate --code-blocks summary
```

**Implementation:**

```rust
enum CodeBlockHandling {
    None,        // Strip before LLM
    Context,     // Include brief snippets in context field
    Full,        // Include in extraction output
    Summary,     // LLM summarizes instead of quoting
}

struct ExtractionConfig {
    code_blocks: CodeBlockHandling,
    max_code_lines: usize,
    code_in_categories: HashSet<String>,
}

fn preprocess_messages(
    messages: Vec<Message>,
    config: &ExtractionConfig,
) -> String {
    let mut processed = String::new();

    for msg in messages {
        match config.code_blocks {
            CodeBlockHandling::None => {
                // Strip code blocks entirely
                let without_code = strip_code_blocks(&msg.content);
                processed.push_str(&format_message(msg, &without_code));
            }
            CodeBlockHandling::Context => {
                // Keep brief code in context, strip from main extraction
                let (content, code_snippets) = extract_code_snippets(&msg.content, config.max_code_lines);
                if !code_snippets.is_empty() {
                    processed.push_str(&format_message_with_context(
                        msg, &content, &code_snippets
                    ));
                } else {
                    processed.push_str(&format_message(msg, &content));
                }
            }
            CodeBlockHandling::Full => {
                // Include everything as-is
                processed.push_str(&format_message(msg, &msg.content));
            }
            CodeBlockHandling::Summary => {
                // Keep code, but instruct LLM to summarize
                processed.push_str(&format_message(msg, &msg.content));
            }
        }
    }

    processed
}
```

### Option 2: Smart Inclusion Based on Category

Only include code blocks when they add value to specific categories:

```json
{
  "facts": [
    {
      "statement": "API authentication uses JWT with RS256 signing",
      "category": "api",
      "code_example": {
        "language": "rust",
        "snippet": "let token = jwt::encode(&header, &claims, &encoding_key)?;",
        "lines": 1
      },
      "source_timestamp": "2026-02-01T10:00:00Z"
    }
  ]
}
```

**Rules for inclusion:**
- `architecture`: Include structural code examples (e.g., module definitions, trait bounds)
- `api`: Include function signatures or usage examples
- `bug`: Include minimal reproduction if mentioned
- `config`: Include config file snippets
- `other`: Exclude by default

### Option 3: Inline Code Snippets Only

Include only single-line or brief inline code (marked with backticks), exclude multi-line blocks:

```
Instead of quoting:
```rust
async fn process_request(req: Request) -> Result<Response> {
    let user = authenticate(req.headers())?;
    let data = fetch_data(req.body()).await?;
    Ok(Response::new(data))
}
```

Extract as: "Use `async fn process_request` with `authenticate()` and `fetch_data()`"
```

**Implementation:**
```rust
fn summarize_code_blocks(content: &str) -> String {
    // Extract inline code (backtick-wrapped)
    let inline_code: Vec<&str> = Regex::new(r"`([^`]+)`")
        .unwrap()
        .find_iter(content)
        .map(|m| m.as_str())
        .collect();

    // For multi-line blocks, extract function/class names and key patterns
    let multi_line_code: Vec<&str> = Regex::new(r"```[\w]*\n([\s\S]*?)```")
        .unwrap()
        .find_iter(content)
        .map(|m| m.as_str())
        .collect();

    let mut summary = content.clone();

    // Replace multi-line blocks with function references
    for block in multi_line_code {
        if let Some(summary_text) = extract_function_signatures(block) {
            summary = summary.replace(block, &summary_text);
        }
    }

    summary
}

fn extract_function_signatures(code_block: &str) -> Option<String> {
    // Extract "fn name(...)", "class Name", "def name(...)" patterns
    let patterns = vec![
        r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\([^)]*\)",
        r"class\s+(\w+)",
        r"def\s+(\w+)\s*\([^)]*\):",
    ];

    let mut signatures = Vec::new();

    for pattern in &patterns {
        let re = Regex::new(pattern).unwrap();
        for caps in re.captures_iter(code_block) {
            if let Some(name) = caps.get(1) {
                signatures.push(format!("`{}`", name.as_str()));
            }
        }
    }

    if signatures.is_empty() {
        None
    } else {
        Some(signatures.join(", "))
    }
}
```

### Option 4: Separate "Code Reference" Section

Keep logbook concise, but link back to original conversations or store code snippets separately:

**In logbook:**
```json
{
  "facts": [
    {
      "statement": "Connection pool configured with max 10 connections",
      "category": "config",
      "code_references": [
        {
          "conversation_id": "uuid-1",
          "message_id": "uuid-2",
          "lines": [5, 6, 7]
        }
      ],
      "source_timestamp": "2026-02-01T10:00:00Z"
    }
  ]
}
```

**Separate code reference file:**
```json
{
  "conversation_id": "uuid-1",
  "message_id": "uuid-2",
  "code_blocks": [
    {
      "language": "rust",
      "start_line": 5,
      "end_line": 7,
      "content": "let pool = PgPoolOptions::new()\n    .max_connections(10)\n    .connect(&database_url).await?;"
    }
  ]
}
```

---

## My Recommendation

**Use Option 1 (Configurable) with "context" mode as default:**

1. **Default behavior:** Include brief code snippets (max 5-10 lines) as "context" for architecture, API, bug, and config facts. Strip from other categories.

2. **User can override:**
   - `--code-blocks none` for completely concise logbook
   - `--code-blocks summary` to have LLM summarize code instead of quoting
   - `--code-blocks full` to include everything (verbose mode)

3. **Rationale:**
   - Provides concrete examples where they add value
   - Keeps logbook concise by default
   - Gives users flexibility based on their needs
   - Code in "context" field is clearly separated from core facts

4. **Example output with context mode:**

```json
{
  "facts": [
    {
      "statement": "JWT authentication with RS256 signing",
      "category": "api",
      "confidence": 1.0,
      "related_entities": ["jsonwebtoken crate"],
      "context": "Sign tokens with `jwt::encode(&header, &claims, &key)` using RS256",
      "source_timestamp": "2026-02-01T10:00:00Z"
    }
  ]
}
```

This balances conciseness with the value of concrete examples. Users wanting a truly minimal logbook can use `--code-blocks none`, while those wanting full documentation can use `--code-blocks full`.

---

## Updated JSON Schema with Context Field

```json
{
  "properties": {
    "facts": {
      "items": {
        "properties": {
          "statement": { "type": "string" },
          "category": { "type": "string" },
          "confidence": { "type": "number" },
          "related_entities": { "items": {"type": "string"}, "type": "array" },
          "context": {
            "type": "string",
            "description": "Brief context or code snippet (only included if --code-blocks context)"
          },
          "source_timestamp": { "format": "date-time", "type": "string" }
        },
        "required": ["statement", "category", "confidence", "source_timestamp"],
        "type": "object"
      },
      "type": "array"
    }
  }
}
```

The `context` field is optional and only populated when `--code-blocks context` or `--code-blocks full` is specified.
