// Perplexity web sync via the authenticated thread endpoints used by its UI.

import { fetchJson, textPart, toMs } from '../lib/common.js';

const BASE = 'https://www.perplexity.ai';
const LIST_URL = `${BASE}/rest/thread/list_ask_threads?version=2.18&source=default`;
const PAGE_SIZE = 20;
const OVERLAP_MS = 5 * 60 * 1000;

const headers = {
  accept: '*/*',
  'content-type': 'application/json',
  'x-app-apiclient': 'default',
  'x-app-apiversion': '2.18',
};

async function listThreads(sinceMs) {
  const threads = [];
  for (let offset = 0; offset <= 10_000; offset += PAGE_SIZE) {
    const data = await fetchJson(LIST_URL, {
      method: 'POST',
      headers,
      body: JSON.stringify({ limit: PAGE_SIZE, ascending: false, offset, search_term: '' }),
    });
    const items = Array.isArray(data) ? data : [];
    let reachedOld = false;
    for (const item of items) {
      const updatedAt = toMs(item.last_query_datetime);
      if (sinceMs && updatedAt && updatedAt <= sinceMs) {
        reachedOld = true;
        continue;
      }
      threads.push({ ...item, updatedAt });
    }
    if (reachedOld || items.length < PAGE_SIZE) break;
  }
  return threads;
}

function answerText(entry) {
  const markdown = (entry.blocks ?? []).find(block => block?.markdown_block)?.markdown_block;
  if (typeof markdown?.answer === 'string' && markdown.answer.trim()) return markdown.answer;
  if (Array.isArray(markdown?.chunks)) return markdown.chunks.join('').trim();
  return '';
}

async function readThread(summary) {
  const url = `${BASE}/rest/thread/${encodeURIComponent(summary.slug)}?with_parent_info=true&with_schematized_response=true&version=2.18&source=default&limit=100&offset=0&from_first=true`;
  const data = await fetchJson(url, { headers });
  const entries = Array.isArray(data?.entries) ? data.entries : [];
  const messages = [];
  for (const entry of entries) {
    const createdAt = toMs(entry.updated_datetime ?? entry.entry_updated_datetime) ?? summary.updatedAt;
    if (typeof entry.query_str === 'string' && entry.query_str.trim()) {
      messages.push({ role: 'user', content: entry.query_str, createdAt, model: null, parts: [textPart(entry.query_str)] });
    }
    const answer = answerText(entry);
    if (answer) {
      messages.push({ role: 'assistant', content: answer, createdAt, model: entry.display_model ?? null, parts: [textPart(answer)] });
    }
  }
  if (!messages.length) return null;
  const externalId = String(summary.context_uuid ?? summary.uuid ?? summary.slug);
  return {
    externalId,
    title: summary.title ?? null,
    createdAt: summary.updatedAt ?? messages[0].createdAt ?? Date.now(),
    updatedAt: summary.updatedAt,
    model: summary.display_model ?? null,
    provider: 'perplexity',
    messages,
    metadata: { url: `${BASE}/search/${summary.slug}`, slug: summary.slug },
  };
}

export async function syncPerplexity({ state, push, register = async () => {}, log, report = async () => {} }) {
  await report({ phase: 'discovering' });
  const lastSyncMs = state?.lastSyncMs ?? null;
  await register('perplexity-web', 'perplexity');
  const since = lastSyncMs ? lastSyncMs - OVERLAP_MS : null;
  const runStartedMs = Date.now();
  const summaries = await listThreads(since);
  await report({ phase: 'importing', detected: summaries.length, processed: 0 });
  let total = 0;
  let failures = 0;
  let processed = 0;
  let batch = [];
  for (const summary of summaries) {
    try {
      const conversation = await readThread(summary);
      if (conversation) batch.push(conversation);
    } catch (error) {
      failures++;
      log(`perplexity: skipping thread ${summary.slug}: ${error.message}`);
    }
    processed++;
    await report({ processed });
    if (batch.length >= 10) {
      total += await push('perplexity-web', 'perplexity', batch);
      batch = [];
    }
  }
  if (batch.length) total += await push('perplexity-web', 'perplexity', batch);
  await report({ phase: 'complete', processed });
  return {
    state: { lastSyncMs: failures ? lastSyncMs : runStartedMs },
    conversations: total,
  };
}

export const perplexityInternals = { answerText };
