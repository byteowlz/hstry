// Claude provider: syncs conversations from claude.ai via its web API,
// authenticated with the browser session cookies (no bearer token needed).

import {
  fetchJson,
  shortId,
  sleep,
  textPart,
  thinkingPart,
  toolCallPart,
  toolResultPart,
  toMs,
} from '../lib/common.js';

const BASE = 'https://claude.ai';
const PAGE_SIZE = 50;
const OVERLAP_MS = 5 * 60 * 1000;
const THROTTLE_MS = 400;

async function listOrganizations() {
  const orgs = await fetchJson(`${BASE}/api/organizations`);
  if (!Array.isArray(orgs)) return [];
  return orgs
    .filter(org => org?.uuid)
    .filter(org => !Array.isArray(org.capabilities) || org.capabilities.includes('chat'))
    .map(org => ({ id: org.uuid, name: org.name ?? 'organization' }));
}

async function* listUpdatedConversations(orgId, sinceMs) {
  for (let offset = 0; ; offset += PAGE_SIZE) {
    const data = await fetchJson(
      `${BASE}/api/organizations/${orgId}/chat_conversations?limit=${PAGE_SIZE}&offset=${offset}&consistency=eventual`
    );
    const items = Array.isArray(data) ? data : (data?.data ?? []);
    for (const item of items) {
      const updatedMs = toMs(item.updated_at);
      if (sinceMs && updatedMs && updatedMs <= sinceMs) return;
      yield item;
    }
    if (items.length < PAGE_SIZE) return;
  }
}

function extractMessage(msg) {
  const sender = msg?.sender;
  const role = sender === 'human' ? 'user' : sender === 'assistant' ? 'assistant' : null;
  if (!role) return null;

  const parts = [];
  const texts = [];

  const blocks = Array.isArray(msg.content) ? msg.content : [];
  for (const block of blocks) {
    if (!block?.type) continue;
    if (block.type === 'text' && block.text) {
      texts.push(block.text);
      parts.push(textPart(block.text));
    } else if (block.type === 'thinking' && (block.thinking || block.text)) {
      parts.push(thinkingPart(block.thinking ?? block.text));
    } else if (block.type === 'tool_use' && block.name) {
      parts.push(toolCallPart(block.id ?? block.name, block.name, block.input));
    } else if (block.type === 'tool_result') {
      const output =
        typeof block.content === 'string' ? block.content : JSON.stringify(block.content ?? null);
      parts.push(toolResultPart(block.tool_use_id ?? 'unknown', output));
    }
  }

  let content = texts.join('\n').trim();
  if (!content && typeof msg.text === 'string') {
    content = msg.text.trim();
  }
  if (!content && parts.length === 0) return null;

  return {
    role,
    content,
    createdAt: toMs(msg.created_at),
    model: null,
    parts,
  };
}

function toParsedConversation(detail, orgId) {
  const messages = (detail?.chat_messages ?? []).map(extractMessage).filter(Boolean);
  if (messages.length === 0) return null;

  return {
    externalId: detail.uuid,
    title: detail.name || null,
    createdAt: toMs(detail.created_at) ?? messages[0].createdAt ?? Date.now(),
    updatedAt: toMs(detail.updated_at),
    model: detail.model ?? null,
    provider: 'anthropic',
    messages,
    metadata: {
      url: `${BASE}/chat/${detail.uuid}`,
      orgId,
      ...(detail.summary ? { summary: detail.summary } : {}),
    },
  };
}

export async function syncClaude({ state, push, log }) {
  const orgs = await listOrganizations();
  const newState = { ...state, orgs: {} };
  let total = 0;
  const multiOrg = orgs.length > 1;

  for (const org of orgs) {
    const key = shortId(org.id);
    const sourceId = multiOrg ? `claude-web-${key}` : 'claude-web';
    const lastSyncMs = state?.orgs?.[key]?.lastSyncMs ?? null;
    const since = lastSyncMs ? lastSyncMs - OVERLAP_MS : null;
    const runStartedMs = Date.now();
    let failures = 0;

    let batch = [];
    let first = true;
    for await (const item of listUpdatedConversations(org.id, since)) {
      if (!first) await sleep(THROTTLE_MS);
      first = false;
      try {
        const detail = await fetchJson(
          `${BASE}/api/organizations/${org.id}/chat_conversations/${item.uuid}?tree=True&rendering_mode=messages&render_all_tools=true&consistency=eventual`
        );
        const conv = toParsedConversation(detail, org.id);
        if (conv) batch.push(conv);
      } catch (err) {
        failures++;
        log(`claude: skipping conversation ${item.uuid}: ${err.message}`);
      }
      if (batch.length >= 10) {
        total += await push(sourceId, 'claude-web', batch);
        batch = [];
      }
    }
    if (batch.length > 0) {
      total += await push(sourceId, 'claude-web', batch);
    }

    // Keep the watermark unless the run was clean, so failures are retried.
    if (failures > 0) {
      log(`claude: ${failures} conversation(s) failed; keeping watermark to retry next run`);
    }
    newState.orgs[key] = {
      lastSyncMs: failures > 0 ? lastSyncMs : runStartedMs,
      name: org.name,
    };
  }

  return { state: newState, conversations: total };
}
