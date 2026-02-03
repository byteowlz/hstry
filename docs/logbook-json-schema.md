# Complete JSON Schema for Logbook Extraction

## Top-Level Structure

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://hstry.dev/schemas/logbook-v1.json",
  "title": "Hstry Logbook",
  "description": "Concise extraction of facts and decisions from conversation history",
  "type": "object",
  "properties": {
    "version": {
      "type": "string",
      "description": "Schema version (semver)",
      "pattern": "^\\d+\\.\\d+\\.\\d+$"
    },
    "metadata": {
      "$ref": "#/$defs/Metadata"
    },
    "stats": {
      "$ref": "#/$defs/LogbookStats"
    },
    "facts": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/Fact"
      }
    },
    "decisions": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/Decision"
      }
    }
  },
  "required": ["version", "metadata", "stats", "facts", "decisions"],
  "additionalProperties": false
}
```

## Definitions

```json
{
  "$defs": {
    "Metadata": {
      "type": "object",
      "description": "Information about the logbook generation",
      "properties": {
        "workspace": {
          "type": "string",
          "description": "Workspace name"
        },
        "generated_at": {
          "type": "string",
          "format": "date-time",
          "description": "ISO8601 timestamp when logbook was generated"
        },
        "source_range": {
          "$ref": "#/$defs/SourceRange"
        },
        "config": {
          "$ref": "#/$defs/ExtractionConfig"
        },
        "hstry_version": {
          "type": "string",
          "description": "Version of hstry used to generate this logbook"
        }
      },
      "required": ["workspace", "generated_at", "source_range"]
    },

    "SourceRange": {
      "type": "object",
      "description": "Time range of conversations processed",
      "properties": {
        "from": {
          "type": "string",
          "format": "date-time",
          "description": "Oldest conversation timestamp processed"
        },
        "to": {
          "type": "string",
          "format": "date-time",
          "description": "Newest conversation timestamp processed"
        },
        "conversations_processed": {
          "type": "integer",
          "minimum": 0,
          "description": "Number of conversations analyzed"
        },
        "messages_processed": {
          "type": "integer",
          "minimum": 0,
          "description": "Number of messages analyzed"
        }
      },
      "required": ["from", "to"]
    },

    "ExtractionConfig": {
      "type": "object",
      "description": "Configuration used for extraction",
      "properties": {
        "code_blocks": {
          "type": "string",
          "enum": ["none", "context", "summary", "full"],
          "description": "How code blocks were handled"
        },
        "chunk_strategy": {
          "type": "string",
          "enum": ["conversation", "messages", "tokens"],
          "description": "Chunking strategy used"
        },
        "chunk_size": {
          "type": "integer",
          "minimum": 1,
          "description": "Chunk size (messages or tokens depending on strategy)"
        },
        "similarity_threshold": {
          "type": "number",
          "minimum": 0.0,
          "maximum": 1.0,
          "description": "Deduplication similarity threshold"
        },
        "llm_model": {
          "type": "string",
          "description": "LLM model used for extraction"
        }
      }
    },

    "LogbookStats": {
      "type": "object",
      "description": "Statistics about extracted content",
      "properties": {
        "total_facts": {
          "type": "integer",
          "minimum": 0,
          "description": "Total number of facts extracted"
        },
        "total_decisions": {
          "type": "integer",
          "minimum": 0,
          "description": "Total number of decisions extracted"
        },
        "facts_by_category": {
          "type": "object",
          "description": "Facts count per category",
          "additionalProperties": {
            "type": "integer",
            "minimum": 0
          }
        },
        "decisions_by_category": {
          "type": "object",
          "description": "Decisions count per category",
          "additionalProperties": {
            "type": "integer",
            "minimum": 0
          }
        },
        "deduplication_stats": {
          "$ref": "#/$defs/DeduplicationStats"
        }
      },
      "required": ["total_facts", "total_decisions"]
    },

    "DeduplicationStats": {
      "type": "object",
      "description": "Statistics about deduplication",
      "properties": {
        "facts_before_dedup": {
          "type": "integer",
          "minimum": 0
        },
        "facts_after_dedup": {
          "type": "integer",
          "minimum": 0
        },
        "facts_removed": {
          "type": "integer",
          "minimum": 0
        },
        "decisions_before_dedup": {
          "type": "integer",
          "minimum": 0
        },
        "decisions_after_dedup": {
          "type": "integer",
          "minimum": 0
        },
        "decisions_merged": {
          "type": "integer",
          "minimum": 0
        }
      },
      "required": [
        "facts_before_dedup", "facts_after_dedup", "facts_removed",
        "decisions_before_dedup", "decisions_after_dedup", "decisions_merged"
      ]
    },

    "Fact": {
      "type": "object",
      "description": "A fact extracted from conversation",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid",
          "description": "Unique identifier for this fact"
        },
        "statement": {
          "type": "string",
          "minLength": 1,
          "maxLength": 500,
          "description": "Concise factual statement (1-2 sentences)"
        },
        "category": {
          "type": "string",
          "enum": [
            "architecture",
            "bug",
            "feature",
            "config",
            "performance",
            "api",
            "database",
            "security",
            "testing",
            "deployment",
            "other"
          ],
          "description": "Category of the fact"
        },
        "confidence": {
          "type": "number",
          "minimum": 0.0,
          "maximum": 1.0,
          "description": "Confidence score (0-1)"
        },
        "related_entities": {
          "type": "array",
          "items": {
            "type": "string"
          },
          "description": "Files, components, or systems mentioned"
        },
        "context": {
          "type": "string",
          "maxLength": 1000,
          "description": "Brief context or code snippet (only included with --code-blocks context/full)"
        },
        "source": {
          "$ref": "#/$defs/SourceReference"
        }
      },
      "required": ["id", "statement", "category", "confidence", "source"]
    },

    "Decision": {
      "type": "object",
      "description": "A decision made during the conversation",
      "properties": {
        "id": {
          "type": "string",
          "format": "uuid",
          "description": "Unique identifier for this decision"
        },
        "description": {
          "type": "string",
          "minLength": 1,
          "maxLength": 500,
          "description": "What was decided (concise)"
        },
        "rationale": {
          "type": "string",
          "minLength": 1,
          "maxLength": 2000,
          "description": "Why this decision was made"
        },
        "alternatives_considered": {
          "type": "array",
          "items": {
            "type": "string",
            "maxLength": 300
          },
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
        "code_evidence": {
          "$ref": "#/$defs/CodeEvidence",
          "description": "Code demonstrating the decision (optional)"
        },
        "source": {
          "$ref": "#/$defs/SourceReference"
        }
      },
      "required": ["id", "description", "rationale", "impact", "category", "source"]
    },

    "CodeEvidence": {
      "type": "object",
      "description": "Code demonstrating a decision",
      "properties": {
        "snippet": {
          "type": "string",
          "minLength": 1,
          "maxLength": 5000,
          "description": "Code demonstrating the decision (3-10 lines recommended)"
        },
        "language": {
          "type": "string",
          "description": "Programming language (rust, typescript, python, etc.)"
        },
        "purpose": {
          "type": "string",
          "maxLength": 300,
          "description": "What this code demonstrates"
        },
        "context": {
          "type": "string",
          "maxLength": 300,
          "description": "Where this code would be used (optional)"
        }
      },
      "required": ["snippet", "language", "purpose"]
    },

    "SourceReference": {
      "type": "object",
      "description": "Reference to the source conversation",
      "properties": {
        "conversation_id": {
          "type": "string",
          "format": "uuid"
        },
        "conversation_title": {
          "type": "string",
          "description": "Title of the source conversation"
        },
        "message_id": {
          "type": "string",
          "format": "uuid",
          "description": "Specific message where fact/decision appeared"
        },
        "timestamp": {
          "type": "string",
          "format": "date-time",
          "description": "ISO8601 timestamp from the conversation"
        },
        "workspace": {
          "type": "string",
          "description": "Workspace name"
        },
        "model": {
          "type": "string",
          "description": "LLM model used in the conversation"
        }
      },
      "required": ["conversation_id", "timestamp"]
    }
  }
}
```

---

## Complete Example

```json
{
  "version": "1.0.0",
  "metadata": {
    "workspace": "my-project",
    "generated_at": "2026-02-02T13:11:53.123Z",
    "source_range": {
      "from": "2026-01-15T10:30:00Z",
      "to": "2026-02-02T11:00:00Z",
      "conversations_processed": 47,
      "messages_processed": 1523
    },
    "config": {
      "code_blocks": "context",
      "chunk_strategy": "conversation",
      "chunk_size": 1,
      "similarity_threshold": 0.85,
      "llm_model": "claude-3-5-sonnet-20241022"
    },
    "hstry_version": "0.5.2"
  },
  "stats": {
    "total_facts": 23,
    "total_decisions": 8,
    "facts_by_category": {
      "architecture": 5,
      "api": 4,
      "config": 3,
      "bug": 2,
      "feature": 4,
      "performance": 2,
      "database": 2,
      "deployment": 1
    },
    "decisions_by_category": {
      "technical": 6,
      "infrastructure": 2
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
  "facts": [
    {
      "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
      "statement": "The project uses Rust with Tokio for async runtime",
      "category": "architecture",
      "confidence": 1.0,
      "related_entities": ["Rust", "Tokio", "async runtime"],
      "context": "Use `#[tokio::main]` and `.await` for async operations",
      "source": {
        "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "conversation_title": "Project Setup and Architecture",
        "message_id": "b2c3d4e5-f6a7-8901-bcde-f12345678901",
        "timestamp": "2026-01-15T10:30:00Z",
        "workspace": "my-project",
        "model": "claude-3-5-sonnet-20241022"
      }
    },
    {
      "id": "f47ac10b-58cc-4372-a567-0e02b2c3d480",
      "statement": "PostgreSQL 15 is used as the primary database with JSONB support",
      "category": "database",
      "confidence": 1.0,
      "related_entities": ["PostgreSQL", "JSONB"],
      "context": "Queries use `sqlx::query_as!` with `jsonb` type for structured data",
      "source": {
        "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
        "conversation_title": "Database Design",
        "message_id": "d4e5f6a7-b8c9-0123-def0-123456789012",
        "timestamp": "2026-01-16T14:20:00Z",
        "workspace": "my-project"
      }
    },
    {
      "id": "f47ac10b-58cc-4372-a567-0e02b2c3d481",
      "statement": "Memory leak in connection pool when connections aren't properly released",
      "category": "bug",
      "confidence": 0.85,
      "related_entities": ["PgPool", "connection pool"],
      "context": "Fix in v0.2.1: ensure `.await` is called for all pool operations",
      "source": {
        "conversation_id": "e5f6a7b8-c9d0-1234-ef01-234567890123",
        "conversation_title": "Debugging Memory Issues",
        "message_id": "f6a7b8c9-d0e1-2345-f012-345678901234",
        "timestamp": "2026-01-28T09:45:00Z"
      }
    }
  ],
  "decisions": [
    {
      "id": "d47ac10b-58cc-4372-a567-0e02b2c3d479",
      "description": "Use PostgreSQL as the primary database",
      "rationale": "Need ACID transactions and native JSONB support for structured data",
      "alternatives_considered": ["MySQL", "MongoDB", "SQLite"],
      "impact": "high",
      "category": "technical",
      "code_evidence": {
        "snippet": "let pool = PgPoolOptions::new()\n    .max_connections(10)\n    .connect(&database_url).await?;\n\nlet user: User = sqlx::query_as!(\n    User,\n    \"SELECT * FROM users WHERE id = $1 AND data @> $2\",\n    id,\n    json!( {\"key\": \"value\"} )\n)\n.fetch_one(&pool)\n.await?;",
        "language": "rust",
        "purpose": "Example of PostgreSQL connection setup with JSONB query",
        "context": "Database module initialization"
      },
      "source": {
        "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
        "conversation_title": "Database Design",
        "message_id": "e5f6a7b8-c9d0-1234-ef01-234567890123",
        "timestamp": "2026-01-16T14:20:00Z"
      }
    },
    {
      "id": "d47ac10b-58cc-4372-a567-0e02b2c3d480",
      "description": "Deploy with Kubernetes on AWS EKS",
      "rationale": "Team has existing expertise, managed service reduces operational burden",
      "alternatives_considered": ["Docker Compose on EC2", "AWS ECS", "Self-hosted k8s"],
      "impact": "high",
      "category": "infrastructure",
      "code_evidence": null,
      "source": {
        "conversation_id": "f7a8b9c0-d1e2-3456-f123-456789012345",
        "conversation_title": "Infrastructure Planning",
        "message_id": "08b9c0d1-e2f3-4567-0234-567890123456",
        "timestamp": "2026-01-25T16:00:00Z"
      }
    },
    {
      "id": "d47ac10b-58cc-4372-a567-0e02b2c3d481",
      "description": "Use async/await pattern with Result<T, E> for error handling",
      "rationale": "Allows non-blocking operations with explicit error handling",
      "alternatives_considered": ["Blocking with panic on error", "Callback-based async", "Option for errors"],
      "impact": "high",
      "category": "technical",
      "code_evidence": {
        "snippet": "async fn fetch_user(id: i32) -> Result<User, DbError> {\n    let user = sqlx::query_as!(User, \"SELECT * FROM users WHERE id = ?\", id)\n        .fetch_one(&pool)\n        .await\n        .map_err(|e| DbError::NotFound(id))?;\n    Ok(user)\n}",
        "language": "rust",
        "purpose": "Example of async function with Result error handling"
      },
      "source": {
        "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "message_id": "b2c3d4e5-f6a7-8901-bcde-f12345678901",
        "timestamp": "2026-01-20T11:30:00Z"
      }
    }
  ]
}
```

---

## Example with Code Blocks: None Mode

```json
{
  "version": "1.0.0",
  "metadata": {
    "workspace": "my-project",
    "generated_at": "2026-02-02T13:11:53.123Z",
    "source_range": {
      "from": "2026-01-15T10:30:00Z",
      "to": "2026-02-02T11:00:00Z"
    },
    "config": {
      "code_blocks": "none"
    }
  },
  "stats": {
    "total_facts": 23,
    "total_decisions": 8
  },
  "facts": [
    {
      "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
      "statement": "The project uses Rust with Tokio for async runtime",
      "category": "architecture",
      "confidence": 1.0,
      "source": {
        "conversation_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "timestamp": "2026-01-15T10:30:00Z"
      }
    }
  ],
  "decisions": [
    {
      "id": "d47ac10b-58cc-4372-a567-0e02b2c3d479",
      "description": "Use PostgreSQL as the primary database",
      "rationale": "Need ACID transactions and native JSONB support",
      "alternatives_considered": ["MySQL", "MongoDB"],
      "impact": "high",
      "category": "technical",
      "source": {
        "conversation_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
        "timestamp": "2026-01-16T14:20:00Z"
      }
    }
  ]
}
```

---

## Schema Validation

```bash
# Validate a logbook file against the schema
ajv validate -s logbook-schema.json -d logbook.json

# Using hstry CLI
hstry logbook validate --schema logbook-schema.json logbook.json

# Generate TypeScript types from schema
json2ts -i logbook-schema.json -o logbook.types.ts
```

---

## Versioning

The schema version follows semver: `MAJOR.MINOR.PATCH`

- **MAJOR**: Breaking changes to structure
- **MINOR**: New optional fields added
- **MINOR**: New enum values added to existing fields
- **PATCH**: Documentation updates, description changes

**Backwards compatibility** is maintained across MINOR and PATCH versions.

Example version progression:
- `1.0.0` - Initial release
- `1.1.0` - Added `hstry_version` field (optional)
- `1.2.0` - Added new fact category `ai`
- `2.0.0` - Restructured source reference (breaking)
