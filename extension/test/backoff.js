import { fetchJson, RateLimitedError } from '../lib/common.js';

let calls = 0;
globalThis.fetch = async () => {
  calls++;
  if (calls <= 2) return new Response('rl', { status: 429, headers: { 'retry-after': '0' } });
  return new Response(JSON.stringify({ ok: true, calls }), { status: 200, headers: { 'Content-Type': 'application/json' } });
};
const data = await fetchJson('https://example.com/x', {}, { retries: 5, baseDelayMs: 20 });
console.log(data.ok && data.calls === 3 ? 'PASS recovered after 2x 429' : `FAIL ${JSON.stringify(data)}`);

calls = 0;
globalThis.fetch = async () => new Response('nope', { status: 429, headers: { 'retry-after': '0' } });
try {
  await fetchJson('https://example.com/y', {}, { retries: 2, baseDelayMs: 5 });
  console.log('FAIL should have thrown');
} catch (e) {
  console.log(e instanceof RateLimitedError ? 'PASS gives up as RateLimitedError' : `FAIL wrong error: ${e.name}`);
}
