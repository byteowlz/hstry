// hstry sync service worker: polls AI chat platforms on an alarm and pushes
// new/updated conversations to a local hstry-api instance (POST /ingest).

import { NotLoggedInError } from './lib/common.js';
import { syncChatGPT } from './providers/chatgpt.js';
import { syncClaude } from './providers/claude.js';

const PROVIDERS = {
  chatgpt: syncChatGPT,
  claude: syncClaude,
};

const DEFAULT_SETTINGS = {
  port: 3000,
  token: '',
  intervalMinutes: 15,
  providers: { chatgpt: true, claude: true },
};

const ALARM_NAME = 'hstry-sync';
let syncing = false;

async function getSettings() {
  const { settings } = await chrome.storage.local.get('settings');
  return {
    ...DEFAULT_SETTINGS,
    ...settings,
    providers: { ...DEFAULT_SETTINGS.providers, ...settings?.providers },
  };
}

async function getStatus() {
  const { status } = await chrome.storage.local.get('status');
  return status ?? {};
}

async function setStatus(status) {
  await chrome.storage.local.set({ status });
}

function makePush(settings) {
  return async function push(sourceId, adapter, conversations) {
    const res = await fetch(`http://127.0.0.1:${settings.port}/ingest`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(settings.token ? { Authorization: `Bearer ${settings.token}` } : {}),
      },
      body: JSON.stringify({ source: sourceId, adapter, conversations }),
    });
    if (!res.ok) {
      throw new Error(`hstry-api /ingest -> ${res.status} (is hstry-api running on port ${settings.port}?)`);
    }
    const data = await res.json();
    return data?.conversations ?? conversations.length;
  };
}

async function ensureAlarm() {
  const settings = await getSettings();
  const existing = await chrome.alarms.get(ALARM_NAME);
  if (!existing || existing.periodInMinutes !== settings.intervalMinutes) {
    await chrome.alarms.create(ALARM_NAME, {
      periodInMinutes: settings.intervalMinutes,
      delayInMinutes: 1,
    });
  }
}

async function runSync(trigger) {
  if (syncing) return;
  syncing = true;
  await chrome.action.setBadgeText({ text: '…' });

  const settings = await getSettings();
  const push = makePush(settings);
  const status = await getStatus();
  let anyError = false;

  for (const [name, sync] of Object.entries(PROVIDERS)) {
    if (!settings.providers[name]) continue;

    const entry = { ...(status[name] ?? {}), lastRunMs: Date.now(), trigger };
    try {
      const result = await sync({
        state: status[name]?.state ?? {},
        push,
        log: message => console.warn(`[hstry-sync] ${message}`),
      });
      entry.state = result.state;
      entry.lastSuccessMs = Date.now();
      entry.lastCount = result.conversations;
      entry.lastError = null;
    } catch (err) {
      anyError = true;
      entry.lastError =
        err instanceof NotLoggedInError ? `${err.message} — open the site and log in` : err.message;
      console.warn(`[hstry-sync] ${name} failed:`, err);
    }
    status[name] = entry;
    await setStatus(status);
  }

  await chrome.action.setBadgeText({ text: anyError ? '!' : '' });
  if (anyError) {
    await chrome.action.setBadgeBackgroundColor({ color: '#cc3333' });
  }
  syncing = false;
}

chrome.runtime.onInstalled.addListener(ensureAlarm);
chrome.runtime.onStartup.addListener(ensureAlarm);

chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm.name === ALARM_NAME) runSync('alarm');
});

chrome.action.onClicked.addListener(() => runSync('manual'));

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  // Acknowledge synchronously and let the work continue in the background. A
  // full sync can outlive the message channel (or the worker), so we must not
  // hold the channel open waiting for it — status flows to the UI via
  // chrome.storage instead. Returning true here would reproduce the
  // "message channel closed before a response was received" error.
  if (message?.type === 'syncNow') {
    runSync('manual');
    sendResponse({ ok: true });
    return false;
  }
  if (message?.type === 'settingsChanged') {
    ensureAlarm();
    sendResponse({ ok: true });
    return false;
  }
  return false;
});
