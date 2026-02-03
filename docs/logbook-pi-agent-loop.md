# Logbook Generation Using pi's Agent-Loop

## Revised Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    hstry (data layer)               │
│  ┌─────────────────────────────────────────────────┐   │
│  │  CLI: hstry logbook --workspace X           │   │
│  │  - Reads conversations from DB              │   │
│  │  - Formats chunks                          │   │
│  │  - Invokes pi agent via CLI tool            │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                       │
                       │ (chunks)
                       ▼
┌─────────────────────────────────────────────────────────┐
│              pi agent-loop (orchestration)            │
│  ┌─────────────────────────────────────────────────┐   │
│  │  LogbookAgent                                 │   │
│  │  - Context: [logbook_state, current_chunk]   │   │
│  │  - Tools: append_events()                     │   │
│  │  - Loop over conversations                    │   │
│  │  - LLM calls via @mariozechner/pi-ai          │   │
│  └─────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────┐
│              Output: logbook.json                    │
└─────────────────────────────────────────────────────────┘
```

## Why This is Better

| Aspect | hstry-only | pi agent-loop |
|---------|-------------|---------------|
| **Separation of concerns** | ❌ LLM logic in hstry | ✓ hstry = data, pi = processing |
| **LLM infrastructure** | ❌ Must reimplement | ✓ Uses @mariozechner/pi-ai |
| **Agent capabilities** | ❌ Custom agent loop | ✓ Proven agent-loop framework |
| **Tool ecosystem** | ❌ None | ✓ Uses pi's tool system |
| **Extensibility** | ❌ Rust-only | ✓ TypeScript, easy to extend |
| **Testing** | ❌ Harder (needs mock LLM) | ✓ Easier (pi has test utils) |
| **State management** | ❌ Custom | ✓ Built-in context management |

## Implementation

### hstry: Export/Import Tool

```rust
// crates/hstry-cli/src/logbook.rs

pub struct LogbookInput {
    pub workspace: String,
    pub action: String,  // "get_conversations", "export_chunk"
    pub conversation_id: Option<String>,
}

pub struct ConversationChunk {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub messages: Vec<MessageExport>,
}

pub fn handle_logbook_export(input: LogbookInput) -> Result<serde_json::Value> {
    match input.action.as_str() {
        "get_conversations" => {
            let conversations = get_conversations(&input.workspace)?;
            Ok(json!(conversations))
        }
        "export_chunk" => {
            let chunk = export_conversation_chunk(
                &input.workspace,
                input.conversation_id.unwrap()
            )?;
            Ok(json!(chunk))
        }
        _ => Err(anyhow!("Unknown action: {}", input.action))
    }
}
```

```bash
# CLI integration
hstry logbook export --workspace my-project
hstry logbook export --workspace my-project --conversation-id <uuid>
```

### pi-mono: Logbook Agent

```typescript
// pi-mono/packages/logbook-agent/src/agent.ts

import { agentLoop } from "@mariozechner/agent";
import { streamSimple } from "@mariozechner/pi-ai";

interface LogbookState {
    workspace: string;
    timeline: TimelineEvent[];
    processed_conversations: string[];
}

export interface TimelineEvent {
    timestamp: string;
    type: "decision" | "fact" | "bug";
    content: string;
    rationale?: string;
    alternatives?: string[];
    code?: string;
    resolution?: string;
    source: {
        conversation_id: string;
        message_id?: string;
    };
}

