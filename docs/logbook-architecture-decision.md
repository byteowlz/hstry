# Where to Build Logbook Functionality?

## Options

### Option 1: In hstry directly
Add logbook generation as a core hstry feature.

### Option 2: In mmry (byt memory)
Store logbook as structured memories.

### Option 3: In pi-mono
Build as a separate tool that interfaces with hstry.

---

## Analysis

### Option 1: In hstry ✓ RECOMMENDED

**Pros:**
- Direct database access - no API overhead
- Natural fit: logbook is derived FROM conversation data
- Already has conversation traversal logic
- Single codebase, no external dependencies
- Can leverage existing CLI structure
- Simple: `hstry logbook --workspace my-project`

**Cons:**
- Couples hstry to LLM providers (but this is inevitable)
- Adds complexity to hstry core

**Verdict:** ✅ Best option

---

### Option 2: In mmry (byt memory)

**Pros:**
- mmry is designed for project knowledge
- Agents already query mmry for context
- Cross-repo memory support

**Cons:**
- mmry stores unstructured memories, not structured timelines
- Logbook is a *derived view*, not a memory
- mmry lacks chronology (sorted by importance, not date)
- Would require new structured memory type
- No direct conversation data access
- Would need to pull from hstry anyway

**Verdict:** ❌ Mismatch - mmry is for persistent knowledge, not derived summaries

---

### Option 3: In pi-mono

**Pros:**
- pi is the agent framework, natural place for agent tools
- Keeps hstry as pure storage layer
- Modular - independent of hstry version
- Can work with multiple data sources

**Cons:**
- Needs to reimplement conversation traversal
- Lacks direct DB access - slower API calls
- Redundant data fetching logic
- Integration complexity
- Duplicate logic (what if hstry adds features?)

**Verdict:** ❌ Overcomplicated for this use case

---

## Recommended Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      hstry                            │
│  ┌─────────────────────────────────────────────────┐   │
│  │  Core (existing)                              │   │
│  │    - DB: conversations, messages              │   │
│  │    - CLI: sync, search, import, export        │   │
│  └─────────────────────────────────────────────────┘   │
│                                                         │
│  ┌─────────────────────────────────────────────────┐   │
│  │  Logbook Module (NEW)                        │   │
│  │    - logbook generation (LLM-based)          │   │
│  │    - incremental updates                      │   │
│  │    - CLI command: `hstry logbook`            │   │
│  │    - Output: text/JSON/markdown             │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                        │
                        │ CLI or JSON API
                        ▼
┌─────────────────────────────────────────────────────────┐
│                      pi-mono                          │
│  ┌─────────────────────────────────────────────────┐   │
│  │  Agent Tools                                 │   │
│  │    - Tool: GetProjectLogbook(workspace)         │   │
│  │    - Implementation: calls hstry CLI           │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────┐
│                   AI Agents                           │
│  - Call tool to get logbook                           │
│  - Use as context for decisions                      │
└─────────────────────────────────────────────────────────┘
```

---

## Detailed Design: In hstry

### New Crate: `hstry-logbook`

```rust
// crates/hstry-logbook/Cargo.toml
[package]
name = "hstry-logbook"
version = "0.1.0"

[dependencies]
hstry-core = { path = "../hstry-core" }
tokio = { workspace = true }
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

# LLM clients (optional features)
anthropic = { version = "0.1", optional = true }
openai = { version = "0.1", optional = true }
ollama = { version = "0.1", optional = true }
```

### CLI Integration

```rust
// crates/hstry-cli/src/main.rs

enum Command {
    // ... existing commands ...
    Logbook {
        /// Workspace to generate logbook for
        #[arg(long)]
        workspace: String,

        /// Output format (text, json, markdown)
        #[arg(long, default_value = "text")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long)]
        output: Option<PathBuf>,

        /// LLM provider to use
        #[arg(long, default_value = "anthropic")]
        provider: String,

        /// LLM model
        #[arg(long, default_value = "claude-3-5-sonnet-20241022")]
        model: String,

        /// Include code snippets
        #[arg(long)]
        include_code: bool,

        /// Continue from existing logbook
        #[arg(long)]
        continue_from: Option<PathBuf>,

        /// Max events in logbook (0 = unlimited)
        #[arg(long, default_value = "0")]
        max_events: usize,

        /// Process conversations in parallel
        #[arg(long)]
        parallel: bool,
    },
}

async fn handle_logbook(cmd: LogbookCommand, config: &Config) -> Result<()> {
    let db = Database::open(&config.db_path).await?;
    let llm_client = create_llm_client(&cmd.provider, &cmd.model, config)?;

    let mut generator = LogbookGenerator::new(db, llm_client, LogbookConfig {
        max_events: if cmd.max_events == 0 { usize::MAX } else { cmd.max_events },
        include_code: cmd.include_code,
        parallel: cmd.parallel,
    });

    let logbook = if let Some(existing_path) = cmd.continue_from {
        // Incremental update
        let existing = read_logbook(&existing_path)?;
        generator.update(&cmd.workspace, &existing).await?
    } else {
        // Full generation
        generator.generate(&cmd.workspace).await?
    };

    match cmd.format.as_str() {
        "json" => write_json(&logbook, cmd.output)?,
        "markdown" => write_markdown(&logbook, cmd.output)?,
        _ => write_text(&logbook, cmd.output)?,
    }

    Ok(())
}
```

### LLM Client Abstraction

```rust
// crates/hstry-logbook/src/llm.rs

