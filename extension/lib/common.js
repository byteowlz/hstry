// Shared helpers for hstry sync providers.

let partCounter = 0;

export function textPart(text) {
  return { id: `p-${++partCounter}`, type: 'text', text };
}

export function thinkingPart(text) {
  return { id: `p-${++partCounter}`, type: 'thinking', text };
}

export function toolCallPart(toolCallId, name, input) {
  return { id: `p-${++partCounter}`, type: 'tool_call', toolCallId, name, input };
}

export function toolResultPart(toolCallId, output) {
  return { id: `p-${++partCounter}`, type: 'tool_result', toolCallId, output };
}

export class NotLoggedInError extends Error {
  constructor(service) {
    super(`${service}: not logged in`);
    this.name = 'NotLoggedInError';
  }
}

export async function fetchJson(url, init = {}) {
  const res = await fetch(url, { credentials: 'include', ...init });
  if (res.status === 401 || res.status === 403) {
    throw new NotLoggedInError(new URL(url).hostname);
  }
  if (!res.ok) {
    throw new Error(`${init.method ?? 'GET'} ${url} -> ${res.status}`);
  }
  return res.json();
}

export function toMs(value) {
  if (typeof value === 'number' && Number.isFinite(value)) {
    // Unix seconds (possibly fractional) vs milliseconds.
    return Math.floor(value < 1e12 ? value * 1000 : value);
  }
  if (typeof value === 'string') {
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? null : parsed;
  }
  return null;
}

export function shortId(id) {
  return String(id).replace(/[^a-zA-Z0-9]/g, '').slice(0, 8).toLowerCase();
}
