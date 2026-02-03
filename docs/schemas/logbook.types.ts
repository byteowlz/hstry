/**
 * Hstry Logbook Types
 * Auto-generated from logbook-v1.json schema
 * Version: 1.0.0
 */

export interface Logbook {
  /** Schema version (semver) */
  version: `${number}.${number}.${number}`;

  /** Information about the logbook generation */
  metadata: LogbookMetadata;

  /** Statistics about extracted content */
  stats: LogbookStats;

  /** Facts extracted from conversations */
  facts: Fact[];

  /** Decisions extracted from conversations */
  decisions: Decision[];
}

export interface LogbookMetadata {
  /** Workspace name */
  workspace: string;

  /** ISO8601 timestamp when logbook was generated */
  generated_at: string;

  /** Time range of conversations processed */
  source_range: SourceRange;

  /** Configuration used for extraction */
  config?: ExtractionConfig;

  /** Version of hstry used to generate this logbook */
  hstry_version?: string;
}

export interface SourceRange {
  /** Oldest conversation timestamp processed */
  from: string;

  /** Newest conversation timestamp processed */
  to: string;

  /** Number of conversations analyzed */
  conversations_processed?: number;

  /** Number of messages analyzed */
  messages_processed?: number;
}

export interface ExtractionConfig {
  /** How code blocks were handled */
  code_blocks?: "none" | "context" | "summary" | "full";

  /** Chunking strategy used */
  chunk_strategy?: "conversation" | "messages" | "tokens";

  /** Chunk size (messages or tokens depending on strategy) */
  chunk_size?: number;

  /** Deduplication similarity threshold (0.0-1.0) */
  similarity_threshold?: number;

  /** LLM model used for extraction */
  llm_model?: string;
}

export interface LogbookStats {
  /** Total number of facts extracted */
  total_facts: number;

  /** Total number of decisions extracted */
  total_decisions: number;

  /** Facts count per category */
  facts_by_category?: Record<string, number>;

  /** Decisions count per category */
  decisions_by_category?: Record<string, number>;

  /** Statistics about deduplication */
  deduplication_stats?: DeduplicationStats;
}

export interface DeduplicationStats {
  facts_before_dedup: number;
  facts_after_dedup: number;
  facts_removed: number;
  decisions_before_dedup: number;
  decisions_after_dedup: number;
  decisions_merged: number;
}

export interface Fact {
  /** Unique identifier for this fact */
  id: string;

  /** Concise factual statement (1-2 sentences) */
  statement: string;

  /** Category of the fact */
  category:
    | "architecture"
    | "bug"
    | "feature"
    | "config"
    | "performance"
    | "api"
    | "database"
    | "security"
    | "testing"
    | "deployment"
    | "other";

  /** Confidence score (0-1) */
  confidence: number;

  /** Files, components, or systems mentioned */
  related_entities?: string[];

  /** Brief context or code snippet (only included with --code-blocks context/full) */
  context?: string;

  /** Reference to the source conversation */
  source: SourceReference;
}

export interface Decision {
  /** Unique identifier for this decision */
  id: string;

  /** What was decided (concise) */
  description: string;

  /** Why this decision was made */
  rationale: string;

  /** Other options that were discussed */
  alternatives_considered?: string[];

  /** Impact level of this decision */
  impact: "high" | "medium" | "low";

  /** Type of decision */
  category:
    | "technical"
    | "product"
    | "process"
    | "infrastructure"
    | "team"
    | "other";

  /** Code demonstrating the decision (optional) */
  code_evidence?: CodeEvidence | null;

  /** Reference to the source conversation */
  source: SourceReference;
}

export interface CodeEvidence {
  /** Code demonstrating the decision (3-10 lines recommended) */
  snippet: string;

  /** Programming language (rust, typescript, python, etc.) */
  language: string;

  /** What this code demonstrates */
  purpose: string;

  /** Where this code would be used (optional) */
  context?: string;
}

export interface SourceReference {
  /** Conversation UUID */
  conversation_id: string;

  /** Title of the source conversation */
  conversation_title?: string;

