import { DEFAULT_PROVIDER_SETTINGS, PROVIDERS } from './providers/index.js';

export { DEFAULT_PROVIDER_SETTINGS, PROVIDERS };

export function normalizeSettings(values, defaults) {
  const port = Number(values.port);
  const intervalMinutes = Number(values.intervalMinutes);
  return {
    port: Number.isInteger(port) && port >= 1 && port <= 65535 ? port : defaults.port,
    token: String(values.token ?? '').trim(),
    intervalMinutes:
      Number.isInteger(intervalMinutes) && intervalMinutes >= 1 && intervalMinutes <= 720
        ? intervalMinutes
        : defaults.intervalMinutes,
    providers: Object.fromEntries(
      Object.keys(PROVIDERS).map(id => [id, Boolean(values.providers?.[id])])
    ),
  };
}

export function formatTime(ms, now = Date.now()) {
  if (!ms) return 'Never';
  const elapsed = Math.max(0, now - ms);
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

export function providerState(entry, enabled = true, { apiConnected = false } = {}) {
  if (!enabled) return { tone: 'muted', label: 'Paused', detail: 'Enable in settings to sync.' };
  if (entry?.running) {
    const progress = entry.progress ?? {};
    if (progress.phase === 'discovering') {
      return { tone: 'syncing', label: 'Discovering', detail: 'Checking for changed conversations…' };
    }
    return {
      tone: 'syncing',
      label: 'Syncing',
      detail: progressDetail(progress),
    };
  }
  if (!entry?.lastRunMs) return { tone: 'idle', label: 'Ready to sync', detail: 'No sync has run yet.' };
  if (
    apiConnected &&
    entry.lastError &&
    (entry.lastError.includes('cannot reach hstry-api') || entry.lastError.includes('Failed to fetch'))
  ) {
    return {
      tone: 'idle',
      label: 'Ready to retry',
      detail: 'The API is connected now. Run sync again.',
    };
  }
  if (entry.lastError) return { tone: 'error', label: 'Needs attention', detail: friendlyError(entry.lastError) };
  if (entry.progress?.phase === 'complete') {
    const progress = entry.progress;
    const changed = progress.created + progress.updated;
    return {
      tone: 'success',
      label: 'Up to date',
      detail: changed > 0
        ? `${progress.created} new · ${progress.updated} updated · ${progress.detected} detected`
        : progress.accepted > 0
          ? `${progress.accepted} accepted · ${progress.detected} detected`
          : `No changes · ${progress.detected} detected`,
    };
  }
  return {
    tone: 'success',
    label: 'Up to date',
    detail: `${entry.lastCount ?? 0} conversation${entry.lastCount === 1 ? '' : 's'} synced last run.`,
  };
}

export function progressDetail(progress) {
  const detected = progress.detected ?? 0;
  const processed = progress.processed ?? 0;
  const parts = [`${detected} detected`, `${processed} processed`];
  if ((progress.created ?? 0) > 0) parts.push(`${progress.created} new`);
  if ((progress.updated ?? 0) > 0) parts.push(`${progress.updated} updated`);
  return parts.join(' · ');
}

export function friendlyError(error) {
  const text = String(error ?? 'Unknown sync error');
  if (text.includes('not logged in')) return 'Open this provider and sign in, then sync again.';
  if (text.includes('cannot reach hstry-api') || text.includes('Failed to fetch')) {
    return 'The local hstry API is offline. Start it, then try again.';
  }
  if (text.includes('rejected the token') || text.includes('401')) {
    return 'The ingest token does not match the local API.';
  }
  if (text.includes('429') || text.includes('RateLimited')) {
    return 'The provider is rate-limiting requests. Wait a few minutes and retry.';
  }
  return text;
}
