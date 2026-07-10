// ChatGPT provider: syncs conversations (personal + Teams workspaces) via the
// chatgpt.com backend API, authenticated with the browser session cookies.

import {
  NotLoggedInError,
  fetchJson,
  shortId,
  sleep,
  textPart,
  thinkingPart,
  toMs,
} from '../lib/common.js';

const BASE = 'https://chatgpt.com';
const PAGE_SIZE = 50;
// Refetch a little history on every run so near-simultaneous edits are not
// missed between polls.
const OVERLAP_MS = 5 * 60 * 1000;
// Pace detail requests to stay under ChatGPT's rate limit. Backoff in
// fetchJson handles bursts, but pacing avoids tripping 429 in the first place.
const THROTTLE_MS = 400;

async function getAccessToken() {
  const data = await fetchJson(`${BASE}/api/auth/session`);
  if (!data?.accessToken) throw new NotLoggedInError('chatgpt.com');
  return data.accessToken;
}

function authHeaders(token, accountId) {
  const headers = { Authorization: `Bearer ${token}` };
  if (accountId) headers['chatgpt-account-id'] = accountId;
  return headers;
}

/** Enumerate accounts (personal + Teams workspaces). Falls back to the
 * default account when the accounts endpoint is unavailable. */
async function listAccounts(token) {
  try {
    const data = await fetchJson(`${BASE}/backend-api/accounts/check/v4-2023-04-27`, {
      headers: authHeaders(token),
    });
    const accounts = [];
    for (const entry of Object.values(data?.accounts ?? {})) {
      const account = entry?.account ?? entry;
      const id = account?.account_id ?? account?.id;
      if (!id || account?.is_deactivated) continue;
      if (accounts.some(a => a.id === id)) continue;
      accounts.push({
        id,
        name: account?.organization_name || account?.name || account?.plan_type || 'personal',
        isWorkspace: (account?.structure ?? account?.plan_type) === 'workspace',
      });
    }
    if (accounts.length > 0) return accounts;
  } catch {
    // Endpoint shape changed or unavailable: sync the default account only.
  }
  return [{ id: null, name: 'default', isWorkspace: false }];
}

async function* listUpdatedConversations(token, accountId, sinceMs) {
  for (let offset = 0; ; offset += PAGE_SIZE) {
    const data = await fetchJson(
      `${BASE}/backend-api/conversations?offset=${offset}&limit=${PAGE_SIZE}&order=updated`,
      { headers: authHeaders(token, accountId) }
    );
    const items = data?.items ?? [];
    for (const item of items) {
      const updatedMs = toMs(item.update_time);
      if (sinceMs && updatedMs && updatedMs <= sinceMs) return;
      yield item;
    }
    if (items.length < PAGE_SIZE || offset + PAGE_SIZE >= (data?.total ?? 0)) return;
  }
}

/** Linearize the active branch of the mapping tree (current_node -> root). */
function linearize(detail) {
  const mapping = detail?.mapping ?? {};
  const chain = [];
  let nodeId = detail?.current_node;
  while (nodeId && mapping[nodeId]) {
    chain.push(mapping[nodeId]);
    nodeId = mapping[nodeId].parent;
  }
  chain.reverse();
  return chain;
}

function extractMessage(node) {
  const msg = node?.message;
  if (!msg) return null;
  const role = msg.author?.role;
  if (role !== 'user' && role !== 'assistant') return null;
  if (msg.metadata?.is_visually_hidden_from_conversation) return null;

  const parts = [];
  const texts = [];
  const content = msg.content ?? {};

  if (content.content_type === 'text' || content.content_type === 'multimodal_text') {
    for (const part of content.parts ?? []) {
      if (typeof part === 'string' && part.trim()) {
        texts.push(part);
        parts.push(textPart(part));
      } else if (part?.content_type === 'image_asset_pointer') {
        const ref = `[image] ${part.asset_pointer ?? ''}`.trim();
        texts.push(ref);
        parts.push(textPart(ref));
      }
    }
  } else if (content.content_type === 'code' && content.text) {
    const code = `\`\`\`${content.language ?? ''}\n${content.text}\n\`\`\``;
    texts.push(code);
    parts.push(textPart(code));
  } else if (content.content_type === 'thoughts') {
    for (const thought of content.thoughts ?? []) {
      const text = thought?.content ?? thought?.summary;
      if (text) parts.push(thinkingPart(text));
    }
  }

  const contentStr = texts.join('\n').trim();
  if (!contentStr && parts.length === 0) return null;

  return {
    role,
    content: contentStr,
    createdAt: toMs(msg.create_time),
    model: msg.metadata?.model_slug ?? null,
    parts,
  };
}

function toParsedConversation(detail, conversationId, accountId) {
  const messages = linearize(detail)
    .map(extractMessage)
    .filter(Boolean);
  if (messages.length === 0) return null;

  const model = [...messages].reverse().find(m => m.model)?.model ?? null;

  return {
    externalId: conversationId,
    title: detail?.title ?? null,
    createdAt: toMs(detail?.create_time) ?? messages[0].createdAt ?? Date.now(),
    updatedAt: toMs(detail?.update_time),
    model,
    provider: 'openai',
    messages,
    metadata: {
      url: `${BASE}/c/${conversationId}`,
      ...(accountId ? { accountId } : {}),
    },
  };
}

export async function syncChatGPT({ state, push, register = async () => {}, log, report = async () => {} }) {
  await report({ phase: 'discovering' });
  const token = await getAccessToken();
  const accounts = await listAccounts(token);
  const newState = { ...state, accounts: {} };
  let total = 0;
  let detected = 0;
  let processed = 0;

  for (const account of accounts) {
    const key = account.id ? shortId(account.id) : 'default';
    const sourceId = account.id && account.isWorkspace ? `chatgpt-web-${key}` : 'chatgpt-web';
    await register(sourceId, 'chatgpt-web');
    const lastSyncMs = state?.accounts?.[key]?.lastSyncMs ?? null;
    const since = lastSyncMs ? lastSyncMs - OVERLAP_MS : null;
    const runStartedMs = Date.now();
    let failures = 0;

    let batch = [];
    let first = true;
    for await (const item of listUpdatedConversations(token, account.id, since)) {
      detected++;
      await report({ phase: 'importing', detected, processed });
      if (!first) await sleep(THROTTLE_MS);
      first = false;
      try {
        const detail = await fetchJson(`${BASE}/backend-api/conversation/${item.id}`, {
          headers: authHeaders(token, account.id),
        });
        const conv = toParsedConversation(detail, item.id, account.id);
        if (conv) batch.push(conv);
      } catch (err) {
        failures++;
        log(`chatgpt: skipping conversation ${item.id}: ${err.message}`);
      }
      processed++;
      await report({ detected, processed });
      if (batch.length >= 10) {
        total += await push(sourceId, 'chatgpt-web', batch);
        batch = [];
      }
    }
    if (batch.length > 0) {
      total += await push(sourceId, 'chatgpt-web', batch);
    }

    // Only advance the watermark on a clean run. If any conversation failed
    // (e.g. persistent 429), keep the old watermark so the next sync retries
    // them rather than skipping past them forever. Re-pushes dedupe server-side.
    if (failures > 0) {
      log(`chatgpt: ${failures} conversation(s) failed; keeping watermark to retry next run`);
    }
    newState.accounts[key] = {
      lastSyncMs: failures > 0 ? lastSyncMs : runStartedMs,
      name: account.name,
    };
  }

  await report({ phase: 'complete', detected, processed });

  return { state: newState, conversations: total };
}
