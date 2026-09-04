import { tmpdir } from 'os';
import { join } from 'path';

const request = {
  method: 'export',
  params: {
    conversations: [{
      externalId: 'codex-session',
      title: 'Converted Codex session',
      createdAt: 1_700_000_000_000,
      workspace: join(tmpdir(), 'hstry-pi-workspace'),
      messages: [
        {
          role: 'assistant',
          content: 'No source usage metadata',
          createdAt: 1_700_000_001_000,
          model: 'gpt-test',
        },
        {
          role: 'assistant',
          content: 'Tokens but no source cost metadata',
          createdAt: 1_700_000_002_000,
          model: 'gpt-test',
          tokens: 42,
        },
      ],
    }],
    opts: { format: 'pi', includeTools: true },
  },
};

const result = Bun.spawnSync(
  ['bun', 'run', new URL('./adapter.ts', import.meta.url).pathname],
  { env: { ...process.env, HSTRY_REQUEST: JSON.stringify(request) } },
);

if (!result.success) throw new Error(result.stderr.toString());
const exported = JSON.parse(result.stdout.toString());
const entries = exported.files[0].content
  .split(/\r?\n/)
  .filter(Boolean)
  .map(line => JSON.parse(line));
const assistants = entries
  .filter(entry => entry.type === 'message' && entry.message?.role === 'assistant')
  .map(entry => entry.message);

if (assistants.length !== 2) throw new Error(`expected 2 assistant messages, got ${assistants.length}`);
for (const message of assistants) {
  if (typeof message.usage?.input !== 'number') throw new Error('assistant usage.input missing');
  if (typeof message.usage?.cost?.total !== 'number') throw new Error('assistant usage.cost.total missing');
}
if (assistants[0].usage.totalTokens !== 0) throw new Error('missing tokens should export as zero');
if (assistants[1].usage.totalTokens !== 42) throw new Error('source token count was not preserved');
console.log('pi adapter fixture test passed');
