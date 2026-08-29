import { mkdtemp, mkdir, readFile, writeFile, rm } from 'fs/promises';
import { join } from 'path';
import { tmpdir } from 'os';

const root = await mkdtemp(join(tmpdir(), 'hstry-workbuddy-'));
const source = join(root, '.workbuddy', 'projects');
await mkdir(join(source, 'project-a'), { recursive: true });
const fixture = await readFile(new URL('../../testdata/workbuddy/project-a/session-test.jsonl', import.meta.url));
await writeFile(join(source, 'project-a', 'session-test.jsonl'), fixture);

function request(method, params = {}) {
  const result = Bun.spawnSync(['bun', 'run', new URL('./adapter.ts', import.meta.url).pathname], {
    env: { ...process.env, HOME: root, HSTRY_REQUEST: JSON.stringify({ method, params }) },
  });
  if (!result.success) throw new Error(result.stderr.toString());
  return JSON.parse(result.stdout.toString());
}

try {
  const confidence = request('detect', { path: source });
  if (confidence !== 0.9) throw new Error(`unexpected confidence: ${confidence}`);
  const conversations = request('parse', { path: source, opts: {} });
  if (conversations.length !== 1) throw new Error('expected one conversation');
  const [conversation] = conversations;
  if (conversation.title !== 'WorkBuddy deployment') throw new Error('title lost');
  if (conversation.workspace !== '/projects/demo') throw new Error('workspace lost');
  if (conversation.messages.length !== 4) throw new Error('event mapping failed');
  if (conversation.messages[0].content !== 'Deploy the service') throw new Error('user query extraction failed');
  if (conversation.messages[1].parts.at(-1).type !== 'tool_call') throw new Error('tool call lost');
  if (conversation.updatedAt !== 1782900000500) throw new Error('updated timestamp is not event-derived');
  if (!Number.isInteger(conversation.createdAt)) throw new Error('timestamp must be integer ms');
  console.log('workbuddy adapter fixture test passed');
} finally {
  await rm(root, { recursive: true, force: true });
}
