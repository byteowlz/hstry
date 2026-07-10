import { DEFAULT_PROVIDER_SETTINGS, friendlyError, formatTime, normalizeSettings, progressDetail, providerState } from '../ui.js';

let failures = 0;
function check(label, actual, expected) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  console.log(`${ok ? 'PASS' : 'FAIL'} ${label}`);
  if (!ok) { console.log(`  got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`); failures++; }
}

const now = Date.parse('2026-07-10T12:00:00Z');
check('new providers are opt-in', DEFAULT_PROVIDER_SETTINGS, {
  chatgpt: true,
  claude: true,
  gemini: false,
  perplexity: false,
});
check('never synced time', formatTime(null, now), 'Never');
check('relative sync time', formatTime(now - 17 * 60_000, now), '17m ago');
check('fresh provider is informative', providerState(undefined, true), { tone: 'idle', label: 'Ready to sync', detail: 'No sync has run yet.' });
check('disabled provider is paused', providerState(undefined, false).label, 'Paused');
check('offline error is actionable', friendlyError('cannot reach hstry-api at x'), 'The local hstry API is offline. Start it, then try again.');
check('login error is actionable', friendlyError('claude.ai: not logged in'), 'Open this provider and sign in, then sync again.');
check('recovered API makes stale offline failure retryable', providerState({
  lastRunMs: now - 27 * 60_000,
  lastError: 'cannot reach hstry-api at http://127.0.0.1:3000/ingest',
}, true, { apiConnected: true }), {
  tone: 'idle',
  label: 'Ready to retry',
  detail: 'The API is connected now. Run sync again.',
});
check('running provider exposes live phase', providerState({
  running: true,
  progress: { phase: 'importing', detected: 12, processed: 7, created: 3, updated: 2 },
}, true), {
  tone: 'syncing',
  label: 'Syncing',
  detail: '12 detected · 7 processed · 3 new · 2 updated',
});
check('progress detail omits empty counters', progressDetail({ detected: 4, processed: 1 }), '4 detected · 1 processed');
check('visible settings normalize before sync', normalizeSettings({
  port: '3434',
  token: ' hello ',
  intervalMinutes: '15',
  providers: { chatgpt: true, claude: true },
}, { port: 3000, intervalMinutes: 15 }), {
  port: 3434,
  token: 'hello',
  intervalMinutes: 15,
  providers: { chatgpt: true, claude: true, gemini: false, perplexity: false },
});
check('invalid port cannot escape form constraints', normalizeSettings({
  port: '70000',
  token: '',
  intervalMinutes: '0',
  providers: {},
}, { port: 3000, intervalMinutes: 15 }), {
  port: 3000,
  token: '',
  intervalMinutes: 15,
  providers: { chatgpt: false, claude: false, gemini: false, perplexity: false },
});

process.exit(failures ? 1 : 0);