pub trait LlmClient: Send + Sync {
    async fn call<T: DeserializeOwned>(
        &self,
        prompt: &str,
        input: &impl Serialize,
    ) -> Result<T>;
}

pub struct AnthropicClient {
    client: anthropic::Client,
    model: String,
}

impl LlmClient for AnthropicClient {
    async fn call<T: DeserializeOwned>(
        &self,
        prompt: &str,
        input: &impl Serialize,
    ) -> Result<T> {
        // Implementation
    }
}

// Similar for OpenAI, Ollama, etc.

pub fn create_llm_client(
    provider: &str,
    model: &str,
    config: &Config,
) -> Result<Box<dyn LlmClient>> {
    match provider {
        "anthropic" => Ok(Box::new(AnthropicClient::new(model, config)?)),
        "openai" => Ok(Box::new(OpenAIClient::new(model, config)?)),
        "ollama" => Ok(Box::new(OllamaClient::new(model, config)?)),
        _ => anyhow::bail!("Unknown LLM provider: {}", provider),
    }
}
```

### Configuration

```toml
[logbook]
# Default LLM provider
default_provider = "anthropic"

# Default model
default_model = "claude-3-5-sonnet-20241022"

# API keys (or use environment variables)
[logbook.providers.anthropic]
api_key = "$ANTHROPIC_API_KEY"

[logbook.providers.openai]
api_key = "$OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"

# Default settings
[logbook.settings]
include_code = false
max_events = 0  # Unlimited
parallel = false
```

---

## Integration with pi-mono

### Agent Tool

```typescript
// pi-mono/packages/agent/src/tools/hstry-logbook.ts

export const getProjectLogbook = {
  name: "get_project_logbook",
  description: "Get a concise chronological logbook of decisions, facts, and bugs for a project",
  parameters: {
    workspace: {
      type: "string",
      description: "Workspace/project name"
    },
    format: {
      type: "string",
      enum: ["text", "json", "markdown"],
      default: "text",
      description: "Output format"
    },
    max_events: {
      type: "number",
      default: 50,
      description: "Maximum number of events to return"
    }
  },
  async execute({ workspace, format, max_events }: any) {
    const { exec } = await import('child_process');
    const { stdout } = await exec(
      `hstry logbook --workspace ${workspace} --format ${format} --max-events ${max_events}`
    );
    return stdout;
  }
};
```

### Agent Usage Example

```typescript
// Agent using the tool
const logbook = await agent.callTool('get_project_logbook', {
  workspace: 'my-project',
  format: 'json',
  max_events: 30
});

// logbook contains:
// {
//   workspace: "my-project",
//   timeline: [...]
// }

// Agent now has quick understanding of project history
```

---

## Storage Options

### Option A: File-based (default)
```bash
# Generate and save
hstry logbook --workspace my-project --output logbook.json

# Update incrementally
hstry logbook --workspace my-project --continue-from logbook.json --output logbook.json
```

**Pros:** Simple, no dependency on mmry
**Cons:** Not searchable across projects

### Option B: Store in mmry (optional)
```bash
# Store as a memory
hstry logbook --workspace my-project | byt memory add -c "logbook" -i 10

# Retrieve
byt memory search "logbook my-project" --repo hstry
```

**Pros:** Searchable, cross-repo
**Cons:** Not structured, loses timeline format

### Option C: Store in hstry DB (future)
```sql
CREATE TABLE logbooks (
    id TEXT PRIMARY KEY,
    workspace TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    content TEXT NOT NULL,  -- JSON
    source_from TEXT,
    source_to TEXT
);
```

**Pros:** Single source of truth, queryable
**Cons:** Storage overhead

**Recommendation:** Start with file-based, add DB storage later if needed.

---

## Migration Path

### Phase 1: Implement in hstry
- [ ] Create `hstry-logbook` crate
- [ ] Implement generation loop
- [ ] Add CLI command
- [ ] Add LLM client abstraction
- [ ] Support Anthropic, OpenAI, Ollama

### Phase 2: Test with real projects
- [ ] Generate logbooks for hstry itself
- [ ] Test incremental updates
- [ ] Validate output quality

### Phase 3: Integrate with pi-mono
- [ ] Create agent tool
- [ ] Test agent using logbook for context
- [ ] Document for agent developers

### Phase 4: Enhance
- [ ] Add DB storage option
- [ ] Add mmry integration
- [ ] Add more output formats
- [ ] Add filtering by date/type

---

## Summary

| Aspect | Recommended |
|--------|-------------|
| Implementation location | In hstry |
| LLM provider | Anthropic (default), OpenAI, Ollama |
| Storage | File-based (default), optional mmry/DB |
| Agent integration | Via pi-mono tool calling hstry CLI |
| API | CLI + JSON file format |

**Key insight:** The logbook is a *derived view* of hstry's core data, not a separate data store. Build it in hstry for direct access and simplicity, expose via CLI for agent consumption.
