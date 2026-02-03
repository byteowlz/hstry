# Logbook Architecture: Final Decision

## Question: Where to implement logbook generation?

### Options Revisited

| Option | Location | LLM Layer |
|---------|-----------|------------|
| 1 | hstry (Rust) | Custom LLM client in Rust |
| 2 | pi agent-loop (TypeScript) | Uses @mariozechner/pi-ai |

---

## Comparison Matrix

| Factor | hstry-only | pi agent-loop | Winner |
|--------|-------------|---------------|---------|
| **Separation of concerns** | ❌ Data + Logic mixed | ✓ hstry=data, pi=logic | pi |
| **LLM infrastructure** | ❌ Must reimplement | ✓ @mariozechner/pi-ai | pi |
| **Agent capabilities** | ❌ Custom loop | ✓ Proven agent-loop | pi |
| **State management** | ❌ Manual | ✓ Built-in | pi |
| **Context compaction** | ❌ Custom | ✓ transformContext hook | pi |
| **Tool ecosystem** | ❌ None | ✓ pi tools | pi |
| **Extensibility** | ❌ Rust changes required | ✓ TypeScript | pi |
| **Testing** | ❌ Harder | ✓ pi test utils | pi |
| **Learning curve** | ❌ New codebase | ✓ Familiar pi patterns | pi |
| **Performance** | ✓ Direct DB access | ✓ Tool call overhead | Tie |
| **Dependencies** | ✓ No new deps | ✓ pi-mono already | Tie |
| **Rust vs TS** | ✓ Type safety | ✓ Faster dev | Tie |

---

## Recommendation: **pi agent-loop**

### Why This Wins

1. **hstry stays pure** - Data storage only, no LLM logic
2. **Proven infrastructure** - pi agent-loop is battle-tested
3. **Clean separation** - Data (hstry) ↔ Processing (pi)
4. **Easy iteration** - TypeScript for rapid development
5. **Leverages existing** - No reinventing wheels

### Architecture

```
hstry (data)
  └─ CLI: hstry logbook --workspace X
      ├─ Reads conversations from DB
      ├─ Formats chunks
      └─ Delegates to pi agent

pi agent-loop (orchestration)
  └─ LogbookAgent
      ├─ Maintains logbook state in context
      ├─ Calls LLM via @mariozechner/pi-ai
      ├─ Tool: append_events()
      └─ Processes each conversation

Output: logbook.json/text/markdown
```

---

## Implementation Plan

### Phase 1: hstry integration
```rust
// crates/hstry-cli/src/logbook.rs
// New command: hstry logbook export --workspace X
// Exports conversations as JSON for agent consumption
```

### Phase 2: pi logbook agent
```typescript
// pi-mono/packages/logbook-agent/
export async function generateLogbook(workspace: string): Promise<TimelineEvent[]>
// Uses agent-loop to process conversations
// Maintains logbook in context
```

### Phase 3: Wire together
```bash
# User runs:
hstry logbook --workspace my-project

# Internally:
# 1. hstry exports conversations
# 2. Calls pi logbook agent
# 3. Agent generates timeline
# 4. Output written
```

### Phase 4: Polish
- Add parallel processing
- Add incremental updates
- Add more output formats
- Add filtering options

---

## File Structure

```
hstry/
  └─ crates/hstry-cli/src/
      └─ logbook.rs         // Export CLI

pi-mono/
  └─ packages/logbook-agent/   // NEW
      └─ src/
          ├─ agent.ts        // Main generation logic
          ├─ hstry-tool.ts  // Tool to call hstry
          ├─ types.ts       // Timeline types
          ├─ format.ts      // Output formatting
          └─ index.ts       // Exports
```

---

## CLI Interface

```bash
# Primary interface (via hstry)
hstry logbook --workspace my-project

# With options
hstry logbook --workspace my-project \
  --format json \
  --output logbook.json \
  --provider anthropic \
  --model claude-3-5-sonnet

# Direct agent (for testing)
pi-agent logbook --workspace my-project
```

---

## Context Management (pi agent-loop)

```typescript
const config: AgentLoopConfig = {
    model,
    tools: [appendEventsTool],
    systemPrompt: "You are a project historian...",
    convertToLlm: (messages) => {
        // Only send user/assistant to LLM
        return messages.filter(m =>
            m.role === 'user' || m.role === 'assistant'
        );
    },
    transformContext: async (messages) => {
        // Compact logbook context if too large
        const tokens = estimateTokens(messages);
        if (tokens > 5000) {
            // Keep only recent 50 events
            return compactLogbook(messages);
        }
        return messages;
    },
};

// Agent loop automatically:
// - Maintains context across turns
// - Calls tools with results
// - Streams progress
// - Handles errors/retries
```

---

## Tool Definition

```typescript
const appendEventsTool: AgentTool<TimelineEvent[]> = {
    name: "append_events",
    description: "Append new events to logbook",
    schema: { /* ... */ },
    async execute(args) {
        // Return events - state managed by agent
        return {
            content: args.events,
            details: "Events to add"
        };
    }
};
```

The agent loop will:
1. Call LLM with [current_logbook, conversation_chunk]
2. LLM decides what to append
3. Calls `append_events` tool
4. Agent updates state (not the tool!)
5. Continues to next conversation

---

## Incremental Updates

```typescript
// In hstry CLI
if (args.continue_from) {
    const existing = JSON.parse(await readFile(args.continue_from));
    const from_date = existing.source_range.to;

    // Only get conversations after that date
    const conversations = await hstryTool({
        workspace,
        action: "get_conversations",
        after: from_date
    });

    // Continue from existing state
    const timeline = await generateLogbook(
        workspace,
        existing.timeline,
        conversations
    );
}
```

---

## Testing

```typescript
// pi-mono/packages/logbook-agent/test/
describe('LogbookAgent', () => {
    it('should extract decision from conversation', async () => {
        const mockHstry = createMockHstryTool();
        const model = createMockModel([
            assistantMessageWithToolCall('append_events', {
                events: [{
                    timestamp: '2026-01-15T10:30:00Z',
                    type: 'decision',
                    content: 'Use PostgreSQL',
                    rationale: 'ACID transactions',
                    source: { conversation_id: 'uuid' }
                }]
            })
        ]);

        const timeline = await generateLogbook('test-workspace', model);

        expect(timeline).toHaveLength(1);
        expect(timeline[0].type).toBe('decision');
    });

    it('should skip duplicate events', async () => {
        const existingTimeline = [/* ... */];
        const model = createMockModel([
            assistantMessageWithToolCall('append_events', { events: [] })
        ]);

        const timeline = await generateLogbook(
            'test-workspace',
            existingTimeline,
            [],
            model
        );

        // Should not add duplicates
        expect(timeline).toHaveLength(existingTimeline.length);
    });
});
```

---

## Migration Path

| Step | Action |
|-------|---------|
| 1 | Add `hstry logbook export` CLI command |
| 2 | Create `pi-mono/packages/logbook-agent` |
| 3 | Implement basic logbook generation |
| 4 | Test on hstry itself |
| 5 | Add incremental updates |
| 6 | Add pi-mono tool for agent consumption |

---

## Final Verdict

**Build logbook generation in pi-mono using agent-loop.**

This approach:
- Keeps hstry pure (data layer)
- Leverages pi's proven agent infrastructure
- Uses @mariozechner/pi-ai for LLM calls
- Provides clean separation of concerns
- Enables rapid iteration in TypeScript

The logbook is an **agent task**, not a data storage concern. Agent tasks belong in agent frameworks.
