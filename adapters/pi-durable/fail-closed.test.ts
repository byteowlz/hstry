import {test, expect} from 'bun:test';
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {Harness, MemoryStorage, createRegistry} from '@earendil-works/pi-durable';
import {createModels} from '@earendil-works/pi-ai/models';
import {fauxProvider, fauxAssistantMessage} from '@earendil-works/pi-ai/providers/faux';
import {BACKGROUND_CONTEXT as context} from '@earendil-works/chord/context';
import {restoreSnapshot, captureSnapshot} from './interop.ts';
import {validateSnapshot} from './snapshot.ts';
import {validateNativePiSession} from '../pi/native.ts';

test('corrupt native associations and foreign context heads are rejected before any target conversation is written', async () => {
  const fixture = JSON.parse(await readFile(new URL('./fixtures/full.pi-durable.json', import.meta.url), 'utf8'));
  for (const scenario of ['foreign-head', 'missing-tool-id', 'unknown-content']) {
    const corrupted = structuredClone(fixture) as {sourceVersion: string; conversations: {record: {id: number; parent?: unknown}; entries: Record<string, unknown>[]}[]};
    if (scenario === 'foreign-head') {
      const independent = corrupted.conversations.find(conversation => conversation.record.id !== 1 && !conversation.record.parent)!;
      independent.entries[0].head = corrupted.conversations[0].entries[0].id;
    } else {
      const entry = corrupted.conversations[0].entries.find(entry => Array.isArray(entry.model) && (entry.model[0] as {role?: string}).role === 'toolResult')!;
      const model = (entry.model as Record<string, unknown>[])[0];
      if (scenario === 'missing-tool-id') delete model.toolCallId;
      else model.content = [{type: 'future-unknown-schema', opaque: true}];
    }
    corrupted.sourceVersion = `sha256:${createHash('sha256').update(JSON.stringify(corrupted.conversations)).digest('hex')}`;
    const storage = new MemoryStorage();
    const harness = await Harness.open(storage, {models: createModels(), registry: createRegistry()}, context);
    try {
      await expect(restoreSnapshot(corrupted as never, harness)).rejects.toThrow();
      expect((await captureSnapshot({harness, storage}, 'empty-target')).conversations).toEqual([]);
    } finally {await harness.close(context);}
  }
  const oversized = {...fixture, opaque: 'x'.repeat(16 * 1024 * 1024)};
  expect(() => validateSnapshot(oversized)).toThrow('16 MiB');
  expect(() => validateNativePiSession({header: {type: 'session', version: 3, id: 'synthetic', cwd: '.', timestamp: '2026-01-01T00:00:00Z'}, entries: [], opaque: oversized.opaque} as never)).toThrow('16 MiB');
});

test('committed but pending native work cannot be exported as settled history', async () => {
  const models = createModels();
  const faux = fauxProvider(); models.setProvider(faux.provider);
  let settle!: () => void;
  const gate = new Promise<void>(resolve => {settle = resolve;});
  faux.setResponses([async () => {await gate; return fauxAssistantMessage('settled fixture');}]);
  const storage = new MemoryStorage();
  const harness = await Harness.open(storage, {models, registry: createRegistry()}, context);
  try {
    const root = await harness.root(context, {agent: {model: {provider: 'faux', modelId: 'faux-1'}}});
    const submission = await root.submit({type: 'input', content: 'synthetic pending turn'}, context);
    await expect(captureSnapshot({harness, storage}, 'pending-source')).rejects.toThrow('pending or uncertain');
    settle();
    expect((await submission.wait(context)).status).toBe('done');
    expect((await captureSnapshot({harness, storage}, 'settled-source')).conversations.length).toBe(1);
  } finally {settle(); await harness.close(context);}
});
