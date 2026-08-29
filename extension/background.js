// hstry sync service worker: polls AI chat platforms on an alarm and pushes
// new/updated conversations to a local hstry-api instance (POST /ingest).

import { NotLoggedInError } from './lib/common.js';
import { DEFAULT_PROVIDER_SETTINGS, PROVIDERS } from './providers/index.js';

const DEFAULT_SETTINGS = {
  port: 3000,
  token: '',
  intervalMinutes: 15,
  providers: DEFAULT_PROVIDER_SETTINGS,
};

const ALARM_NAME = 'hstry-sync';
const CONTINUE_ALARM_PREFIX = 'hstry-sync-continue-';
const activeProviders = new Set();
let statusWriteQueue = Promise.resolve();

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

function setStatusEntry(name, entry) {
  statusWriteQueue = statusWriteQueue.then(async () => {
    const status = await getStatus();
    status[name] = entry;
    await setStatus(status);
  });
  return statusWriteQueue;
}

async function clearInterruptedRuns() {
  const status = await getStatus();
  let changed = false;
  for (const entry of Object.values(status)) {
    if (!entry?.running) continue;
    entry.running = false;
    entry.lastError = 'Previous sync was interrupted. Run it again.';
    entry.progress = { ...entry.progress, phase: 'failed' };
    changed = true;
  }
  if (changed) await setStatus(status);
}

const startupReady = clearInterruptedRuns();

function makePush(settings, onResult = async () => {}) {
  const url = `http://127.0.0.1:${settings.port}/ingest`;
  return async function push(sourceId, adapter, conversations) {
    let res;
    try {
      res = await fetch(url, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(settings.token ? { Authorization: `Bearer ${settings.token}` } : {}),
        },
        body: JSON.stringify({ source: sourceId, adapter, conversations }),
      });
    } catch {
      // fetch() rejects with a bare "Failed to fetch" when nothing is
      // listening. Turn it into something actionable.
      throw new Error(
        `cannot reach hstry-api at ${url} — is it running on port ${settings.port}? ` +
          `start it with: hstry-api --port ${settings.port}` +
          (settings.token ? ' --token <your-token>' : '')
      );
    }
    if (res.status === 401) {
      throw new Error(`hstry-api rejected the token (401) — extension token must match the server's --token / HSTRY_API_TOKEN`);
    }
    if (!res.ok) {
      throw new Error(`hstry-api /ingest -> ${res.status}`);
    }
    const data = await res.json();
    await onResult({
      accepted: data?.conversations ?? conversations.length,
      created: data?.created ?? 0,
      updated: data?.updated ?? 0,
    });
    return data?.conversations ?? conversations.length;
  };
}

function makeRegister(settings) {
  const url = `http://127.0.0.1:${settings.port}/sources`;
  return async function register(source, adapter) {
    let res;
    try {
      res = await fetch(url, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(settings.token ? { Authorization: `Bearer ${settings.token}` } : {}),
        },
        body: JSON.stringify({ source, adapter }),
      });
    } catch {
      throw new Error(`cannot reach hstry-api at ${url}`);
    }
    if (res.status === 401) throw new Error('hstry-api rejected the token (401)');
    if (!res.ok) throw new Error(`hstry-api /sources -> ${res.status}`);
    return res.json();
  };
}

async function registerEnabledSources(settings) {
  settings ??= await getSettings();
  const register = makeRegister(settings);
  const results = {};
  for (const [id, provider] of Object.entries(PROVIDERS)) {
    if (!settings.providers[id]) continue;
    try {
      results[id] = await register(provider.sourceId, provider.adapter);
    } catch (error) {
      results[id] = { error: error.message };
    }
  }
  return results;
}

