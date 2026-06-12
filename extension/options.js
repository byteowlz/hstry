const DEFAULTS = {
  port: 3000,
  token: '',
  intervalMinutes: 15,
  providers: { chatgpt: true, claude: true },
};

const el = id => document.getElementById(id);

async function load() {
  const { settings } = await chrome.storage.local.get('settings');
  const merged = {
    ...DEFAULTS,
    ...settings,
    providers: { ...DEFAULTS.providers, ...settings?.providers },
  };
  el('port').value = merged.port;
  el('token').value = merged.token;
  el('interval').value = merged.intervalMinutes;
  el('provider-chatgpt').checked = merged.providers.chatgpt;
  el('provider-claude').checked = merged.providers.claude;
  await renderStatus();
}

async function save() {
  const settings = {
    port: Number(el('port').value) || DEFAULTS.port,
    token: el('token').value.trim(),
    intervalMinutes: Number(el('interval').value) || DEFAULTS.intervalMinutes,
    providers: {
      chatgpt: el('provider-chatgpt').checked,
      claude: el('provider-claude').checked,
    },
  };
  await chrome.storage.local.set({ settings });
  await chrome.runtime.sendMessage({ type: 'settingsChanged' });
  el('saved').textContent = 'Saved';
  setTimeout(() => (el('saved').textContent = ''), 1500);
}

function formatTime(ms) {
  return ms ? new Date(ms).toLocaleString() : '—';
}

async function renderStatus() {
  const { status } = await chrome.storage.local.get('status');
  const tbody = document.querySelector('#status-table tbody');
  tbody.replaceChildren();
  for (const [name, entry] of Object.entries(status ?? {})) {
    const row = document.createElement('tr');
    const cells = [
      name,
      formatTime(entry.lastRunMs),
      formatTime(entry.lastSuccessMs),
      entry.lastCount ?? '—',
    ];
    for (const value of cells) {
      const td = document.createElement('td');
      td.textContent = String(value);
      row.appendChild(td);
    }
    const statusTd = document.createElement('td');
    statusTd.textContent = entry.lastError ?? 'ok';
    statusTd.className = entry.lastError ? 'error' : 'ok';
    row.appendChild(statusTd);
    tbody.appendChild(row);
  }
}

el('save').addEventListener('click', save);
el('sync-now').addEventListener('click', async () => {
  el('saved').textContent = 'Syncing… (updates below as it runs)';
  // The worker acks immediately and syncs in the background; the status table
  // refreshes via the storage.onChanged listener as each provider finishes.
  try {
    await chrome.runtime.sendMessage({ type: 'syncNow' });
  } catch {
    // Worker was asleep and is spinning up; the sync still starts.
  }
  setTimeout(() => (el('saved').textContent = ''), 2000);
});

chrome.storage.onChanged.addListener(changes => {
  if (changes.status) renderStatus();
});

load();