  /** Specific message UUID where fact/decision appeared */
  message_id?: string;

  /** ISO8601 timestamp from the conversation */
  timestamp: string;

  /** Workspace name */
  workspace?: string;

  /** LLM model used in the conversation */
  model?: string;
}

// ============================================================================
// Utility Types
// ============================================================================

export type FactCategory = Fact["category"];
export type DecisionCategory = Decision["category"];
export type CodeBlockMode = NonNullable<ExtractionConfig["code_blocks"]>;
export type ChunkStrategy = NonNullable<ExtractionConfig["chunk_strategy"]>;

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Get all facts of a specific category
 */
export function getFactsByCategory(
  logbook: Logbook,
  category: FactCategory
): Fact[] {
  return logbook.facts.filter((f) => f.category === category);
}

/**
 * Get all decisions of a specific category
 */
export function getDecisionsByCategory(
  logbook: Logbook,
  category: DecisionCategory
): Decision[] {
  return logbook.decisions.filter((d) => d.category === category);
}

/**
 * Get facts with code evidence (context field present)
 */
export function getFactsWithContext(logbook: Logbook): Fact[] {
  return logbook.facts.filter((f) => f.context != null && f.context !== "");
}

/**
 * Get decisions with code evidence
 */
export function getDecisionsWithCodeEvidence(logbook: Logbook): Decision[] {
  return logbook.decisions.filter((d) => d.code_evidence != null);
}

/**
 * Sort facts by timestamp (newest first)
 */
export function sortFactsByTimestamp(facts: Fact[]): Fact[] {
  return [...facts].sort((a, b) =>
    new Date(b.source.timestamp).getTime() - new Date(a.source.timestamp).getTime()
  );
}

/**
 * Sort decisions by timestamp (newest first)
 */
export function sortDecisionsByTimestamp(decisions: Decision[]): Decision[] {
  return [...decisions].sort((a, b) =>
    new Date(b.source.timestamp).getTime() - new Date(a.source.timestamp).getTime()
  );
}

/**
 * Filter facts by confidence threshold
 */
export function filterFactsByConfidence(
  facts: Fact[],
  minConfidence: number
): Fact[] {
  return facts.filter((f) => f.confidence >= minConfidence);
}

/**
 * Filter decisions by impact level
 */
export function filterDecisionsByImpact(
  decisions: Decision[],
  impact: Decision["impact"]
): Decision[] {
  return decisions.filter((d) => d.impact === impact);
}

/**
 * Get all unique entities mentioned across facts
 */
export function getAllEntities(logbook: Logbook): Set<string> {
  const entities = new Set<string>();
  for (const fact of logbook.facts) {
    if (fact.related_entities) {
      for (const entity of fact.related_entities) {
        entities.add(entity);
      }
    }
  }
  return entities;
}

/**
 * Group facts by date (YYYY-MM-DD)
 */
export function groupFactsByDate(facts: Fact[]): Map<string, Fact[]> {
  const groups = new Map<string, Fact[]>();
  for (const fact of facts) {
    const date = fact.source.timestamp.split("T")[0];
    if (!groups.has(date)) {
      groups.set(date, []);
    }
    groups.get(date)!.push(fact);
  }
  return groups;
}

/**
 * Group decisions by date (YYYY-MM-DD)
 */
export function groupDecisionsByDate(decisions: Decision[]): Map<string, Decision[]> {
  const groups = new Map<string, Decision[]>();
  for (const decision of decisions) {
    const date = decision.source.timestamp.split("T")[0];
    if (!groups.has(date)) {
      groups.set(date, []);
    }
    groups.get(date)!.push(decision);
  }
  return groups;
}

// ============================================================================
// Type Guards
// ============================================================================

/**
 * Check if a decision has code evidence
 */
export function hasCodeEvidence(decision: Decision): decision is Decision & { code_evidence: CodeEvidence } {
  return decision.code_evidence != null;
}

/**
 * Check if a fact has context
 */
export function hasContext(fact: Fact): fact is Fact & { context: string } {
  return fact.context != null && fact.context !== "";
}
