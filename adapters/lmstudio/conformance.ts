// Format checked against LM Studio 0.3.x on-disk chats: ~/.lmstudio/conversations/
// <epoch-ms>.conversation.json (docs: lmstudio.ai/docs/app/basics/chat; the old
// ~/.cache/lm-studio/conversations path is legacy). Shape: {name, pinned,
// createdAt ms, systemPrompt, tokenCount, messages[{versions[{type:
// singleStep|multiStep, role, content[] | steps[{type:'contentBlock',
// style?:{type:'thinking'}, content[], genInfo}], senderInfo}],
// currentlySelected}]}. The `name` title and thinking-style blocks are from
// LM Studio's own files; tool-call steps could not be verified and are not covered.
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const user = (text: string) => ({
  versions: [{ type: 'singleStep', role: 'user', content: [{ type: 'text', text }] }],
  currentlySelected: 0,
});
const assistant = (model: string, steps: unknown[]) => ({
  versions: [{ type: 'multiStep', role: 'assistant', senderInfo: { senderName: model }, steps }],
  currentlySelected: 0,
});
const block = (text: string, style?: unknown) => ({
  type: 'contentBlock', stepIdentifier: `step-${text.length}`, content: [{ type: 'text', text, fromDraftModel: false }],
  defaultShouldIncludeInContext: true, shouldAllowEditing: true, ...(style ? { style } : {}),
});

const chat1 = {
  name: 'Capital of France', pinned: false, createdAt: 1750000000000, preset: '', tokenCount: 42,
  userLastMessagedAt: 1750000005000, systemPrompt: 'Be brief.',
  messages: [
    user('What is the capital of France?'),
    assistant('qwen3-4b', [
      block('The user asks a simple question.', { type: 'thinking', ended: true, title: 'Thought for 1 second' }),
      block('Paris.'),
    ]),
    {
      // regenerated answer: currentlySelected points at the second version
      versions: [
        { type: 'multiStep', role: 'assistant', senderInfo: { senderName: 'qwen3-4b' }, steps: [block('Discarded draft.')] },
        { type: 'multiStep', role: 'assistant', senderInfo: { senderName: 'qwen3-4b' }, steps: [block('Paris, on the Seine.')] },
      ],
      currentlySelected: 1,
    },
  ],
  usePerChatPredictionConfig: true, perChatPredictionConfig: { fields: [] }, clientInput: '', clientInputFiles: [],
};
const chat2 = {
  name: 'Hello', pinned: true, createdAt: 1750100000000, systemPrompt: '',
  messages: [user('Hi'), assistant('llama-3.2-1b', [block('Hello!')])],
};

export const cases: ConformanceCase[] = [{
  name: 'conversation-files',
  async setup(home) {
    const dir = join(home, '.lmstudio', 'conversations');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, '1750000000000.conversation.json'), JSON.stringify(chat1));
    await writeFile(join(dir, '1750100000000.conversation.json'), JSON.stringify(chat2));
    return dir;
  },
}];
