// Gemini web sync through the authenticated batchexecute RPCs used by the web UI.

import { NotLoggedInError, textPart, toMs } from '../lib/common.js';

const BASE = 'https://gemini.google.com';
const OVERLAP_MS = 5 * 60 * 1000;
const PAGE_SIZE = 20;

function extractSession(html) {
  const read = key => html.match(new RegExp(`"${key}":"((?:\\\\.|[^"\\\\])*)"`))?.[1];
  const decode = value => value ? JSON.parse(`"${value}"`) : '';
  return {
    at: decode(read('SNlM0e')),
    sid: decode(read('FdrFJe')),
    bl: decode(read('cfb2h')),
  };
}

function parseRpcResponse(text, rpcId) {
  for (const line of text.split('\n')) {
    if (!line.startsWith('[[')) continue;
    try {
      const outer = JSON.parse(line);
      const entry = outer?.[0];
      if (entry?.[0] === 'wrb.fr' && entry?.[1] === rpcId && typeof entry?.[2] === 'string') {
        return JSON.parse(entry[2]);
      }
    } catch {
      // Streaming responses contain size lines and unrelated chunks.
    }
  }
  throw new Error(`Gemini returned an unreadable ${rpcId} response`);
}

async function getSession() {
  const res = await fetch(`${BASE}/app`, { credentials: 'include' });
  if (res.status === 401 || res.status === 403 || res.url.includes('accounts.google.com')) {
    throw new NotLoggedInError('gemini.google.com');
  }
  if (!res.ok) throw new Error(`GET ${BASE}/app -> ${res.status}`);
  const session = extractSession(await res.text());
  if (!session.at) throw new NotLoggedInError('gemini.google.com');
  const userIndex = res.url.match(/\/u\/(\d+)\//)?.[1] ?? null;
  return { ...session, userIndex };
}

async function batchExecute(session, rpcId, arg) {
  const reqId = Math.floor(Math.random() * 900000) + 100000;
  const accountPath = session.userIndex === null ? '' : `/u/${session.userIndex}`;
  const url = new URL(`${BASE}${accountPath}/_/BardChatUi/data/batchexecute`);
  url.searchParams.set('rpcids', rpcId);
  url.searchParams.set('source-path', '/app');
  url.searchParams.set('_reqid', String(reqId));
  url.searchParams.set('rt', 'c');
  if (session.bl) url.searchParams.set('bl', session.bl);
  if (session.sid) url.searchParams.set('f.sid', session.sid);

  const body = new URLSearchParams();
  body.set('f.req', JSON.stringify([[[rpcId, JSON.stringify(arg), null, 'generic']]]));
  body.set('at', session.at);
  const res = await fetch(url, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded;charset=utf-8', 'X-Same-Domain': '1' },
    body,
  });
  if (res.status === 401 || res.status === 403) throw new NotLoggedInError('gemini.google.com');
  if (!res.ok) throw new Error(`Gemini ${rpcId} -> ${res.status}`);
  return parseRpcResponse(await res.text(), rpcId);
}

async function listChats(session, sinceMs) {
  const chats = new Map();
  let cursor = null;
  for (let page = 0; page < 200; page++) {
    const arg = cursor === null ? [PAGE_SIZE, null, [0, null, 1]] : [PAGE_SIZE, cursor, [0, null, 1]];
    const data = await batchExecute(session, 'MaZiqc', arg);
    const nextCursor = data?.[1] ?? null;
    const items = Array.isArray(data?.[2]) ? data[2] : [];
    let reachedOld = false;
    for (const item of items) {
      const rawId = String(item?.[0] ?? '');
      if (!rawId) continue;
      const updatedAt = toMs(item?.[5]?.[0]);
      if (sinceMs && updatedAt && updatedAt <= sinceMs) {
        reachedOld = true;
        continue;
      }
      chats.set(rawId, {
        rawId,
        id: rawId.replace(/^c_/, ''),
        title: String(item?.[1] ?? 'Untitled conversation'),
        updatedAt,
      });
    }
    if (reachedOld || !nextCursor || nextCursor === cursor) break;
    cursor = nextCursor;
  }
  return [...chats.values()];
}

function assistantText(turn) {
  const direct = turn?.[3]?.[0]?.[0]?.[1]?.[0];
  if (typeof direct === 'string') return direct;
  return '';
}

async function readChat(session, summary) {
  const turns = [];
  let cursor = null;
  for (let page = 0; page < 200; page++) {
    const data = await batchExecute(session, 'hNvQHb', [summary.rawId, 20, cursor, 1, [0], [4], null, 1]);
    const pageTurns = Array.isArray(data?.[0]) ? data[0] : [];
    turns.push(...pageTurns);
    const nextCursor = data?.[1] ?? null;
    if (!nextCursor || nextCursor === cursor) break;
    cursor = nextCursor;
  }
  turns.reverse();

  const messages = [];
  for (const turn of turns) {
    const user = turn?.[2]?.[0]?.[0];
    const assistant = assistantText(turn);
    if (typeof user === 'string' && user.trim()) {
      messages.push({ role: 'user', content: user, createdAt: summary.updatedAt, model: null, parts: [textPart(user)] });
    }
    if (assistant.trim()) {
      messages.push({ role: 'assistant', content: assistant, createdAt: summary.updatedAt, model: 'gemini', parts: [textPart(assistant)] });
    }
  }
  if (messages.length === 0) return null;
  return {
    externalId: summary.id,
    title: summary.title,
    createdAt: summary.updatedAt ?? Date.now(),
    updatedAt: summary.updatedAt,
    model: 'gemini',
    provider: 'google',
    messages,
    metadata: { url: `${BASE}/app/${summary.id}` },
  };
}

export async function syncGemini({ state, push, register = async () => {}, log, report = async () => {} }) {
  await report({ phase: 'discovering' });
  const session = await getSession();
  await register('gemini-web', 'gemini');
  const lastSyncMs = state?.lastSyncMs ?? null;
  const since = lastSyncMs ? lastSyncMs - OVERLAP_MS : null;
  const runStartedMs = Date.now();
  const summaries = await listChats(session, since);
  await report({ phase: 'importing', detected: summaries.length, processed: 0 });
  let total = 0;
  let failures = 0;
  let processed = 0;
  let batch = [];
  for (const summary of summaries) {
    try {
      const conversation = await readChat(session, summary);
      if (conversation) batch.push(conversation);
    } catch (error) {
      failures++;
      log(`gemini: skipping conversation ${summary.id}: ${error.message}`);
    }
    processed++;
    await report({ processed });
    if (batch.length >= 10) {
      total += await push('gemini-web', 'gemini', batch);
      batch = [];
    }
  }
  if (batch.length) total += await push('gemini-web', 'gemini', batch);
  await report({ phase: 'complete', processed });
  return {
    state: { lastSyncMs: failures ? lastSyncMs : runStartedMs },
    conversations: total,
  };
}

export const geminiInternals = { extractSession, parseRpcResponse };