export async function generateLogbook(
    workspace: string,
    model: Model<any>,
): Promise<TimelineEvent[]> {
    // Get all conversations from hstry
    const conversations = await hstryTool({
        workspace,
        action: "get_conversations"
    });

    // Sort chronologically (oldest first for building)
    conversations.sort((a, b) =>
        new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
    );

    const state: LogbookState = {
        workspace,
        timeline: [],
        processed_conversations: [],
    };

    // Process each conversation through agent loop
    for (const conv of conversations) {
        const chunk = await hstryTool({
            workspace,
            action: "export_chunk",
            conversation_id: conv.id
        });

        // Agent decides what to append
        const result = await processConversationWithAgent(state, chunk, model);

        if (result.new_events.length > 0) {
            state.timeline.push(...result.new_events);
        }
        state.processed_conversations.push(conv.id);
    }

    // Sort final timeline (newest first)
    state.timeline.sort((a, b) =>
        new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime()
    );

    return state.timeline;
}

async function processConversationWithAgent(
    state: LogbookState,
    chunk: any,
    model: Model<any>
): Promise<{ new_events: TimelineEvent[] }> {
    const systemPrompt = `You are a project historian extracting important events.

CURRENT LOGBOOK:
${formatLogbook(state.timeline)}

CURRENT CONVERSATION:
${formatChunk(chunk)}

TASK: Append meaningful events from this conversation.
- Only add events NOT already captured
- Types: decision, fact, bug
- Keep content concise (1 sentence)
- Include rationale for decisions
- Brief code only if critical

OUTPUT: JSON with "events" array containing events to add.
If nothing to add, return: {"events": []}`;

    const stream = agentLoop(
        [{ role: "user", content: systemPrompt }],
        {
            systemPrompt: "",
            model,
            tools: [appendEventsTool],
            messages: [],
            convertToLlm: (msgs) => msgs.filter(m => m.role === "user" || m.role === "assistant"),
        },
        undefined,
        streamSimple,
    );

    for await (const event of stream) {
        if (event.type === "tool_call") {
            // Handle append_events tool call
            const toolResult = await event.toolCall.tool.execute(event.toolCall.args);
            // Continue loop...
        }
        if (event.type === "agent_end") {
            const messages = event.messages as any[];
            const lastAssistant = messages.filter(m => m.role === "assistant").pop();
            if (lastAssistant?.tool_calls) {
                // Extract events from tool result
                const parsed = JSON.parse(lastAssistant.tool_calls[0].result);
                return { new_events: parsed.events };
            }
        }
    }

    return { new_events: [] };
}

const appendEventsTool: AgentTool<TimelineEvent[]> = {
    name: "append_events",
    description: "Append new events to logbook timeline",
    schema: {
        type: "object",
        properties: {
            events: {
                type: "array",
                items: {
                    type: "object",
                    properties: {
                        timestamp: { type: "string" },
                        type: { type: "string", enum: ["decision", "fact", "bug"] },
                        content: { type: "string" },
                        rationale: { type: "string" },
                        alternatives: { type: "array", items: { type: "string" } },
                        code: { type: "string" },
                        resolution: { type: "string" },
                        source: {
                            type: "object",
                            properties: {
                                conversation_id: { type: "string" },
                                message_id: { type: "string" }
                            }
                        }
                    },
                    required: ["timestamp", "type", "content", "source"]
                }
            }
        }
    },
    async execute(args): Promise<{ content: TimelineEvent[] }> {
        // Tool just returns events - caller adds to state
        return { content: args.events, details: "Appended to timeline" };
    }
};

function formatLogbook(timeline: TimelineEvent[]): string {
    let text = "=== Current Logbook ===\n\n";
    for (const event of timeline) {
        text += `${event.timestamp.split('T')[0]} | ${event.type.toUpperCase()} | ${event.content}\n`;
        if (event.rationale) text += `  → Rationale: ${event.rationale}\n`;
        if (event.code) text += `  → Code: ${event.code}\n`;
        text += '\n';
    }
    return text;
}

function formatChunk(chunk: any): string {
    let text = `Conversation: ${chunk.id}\n`;
    text += `Title: ${chunk.title}\n`;
    text += `Created: ${chunk.created_at}\n\n---\n`;

    for (const msg of chunk.messages) {
        text += `${msg.role}: ${msg.content}\n`;
        if (msg.timestamp) text += `(Time: ${msg.timestamp})\n`;
        text += '\n';
    }

    return text;
}
```

### hstry CLI Integration

```typescript
// hstry CLI calls pi agent

