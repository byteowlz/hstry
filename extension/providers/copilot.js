// M365 Copilot Chat sync through the authenticated web session.
//
// The Copilot web app routes data requests through a client-action proxy on
// m365.cloud.microsoft. We replay the same GET/POST the app makes, with the
// routing headers and cookies of the signed-in browser, to list conversations
// and pull each conversation's messages. No Graph app registration or admin
// consent is required: the user is authenticated to Copilot in this browser.

import { NotLoggedInError, textPart, toMs } from '../lib/common.js';

const BASE = 'https://m365.cloud.microsoft';
const OVERLAP_MS = 5 * 60 * 1000;
// Pace detail requests to stay under Copilot's rate limits.
const THROTTLE_MS = 400;
const BATCH = 10;

const HOST_CONTEXT = JSON.stringify({
  clientPlatform: 'web',
  hostName: 'officeweb',
  appName: 'SSR',
  appMode: 'default',
});

function baseHeaders(sessionId) {
  return {
    Accept: 'application/json',
    'Content-Type': 'application/json',
    'X-Route-Id': 'chat',
    'X-Session-Id': sessionId,
    'x-host-context': HOST_CONTEXT,
  };
}

/** Parse a Copilot timestamp. Handles ISO strings with >3 fractional seconds
 * (e.g. "2026-09-14T14:32:18.6850423+00:00"), which Date.parse rejects. */
function toTime(value) {
  if (typeof value === 'string') {
    const normalized = value.replace(/(\.\d{3})\d+/, '$1');
    return toMs(normalized);
  }
  return toMs(value);
}

/** Throws NotLoggedInError when the response is a login redirect, an auth
 * error, or the SPA shell (HTML) that the proxy returns for unauthenticated
 * or wrong-routed requests. */
async function readJson(res, what) {
  const ct = res.headers.get('content-type') || '';
  if (ct.includes('html')) throw new NotLoggedInError('m365.cloud.microsoft');
  if (res.redirected && res.url.includes('login.microsoftonline.com')) {
    throw new NotLoggedInError('m365.cloud.microsoft');
  }
  if (res.status === 401 || res.status === 403) throw new NotLoggedInError('m365.cloud.microsoft');
  if (!res.ok) throw new Error(`${what} -> ${res.status}`);
  return res.json();
}

async function listConversations(sessionId) {
  const res = await fetch(`${BASE}/chat?es=SSR&redirfrom=ccmToMcm`, {
    method: 'POST',
    credentials: 'include',
    headers: baseHeaders(sessionId),
    body: JSON.stringify({
      action: 'RefreshNavPane',
      conversationHistoryFilter: null,
      skipNotebooks: false,
      skipAgentListCache: true,
      enableLastMessage: false,
    }),
  });
  const data = await readJson(res, 'RefreshNavPane');
  return data?.store?.conversationPageHistoryList?.chats ?? [];
}

async function fetchConversation(sessionId, conversationId) {
  const res = await fetch(`${BASE}/chat/conversation/${encodeURIComponent(conversationId)}`, {
    method: 'GET',
    credentials: 'include',
    headers: baseHeaders(sessionId),
  });
  const data = await readJson(res, `conversation ${conversationId}`);
  return data?.store?.rawConversationResponse ?? null;
}

/** Assistant responses live in adaptive cards; join visible text blocks. */
function botText(message) {
  const parts = [];
  for (const card of message?.adaptiveCards ?? []) {
    for (const block of card?.body ?? []) {
      const text = block?.text ?? block?.title ?? '';
      if (typeof text === 'string' && text.trim()) parts.push(text.trim());
    }
  }
  return parts.join('\n\n');
}

function toConversation(item, raw) {
  const messages = [];
  for (const message of raw?.messages ?? []) {
    const author = message?.author;
    const createdAt = toTime(message?.createdAt);
    if (author === 'user') {
      const text = (message?.text ?? '').trim();
      if (!text) continue;
      messages.push({ role: 'user', content: text, createdAt, model: null, parts: [textPart(text)] });
    } else if (author === 'bot') {
      const text = botText(message).trim();
      if (!text) continue;
      messages.push({ role: 'assistant', content: text, createdAt, model: null, parts: [textPart(text)] });
    }
  }
  if (messages.length === 0) return null;

  return {
    externalId: String(item.conversationId),
    title: item.chatName || null,
    createdAt: toTime(item.createTimeUtc) ?? messages[0].createdAt ?? Date.now(),
    updatedAt: toTime(item.updateTimeUtc),
    model: null,
    provider: 'microsoft',
    messages,
    metadata: {
      url: `${BASE}/chat/conversation/${item.conversationId}`,
      ...(item.threadId ? { threadId: item.threadId } : {}),
    },
  };
}

export async function syncCopilot({ state, push, register = async () => {}, log, report = async () => {} }) {
  await report({ phase: 'discovering' });
  // A fresh routing/session id per sync is what the web app issues per load.
  const sessionId = crypto.randomUUID();
  await register('m365-copilot', 'm365-copilot');
  const lastSyncMs = state?.lastSyncMs ?? null;
  const since = lastSyncMs ? lastSyncMs - OVERLAP_MS : null;
  const runStartedMs = Date.now();

  const chats = await listConversations(sessionId);
  const incoming = chats.filter(item => {
    const updatedAt = toTime(item.updateTimeUtc);
    return !(since && updatedAt && updatedAt <= since);
  });
  await report({ phase: 'importing', detected: incoming.length, processed: 0 });

  let total = 0;
  let failures = 0;
  let processed = 0;
  let batch = [];
  for (const item of incoming) {
    try {
      const raw = await fetchConversation(sessionId, item.conversationId);
      const conversation = toConversation(item, raw);
      if (conversation) batch.push(conversation);
    } catch (error) {
      failures++;
      log(`copilot: skipping conversation ${item.conversationId}: ${error.message}`);
    }
    processed++;
    await report({ detected: incoming.length, processed });
    if (batch.length >= BATCH) {
      total += await push('m365-copilot', 'm365-copilot', batch);
      batch = [];
    }
    if (processed < incoming.length) await sleep(THROTTLE_MS);
  }
  if (batch.length > 0) {
    total += await push('m365-copilot', 'm365-copilot', batch);
  }

  await report({ phase: 'complete', detected: incoming.length, processed });

  return {
    // Only advance the watermark on a clean run so a later retry re-pulls
    // anything that failed. Re-pushes dedupe server-side.
    state: { lastSyncMs: failures > 0 ? lastSyncMs : runStartedMs },
    conversations: total,
  };
}

async function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}