async function checkApi() {
  const settings = await getSettings();
  const url = `http://127.0.0.1:${settings.port}/health`;
  try {
    const res = await fetch(url);
    if (!res.ok) return { ok: false, error: `hstry-api /health -> ${res.status}`, url };
    const data = await res.json();
    return { ok: data?.status === 'ok', url };
  } catch {
    return { ok: false, error: `cannot reach hstry-api on port ${settings.port}`, url };
  }
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

async function scheduleContinuation(providerName) {
  await chrome.alarms.create(`${CONTINUE_ALARM_PREFIX}${providerName}`, {
    delayInMinutes: 0.1,
  });
}

async function runSync(
  trigger,
  { onlyProvider = null, full = false, onStarted = async () => {} } = {}
) {
  await startupReady;
  const settings = await getSettings();
  const status = await getStatus();
  const selected = Object.entries(PROVIDERS).filter(
    ([name]) =>
      settings.providers[name] &&
      (!onlyProvider || name === onlyProvider) &&
      !activeProviders.has(name)
  );
  if (selected.length === 0) return false;
  for (const [name] of selected) activeProviders.add(name);
  await chrome.action.setBadgeText({ text: '…' });

  let anyError = false;
  let startedReported = false;
  await Promise.all(
    selected.map(async ([name, provider]) => {
      const entry = {
        ...(status[name] ?? {}),
        ...(full && name === onlyProvider ? { state: {} } : {}),
        lastRunMs: Date.now(),
        trigger,
        running: true,
        progress: {
          phase: 'discovering',
          detected: 0,
          processed: 0,
          accepted: 0,
          created: 0,
          updated: 0,
        },
      };
      await setStatusEntry(name, entry);
      if (!startedReported) {
        startedReported = true;
        await onStarted();
      }
      const report = async progress => {
        entry.progress = { ...entry.progress, ...progress };
        await setStatusEntry(name, entry);
      };
      const push = makePush(settings, async result => {
        await report({
          accepted: entry.progress.accepted + result.accepted,
          created: entry.progress.created + result.created,
          updated: entry.progress.updated + result.updated,
        });
      });
      const register = makeRegister(settings);
      try {
        const result = await provider.sync({
          state: entry.state ?? {},
          push,
          register,
          report,
          log: message => console.warn(`[hstry-sync] ${message}`),
        });
        entry.state = result.state;
        entry.lastSuccessMs = Date.now();
        entry.lastCount = result.conversations;
        entry.lastError = null;
        entry.running = false;
        entry.continuationPending = Boolean(result.hasMore);
        entry.progress = {
          ...entry.progress,
          phase: result.hasMore ? 'queued' : 'complete',
        };
        if (result.hasMore) await scheduleContinuation(name);
      } catch (err) {
        anyError = true;
        entry.lastError =
          err instanceof NotLoggedInError ? `${err.message} — open the site and log in` : err.message;
        entry.running = false;
        entry.progress = { ...entry.progress, phase: 'failed' };
        console.warn(`[hstry-sync] ${name} failed:`, err);
      } finally {
        activeProviders.delete(name);
        await setStatusEntry(name, entry);
      }
    })
  );

  await chrome.action.setBadgeText({ text: anyError ? '!' : activeProviders.size > 0 ? '…' : '' });
  if (anyError) {
    await chrome.action.setBadgeBackgroundColor({ color: '#cc3333' });
  }
  return true;
}

chrome.runtime.onInstalled.addListener(() => {
  ensureAlarm();
  registerEnabledSources();
});
chrome.runtime.onStartup.addListener(() => {
  ensureAlarm();
  registerEnabledSources();
});

chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm.name === ALARM_NAME) {
    runSync('alarm');
    return;
  }
  if (alarm.name.startsWith(CONTINUE_ALARM_PREFIX)) {
    const providerName = alarm.name.slice(CONTINUE_ALARM_PREFIX.length);
    if (Object.hasOwn(PROVIDERS, providerName)) {
      runSync('continuation', { onlyProvider: providerName }).then(started => {
        if (!started) scheduleContinuation(providerName);
      });
    }
  }
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
  if (message?.type === 'fullSyncProvider') {
    const providerName = message.provider;
    if (!Object.hasOwn(PROVIDERS, providerName)) {
      sendResponse({ ok: false, error: 'Unknown provider' });
      return false;
    }
    if (activeProviders.has(providerName)) {
      sendResponse({ ok: false, error: 'This provider is already syncing' });
      return false;
    }
    getSettings()
      .then(settings => {
        if (!settings.providers[providerName]) {
          sendResponse({ ok: false, error: 'Enable this provider before syncing' });
          return;
        }
        if (activeProviders.has(providerName)) {
          sendResponse({ ok: false, error: 'This provider is already syncing' });
          return;
        }
        let responded = false;
        const respond = result => {
          if (responded) return;
          responded = true;
          sendResponse(result);
        };
        runSync('full', {
          onlyProvider: providerName,
          full: true,
          onStarted: async () => respond({ ok: true }),
        }).catch(error => respond({ ok: false, error: error.message }));
      })
      .catch(error => sendResponse({ ok: false, error: error.message }));
    return true;
  }
  if (message?.type === 'settingsChanged') {
    Promise.all([ensureAlarm(), registerEnabledSources()]).then(([, sources]) => {
      sendResponse({ ok: true, sources });
    });
    return true;
  }
  if (message?.type === 'checkApi') {
    checkApi().then(sendResponse);
    return true;
  }
  return false;
});
