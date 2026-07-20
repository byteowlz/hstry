/**
 * Shared first-real-user-message (FRUM) extraction for adapters.
 *
 * Different agent harnesses (Claude Code, Codex, Pi, etc.) all face the same
 * problem: the first user message in a session is often a system bootstrap
 * (AGENTS.md instructions, system prompt, MCP tool list). Using that as a
 * conversation title produces noise. This module gives every adapter the same
 * filter so derived titles describe what the user actually asked.
 *
 * Mirrors `is_system_context` in crates/hstry-cli/src/main.rs — keep them
 * aligned when adding new markers.
 */

const SYSTEM_CONTEXT_MARKERS = [
  '# AGENTS.md',
  '# Agent Configuration',
  '<available_skills>',
  'Guidance for coding agents',
  '<SYSTEM_PROMPT>',
  '</SYSTEM_PROMPT>',
  'The conversation history before this point was compacted',
];

/** Returns true if `content` looks like a system bootstrap, not a real user request. */
export function isSystemContext(content: string): boolean {
  if (!content) return false;
  for (const marker of SYSTEM_CONTEXT_MARKERS) {
    if (content.includes(marker)) return true;
  }
  if (content.includes('AGENTS.md') && content.includes('instructions')) return true;
  return false;
}

interface FrumCandidate {
  role?: string;
  content?: string;
}

/**
 * Find the first user message that isn't system context.
 * Returns trimmed content, or undefined if none found.
 */
export function findFirstRealUserMessage(messages: FrumCandidate[]): string | undefined {
  for (const msg of messages) {
    const role = msg.role?.toLowerCase();
    if (role !== 'user' && role !== 'human') continue;
    const content = msg.content?.trim();
    if (!content) continue;
    if (isSystemContext(content)) continue;
    return content;
  }
  return undefined;
}

/**
 * Format a FRUM as a one-line title preview. Collapses whitespace and truncates.
 */
export function formatFrumTitle(message: string, maxLen = 80): string {
  const collapsed = message.replace(/\s+/g, ' ').trim();
  if (collapsed.length <= maxLen) return collapsed;
  return `${collapsed.slice(0, maxLen - 3)}...`;
}
