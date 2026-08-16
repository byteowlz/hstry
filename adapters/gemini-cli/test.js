import { mkdtemp, mkdir, readFile, writeFile, rm } from 'fs/promises';
import { join } from 'path';
import { tmpdir } from 'os';

const root = await mkdtemp(join(tmpdir(), 'hstry-gemini-cli-'));
const source = join(root, '.gemini', 'tmp');
await mkdir(join(source, 'chats'), { recursive: true });
const fixture = await readFile(new URL('../../testdata/gemini-cli/chats/session-test.jsonl', import.meta.url));
await writeFile(join(source, 'chats', 'session-test.jsonl'), fixture);

function request(method, params = {}) {
  const result = Bun.spawnSync(['bun', 'run', new URL('./adapter.ts', import.meta.url).pathname], {
    env: { ...process.env, HOME: root, HSTRY_REQUEST: JSON.stringify({ method, params }) },
  });
  if (!result.success) throw new Error(result.stderr.toString());
  return JSON.parse(result.stdout.toString());
}

try {
  const info = request('info');
  if (info.name !== 'gemini-cli') throw new Error(`unexpected adapter name: ${info.name}`);
  const confidence = request('detect', { path: source });
  if (confidence !== 0.85) throw new Error(`unexpected confidence: ${confidence}`);
  const conversations = request('parse', { path: source, opts: {} });
  if (conversations.length !== 1) throw new Error(`expected one conversation`);
  const [conversation] = conversations;
  if (conversation.externalId !== 'gemini-cli-session-1') throw new Error('wrong session ID');
  if (conversation.messages.length !== 3) throw new Error('bootstrap/duplicate filtering failed');
  if (conversation.messages[1].parts[0].type !== 'thinking') throw new Error('thinking lost');
  if (!Number.isInteger(conversation.createdAt)) throw new Error('timestamp must be integer ms');
  console.log('gemini-cli adapter fixture test passed');
} finally {
  await rm(root, { recursive: true, force: true });
}
