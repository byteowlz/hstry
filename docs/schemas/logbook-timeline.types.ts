/**
 * Hstry Logbook Timeline Types
 * Version: 1.0.0
 */

export interface LogbookTimeline {
  workspace: string;
  generated_at: string;
  source_range: {
    from: string;
    to: string;
  };
  timeline: TimelineEvent[];
}

export type EventType = "decision" | "fact" | "bug";

export interface TimelineEvent {
  timestamp: string;
  type: EventType;
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

// ============================================================================
// Helpers
// ============================================================================

export function isDecision(event: TimelineEvent): event is TimelineEvent & { type: "decision" } {
  return event.type === "decision";
}

export function isFact(event: TimelineEvent): event is TimelineEvent & { type: "fact" } {
  return event.type === "fact";
}

export function isBug(event: TimelineEvent): event is TimelineEvent & { type: "bug" } {
  return event.type === "bug";
}

export function sortByDate(events: TimelineEvent[]): TimelineEvent[] {
  return [...events].sort((a, b) =>
    new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime()
  );
}

export function filterByType(events: TimelineEvent[], type: EventType): TimelineEvent[] {
  return events.filter((e) => e.type === type);
}

export function filterByDateRange(
  events: TimelineEvent[],
  from: string,
  to: string
): TimelineEvent[] {
  const fromTime = new Date(from).getTime();
  const toTime = new Date(to).getTime();
  return events.filter(
    (e) => {
      const t = new Date(e.timestamp).getTime();
      return t >= fromTime && t <= toTime;
    }
  );
}

export function formatEventText(event: TimelineEvent): string {
  const date = new Date(event.timestamp).toISOString().split("T")[0];
  const time = new Date(event.timestamp).toISOString().split("T")[1].split(".")[0];
  const type = event.type.toUpperCase();

  let line = `${date} ${time} | ${type} | ${event.content}`;

  if (event.rationale) {
    line += `\n  → Rationale: ${event.rationale}`;
  }
  if (event.alternatives && event.alternatives.length > 0) {
    line += `\n  → Alternatives: ${event.alternatives.join(", ")}`;
  }
  if (event.code) {
    line += `\n  → Code: ${event.code}`;
  }
  if (event.resolution) {
    line += `\n  → Resolution: ${event.resolution}`;
  }
  line += `\n  → Source: conv:${event.source.conversation_id.slice(0, 8)}`;

  return line;
}

export function formatLogbookText(logbook: LogbookTimeline): string {
  const lines: string[] = [];

  lines.push(`=== Project Logbook: ${logbook.workspace} ===`);
  lines.push(`Generated: ${new Date(logbook.generated_at).toISOString().replace("T", " ").split(".")[0]} UTC`);

  const from = new Date(logbook.source_range.from).toISOString().split("T")[0];
  const to = new Date(logbook.source_range.to).toISOString().split("T")[0];
  lines.push(`Source: ${logbook.timeline.length} events, ${from} → ${to}`);
  lines.push("");

  const sorted = sortByDate(logbook.timeline);
  for (const event of sorted) {
    lines.push(formatEventText(event));
    lines.push("");
  }

  return lines.join("\n");
}

export function countByType(logbook: LogbookTimeline): Record<EventType, number> {
  return {
    decision: logbook.timeline.filter(isDecision).length,
    fact: logbook.timeline.filter(isFact).length,
    bug: logbook.timeline.filter(isBug).length,
  };
}

export function getRecentEvents(logbook: LogbookTimeline, limit: number): TimelineEvent[] {
  return sortByDate(logbook.timeline).slice(0, limit);
}

export function hasCodeEvidence(event: TimelineEvent): boolean {
  return event.code != null && event.code.length > 0;
}