import { exec } from 'child_process';
import { generateLogbook } from '@byteowlz/logbook-agent';
import { getModel } from '@mariozechner/pi-ai';

// In hstry CLI implementation
async function handleLogbookCommand(args: LogbookArgs) {
    const model = await getModel(args.provider, args.model, args.apiKey);

    const timeline = await generateLogbook(args.workspace, model);

    // Write output
    if (args.format === 'json') {
        await writeFile(args.output, JSON.stringify({
            workspace: args.workspace,
            generated_at: new Date().toISOString(),
            timeline
        }, null, 2));
    } else {
        const text = formatTimelineText(timeline);
        await writeFile(args.output, text);
    }
}
```

Or simpler - hstry CLI just spawns pi agent:

```bash
# In hstry CLI
hstry logbook --workspace my-project --provider anthropic --model claude-3-5-sonnet
# Internally runs:
# pi-agent logbook --workspace my-project --provider anthropic --model claude-3-5-sonnet
```

## Tool Definition

```typescript
// pi-mono/packages/logbook-agent/src/hstry-tool.ts

export const hstryTool: AgentTool<any> = {
    name: "hstry",
    description: "Access hstry conversation data",
    schema: {
        type: "object",
        properties: {
            workspace: { type: "string" },
            action: {
                type: "string",
                enum: ["get_conversations", "export_chunk"]
            },
            conversation_id: { type: "string" }
        },
        required: ["workspace", "action"]
    },
    async execute(args) {
        // Call hstry CLI
        const cmd = `hstry logbook export --workspace ${args.workspace}`;
        if (args.action === "export_chunk" && args.conversation_id) {
            return await exec(`${cmd} --conversation-id ${args.conversation_id}`);
        }
        return await exec(cmd);
    }
};
```

## CLI Flow

```bash
# User runs:
hstry logbook --workspace my-project

# Flow:
# 1. hstry reads conversations from DB
# 2. hstry calls logbook agent (pi-mono)
# 3. Agent processes each conversation via agent-loop
# 4. Agent maintains logbook state in context
# 5. Agent outputs final timeline
# 6. hstry writes output (text/JSON/markdown)
```

## Benefits of This Approach

1. **Clean separation**: hstry = storage, pi = processing
2. **Proven infrastructure**: pi's agent-loop is battle-tested
3. **LLM integration**: Use @mariozechner/pi-ai directly
4. **Extensible**: Easy to add new features in TypeScript
5. **Testable**: pi has excellent test infrastructure
6. **Reusable**: Logbook agent can be used standalone or via hstry
7. **Language flexibility**: TypeScript is easier for rapid iteration

## Package Structure

```
pi-mono/packages/logbook-agent/  (NEW)
  └─ src/
      ├─ agent.ts          // Main logbook generation
      ├─ hstry-tool.ts    // Tool to access hstry
      ├─ types.ts         // Timeline types
      ├─ format.ts        // Output formatting
      └─ index.ts        // Exports

hstry/
  └─ crates/hstry-cli/
      └─ src/
          ├─ logbook.rs    // CLI command
          └─ main.ts       // Calls logbook agent
```

## Usage

```bash
# Via hstry (recommended)
hstry logbook --workspace my-project

# Direct pi agent
pi-agent logbook --workspace my-project

# With custom model
hstry logbook --workspace my-project --provider ollama --model llama3.1
```

## Summary

| Architecture | Responsibility |
|--------------|----------------|
| hstry | Data storage, conversation access |
| pi agent-loop | Logbook generation, LLM orchestration |
| @mariozechner/pi-ai | LLM provider abstraction |
| Output | JSON/text/markdown files |

This is the **clean separation of concerns**:
- hstry owns the data
- pi orchestrates the agent
- LLM does the extraction
