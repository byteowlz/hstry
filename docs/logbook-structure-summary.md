# Logbook JSON Structure - Quick Reference

## Root Object

```
{
  version: "1.0.0"           // Schema version (semver)
  metadata: { ... }           // Generation metadata
  stats: { ... }             // Extraction statistics
  facts: [ ... ]             // Array of Fact objects
  decisions: [ ... ]         // Array of Decision objects
}
```

## Metadata

```
{
  workspace: "my-project"           // Workspace name
  generated_at: "2026-02-02T13:11:53Z"
  source_range: {
    from: "2026-01-15T10:30:00Z"
    to: "2026-02-02T11:00:00Z"
    conversations_processed: 47
    messages_processed: 1523
  }
  config: {                        // Optional
    code_blocks: "context"         // none | context | summary | full
    chunk_strategy: "conversation" // conversation | messages | tokens
    similarity_threshold: 0.85
    llm_model: "claude-3-5-sonnet-20241022"
  }
  hstry_version: "0.5.2"          // Optional
}
```

## Stats

```
{
  total_facts: 23
  total_decisions: 8
  facts_by_category: {
    architecture: 5
    api: 4
    config: 3
    bug: 2
    feature: 4
    performance: 2
    database: 2
    deployment: 1
  }
  decisions_by_category: {
    technical: 6
    infrastructure: 2
  }
  deduplication_stats: {          // Optional
    facts_before_dedup: 31
    facts_after_dedup: 23
    facts_removed: 8
    decisions_before_dedup: 10
    decisions_after_dedup: 8
    decisions_merged: 2
  }
}
```

## Fact Object

