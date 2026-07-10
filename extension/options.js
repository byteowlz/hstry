import { DEFAULT_PROVIDER_SETTINGS, PROVIDERS, formatTime, normalizeSettings, providerState } from './ui.js';

const DEFAULTS = { port: 3000, token: '', intervalMinutes: 15, providers: DEFAULT_PROVIDER_SETTINGS };
const el = id => document.getElementById(id);
let currentSettings = DEFAULTS;
let apiConnected = false;

async function load() {
  const { settings } = await chrome.storage.local.get('settings');
  currentSettings = { ...DEFAULTS, ...settings, providers: { ...DEFAULTS.providers, ...settings?.providers } };
  el('port').value = currentSettings.port;
  el('token').value = currentSettings.token;
  el('interval').value = currentSettings.intervalMinutes;
  for (const id of Object.keys(PROVIDERS)) el(`provider-${id}`).checked = currentSettings.providers[id];
  await renderStatus();
}

function readFormSettings() {
  return normalizeSettings({
    port: el('port').value,
    token: el('token').value.trim(),
    intervalMinutes: el('interval').value,
    providers: Object.fromEntries(Object.keys(PROVIDERS).map(id => [id, el(`provider-${id}`).checked])),
  }, DEFAULTS);
}

function renderProviderControls() {
  const list = el('provider-list');
  list.replaceChildren();
  for (const [id, provider] of Object.entries(PROVIDERS)) {
    const row = document.createElement('div');
    row.className = 'provider-row';
    row.innerHTML = `<input id="provider-${id}" type="checkbox" /><label for="provider-${id}">${provider.name}<small></small></label><span class="provider-domain"></span>`;
    row.querySelector('small').textContent = provider.description;
    row.querySelector('.provider-domain').textContent = provider.site;
    list.appendChild(row);
  }
}

async function save({ announce = true } = {}) {
  currentSettings = readFormSettings();
  await chrome.storage.local.set({ settings: currentSettings });
  await chrome.runtime.sendMessage({ type: 'settingsChanged' });
  updateSetupCommand();
  if (announce) {
    el('saved').textContent = 'Settings saved';
    setTimeout(() => (el('saved').textContent = ''), 1800);
  }
  await renderStatus();
}

function updateSetupCommand() {
  const settings = readFormSettings();
  el('setup-command').textContent = `hstry-api --port ${settings.port}${settings.token ? ' --token <token>' : ''}`;
}

async function checkConnection() {
  const indicator = el('connection-result');
  const dot = el('connection-dot');
  indicator.className = 'connection-result checking';
  dot.className = 'status-dot checking';
  indicator.textContent = `Checking http://127.0.0.1:${currentSettings.port}…`;
  try {
    const result = await chrome.runtime.sendMessage({ type: 'checkApi' });
    if (!result?.ok) throw new Error(result?.error);
    indicator.className = 'connection-result success';
    dot.className = 'status-dot success';
    indicator.textContent = `Connected to ${result.url}`;
    apiConnected = true;
    await renderStatus();
    return true;
  } catch {
    indicator.className = 'connection-result error';
    dot.className = 'status-dot error';
    indicator.textContent = `Cannot reach http://127.0.0.1:${currentSettings.port}. Check that hstry-api is running on this exact port.`;
    apiConnected = false;
    await renderStatus();
    return false;
  }
}

async function renderStatus() {
  const { status } = await chrome.storage.local.get('status');
  const list = el('status-list');
  list.replaceChildren();
  for (const [name, provider] of Object.entries(PROVIDERS)) {
    const entry = status?.[name];
    const state = providerState(entry, currentSettings.providers[name], { apiConnected });
    const item = document.createElement('div');
    item.className = 'status-item';
    item.innerHTML = `<div class="status-title"><span class="status-dot ${state.tone}"></span><strong>${provider.name}: ${state.label}</strong></div><p></p>`;
    const copy = item.querySelector('p');
    copy.textContent = entry?.running ? state.detail : `${state.detail} Last run: ${formatTime(entry?.lastRunMs)}.`;
    if (state.tone === 'error') copy.className = 'error-copy';
    list.appendChild(item);
  }
}

el('save').addEventListener('click', async () => {
  await save();
  await checkConnection();
});
el('sync-now').addEventListener('click', async () => {
  const button = el('sync-now');
  button.disabled = true;
  button.textContent = 'Checking…';
  await save({ announce: false });
  const connected = await checkConnection();
  if (connected) {
    button.textContent = 'Syncing…';
    el('saved').textContent = `Sync started using port ${currentSettings.port}`;
    try { await chrome.runtime.sendMessage({ type: 'syncNow' }); } catch { /* worker still starts */ }
  } else {
    el('saved').textContent = 'Sync not started';
  }
  button.disabled = false;
  button.textContent = 'Sync now';
  setTimeout(() => (el('saved').textContent = ''), 2200);
});
el('test-connection').addEventListener('click', async () => {
  await save({ announce: false });
  await checkConnection();
});
el('port').addEventListener('input', updateSetupCommand);
el('token').addEventListener('input', updateSetupCommand);
chrome.storage.onChanged.addListener(changes => { if (changes.status) renderStatus(); });
renderProviderControls();
await load();
updateSetupCommand();
await checkConnection();
