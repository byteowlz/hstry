import {SessionManager} from '@earendil-works/pi-coding-agent';
import {fauxAssistantMessage} from '@earendil-works/pi-ai/providers/faux';
import {validateNativePiSession} from '../pi/native.ts';

export const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jG1cAAAAASUVORK5CYII=';
export const FIXTURE_TIME = 1700000000000;
/** Created through the real SDK, normalized as a detached artifact, then parsed
 * again by the same pinned SDK. No native tasks/checkpoints are fabricated. */
export function goldenPiSource() {
  const source = SessionManager.inMemory('/synthetic/hstry', {id: '00000000-0000-4000-8000-000000000001'});
  const root = source.appendMessage({role: 'user', content: [{type: 'text', text: 'golden-root-archive'}, {type: 'image', mimeType: 'image/png', data: PNG}], timestamp: FIXTURE_TIME});
  for (const [index, id] of ['golden-call-a', 'golden-call-b'].entries()) {
    source.appendMessage(fauxAssistantMessage([{type: 'thinking', thinking: 'synthetic reasoning', thinkingSignature: 'synthetic-unverified-signature'}, {type: 'text', text: 'synthetic signed text', textSignature: 'synthetic-text-signature'}, {type: 'toolCall', id, name: 'historical_guard', arguments: {fixture: index}}], {stopReason: 'toolUse', timestamp: FIXTURE_TIME + index + 1}));
    source.appendMessage({role: 'toolResult', toolCallId: id, toolName: 'historical_guard', content: [{type: 'text', text: 'identical golden result'}], isError: index === 1, timestamp: FIXTURE_TIME + index + 3});
  }
  const answer = source.appendMessage(fauxAssistantMessage('golden answer', {timestamp: FIXTURE_TIME + 6}));
  source.appendModelChange('faux', 'faux-alt');
  source.appendModelChange('faux', 'faux-1');
  source.appendSessionInfo('Golden interoperability session');
  source.appendLabelChange(answer, 'verified-answer');
  source.appendContextEdit(root, {content: 'golden root edited for model context'});
  const compaction = source.appendCompaction('golden-compaction-summary', answer, 100, {opaqueFixture: true});
  source.branch(root);
  source.branchWithSummary(root, 'golden branch handoff', {opaqueBranch: true});
  source.appendMessage({role: 'user', content: 'golden fork continuation', timestamp: FIXTURE_TIME + 7});
  source.resetLeaf();
  source.appendMessage({role: 'user', content: 'golden reset continuation', timestamp: FIXTURE_TIME + 8});
  source.appendCustomMessageEntry('fixture-reference', [{type: 'text', text: 'fixture attachment reference'}], true, {uri: 'fixture://attachments/example.txt'});
  const entries = source.getEntries();
  const ids = Object.fromEntries(entries.map((entry, index) => [entry.id, (0x10000000 + index).toString(16)]));
  const normalized = entries.map((entry, index) => {
    const result = {...entry, id: ids[entry.id], parentId: entry.parentId === null ? null : ids[entry.parentId], timestamp: new Date(FIXTURE_TIME + index).toISOString()} as Record<string, unknown>;
    for (const reference of ['firstKeptEntryId', 'fromId', 'targetId']) if (typeof result[reference] === 'string') result[reference] = ids[result[reference] as string];
    return result;
  });
  const header = {...source.getHeader()!, timestamp: new Date(FIXTURE_TIME).toISOString()};
  const detached = validateNativePiSession({header, entries: normalized});
  const parsed = SessionManager.inMemory(header.cwd, undefined, JSON.parse(JSON.stringify([header, ...detached.entries])));
  return {session: parsed, artifact: detached};
}