```
{
  id: "uuid-v4"
  statement: "The project uses Rust with Tokio for async runtime"
  category: "architecture"         // architecture | bug | feature | config | performance | api | database | security | testing | deployment | other
  confidence: 1.0                  // 0.0 - 1.0
  related_entities: ["Rust", "Tokio", "async runtime"]
  context: "Use `#[tokio::main]` and `.await` for async operations"  // Optional, only with --code-blocks context/full
  source: {
    conversation_id: "uuid-v4"
    conversation_title: "Project Setup"  // Optional
    message_id: "uuid-v4"                // Optional
    timestamp: "2026-01-15T10:30:00Z"
    workspace: "my-project"             // Optional
    model: "claude-3-5-sonnet"          // Optional
  }
}
```

## Decision Object

```
{
  id: "uuid-v4"
  description: "Use PostgreSQL as the primary database"
  rationale: "Need ACID transactions and native JSONB support"
  alternatives_considered: ["MySQL", "MongoDB", "SQLite"]
  impact: "high"                        // high | medium | low
  category: "technical"                // technical | product | process | infrastructure | team | other
  code_evidence: {                     // Optional, only with --code-blocks context/full
    snippet: "let pool = PgPoolOptions::new()..."
    language: "rust"
    purpose: "Example of PostgreSQL connection setup"
    context: "Database module"         // Optional
  }
  source: {
    conversation_id: "uuid-v4"
    conversation_title: "Database Design"  // Optional
    message_id: "uuid-v4"                 // Optional
    timestamp: "2026-01-16T14:20:00Z"
    workspace: "my-project"              // Optional
  }
}
```

---

## Code Block Modes Comparison

| Field | `none` | `context` (default) | `summary` | `full` |
|-------|--------|---------------------|-----------|--------|
| Fact.context | ❌ | ✅ brief snippets | ✅ summarized | ✅ full blocks |
| Decision.code_evidence | ❌ | ✅ 3-10 lines | ✅ summarized | ✅ full blocks |

---

## Fact Categories

- `architecture` - Design patterns, module structure, tech stack choices
- `bug` - Issues discovered, errors reported, bugs fixed
- `feature` - Feature implementations, capabilities added
- `config` - Configuration values, environment variables, settings
- `performance` - Performance findings, optimizations, benchmarks
- `api` - API designs, endpoints, signatures, contracts
- `database` - Schema, queries, migrations, data models
- `security` - Security considerations, vulnerabilities, auth methods
- `testing` - Test strategies, test coverage, testing frameworks
- `deployment` - Deployment methods, CI/CD, infrastructure
- `other` - Anything that doesn't fit other categories

## Decision Categories

- `technical` - Implementation choices, libraries, patterns
- `product` - Product decisions, features, priorities
- `process` - Workflow, team processes, development practices
- `infrastructure` - Deployment, hosting, tools, services
- `team` - Team organization, roles, responsibilities
- `other` - Other organizational decisions

---

## Minimal Example (code_blocks: none)

```json
{
  "version": "1.0.0",
  "metadata": {
    "workspace": "my-project",
    "generated_at": "2026-02-02T13:11:53Z",
    "source_range": {
      "from": "2026-01-15T10:30:00Z",
      "to": "2026-02-02T11:00:00Z"
    },
    "config": { "code_blocks": "none" }
  },
  "stats": {
    "total_facts": 1,
    "total_decisions": 1
  },
  "facts": [{
    "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
    "statement": "The project uses Rust with Tokio",
    "category": "architecture",
    "confidence": 1.0,
    "source": {
      "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "timestamp": "2026-01-15T10:30:00Z"
    }
  }],
  "decisions": [{
    "id": "d47ac10b-58cc-4372-a567-0e02b2c3d479",
    "description": "Use PostgreSQL as the primary database",
    "rationale": "Need ACID transactions",
    "alternatives_considered": ["MySQL"],
    "impact": "high",
    "category": "technical",
    "source": {
      "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
      "timestamp": "2026-01-16T14:20:00Z"
    }
  }]
}
```

## Full Example (code_blocks: context)

```json
{
  "version": "1.0.0",
  "metadata": {
    "workspace": "my-project",
    "generated_at": "2026-02-02T13:11:53Z",
    "source_range": {
      "from": "2026-01-15T10:30:00Z",
      "to": "2026-02-02T11:00:00Z",
      "conversations_processed": 47,
      "messages_processed": 1523
    },
    "config": {
      "code_blocks": "context",
      "chunk_strategy": "conversation",
      "similarity_threshold": 0.85,
      "llm_model": "claude-3-5-sonnet-20241022"
    }
  },
  "stats": {
    "total_facts": 23,
    "total_decisions": 8,
    "facts_by_category": {
      "architecture": 5, "api": 4, "config": 3,
      "bug": 2, "feature": 4, "performance": 2,
      "database": 2, "deployment": 1
    },
    "decisions_by_category": {
      "technical": 6, "infrastructure": 2
    },
    "deduplication_stats": {
      "facts_before_dedup": 31,
      "facts_after_dedup": 23,
      "facts_removed": 8,
      "decisions_before_dedup": 10,
      "decisions_after_dedup": 8,
      "decisions_merged": 2
    }
  },
  "facts": [{
    "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
    "statement": "The project uses Rust with Tokio for async runtime",
    "category": "architecture",
    "confidence": 1.0,
    "related_entities": ["Rust", "Tokio"],
    "context": "Use `#[tokio::main]` and `.await` for async operations",
    "source": {
      "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "conversation_title": "Project Setup",
      "message_id": "b2c3d4e5-f6a7-8901-bcde-f12345678901",
      "timestamp": "2026-01-15T10:30:00Z",
      "workspace": "my-project",
      "model": "claude-3-5-sonnet-20241022"
    }
  }],
  "decisions": [{
    "id": "d47ac10b-58cc-4372-a567-0e02b2c3d479",
    "description": "Use PostgreSQL as the primary database",
    "rationale": "Need ACID transactions and native JSONB support",
    "alternatives_considered": ["MySQL", "MongoDB"],
    "impact": "high",
    "category": "technical",
    "code_evidence": {
      "snippet": "let pool = PgPoolOptions::new()\n    .max_connections(10)\n    .connect(&database_url).await?;",
      "language": "rust",
      "purpose": "Example of PostgreSQL connection setup"
    },
    "source": {
      "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
      "conversation_title": "Database Design",
      "message_id": "d4e5f6a7-b8c9-0123-def0-123456789012",
      "timestamp": "2026-01-16T14:20:00Z"
    }
  }]
}
```

---

## File Locations

- **Full documentation**: `docs/logbook-json-schema.md`
- **JSON Schema**: `docs/schemas/logbook-v1.json`
- **TypeScript types**: `docs/schemas/logbook.types.ts`
- **This quick reference**: `docs/logbook-structure-summary.md`
