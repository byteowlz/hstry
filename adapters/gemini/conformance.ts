// Format checked against real public examples (2026-10):
//  - AI Studio prompt files (marcelamayr/Gemini-json-Viewer, "Penguin's Icy Love Poem.json"):
//    {runSettings:{model}, systemInstruction, chunkedPrompt:{chunks:[{text,role:'user'|'model',isThought?,tokenCount?}]}}
//  - Google Takeout MyActivity.json for Gemini Apps (minipoisson/Gemini_Json2md4NotebookLM
//    convert_history.py): array of {header:'Gemini Apps', title, time ISO, subtitles:[{name,value}],
//    safeHtmlItem:[{html}]}.
//  - generic {conversations|array of {messages[{role,content}]}} (the hstry test-data shape).
import { mkdir, utimes, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const exportData = [
  {
    id: 'gem-conv-1', title: 'Trip ideas', create_time: 1750000000, update_time: 1750000090,
    messages: [
      { role: 'user', content: 'Suggest a weekend trip', create_time: 1750000001 },
      { role: 'model', content: 'How about Lisbon?', create_time: 1750000005, model: 'gemini-2.5-flash' },
      { role: 'user', parts: [{ text: 'Why?' }], create_time: 1750000060 },
      { role: 'model', parts: [{ text: 'Great food and trams.' }], create_time: 1750000090, model: 'gemini-2.5-flash' },
    ],
  },
  {
    id: 'gem-conv-2', title: 'Haiku', create_time: 1750100000,
    messages: [
      { role: 'user', content: 'Write a haiku', create_time: 1750100001 },
      { role: 'model', content: 'Quiet morning rain', create_time: 1750100002, model: 'gemini-2.5-pro' },
    ],
  },
];

const aiStudio = {
  runSettings: { temperature: 1, model: 'models/gemini-2.5-pro', topP: 0.95, enableCodeExecution: false },
  systemInstruction: { parts: [{ text: 'You are a helpful penguin.' }] },
  chunkedPrompt: {
    chunks: [
      { text: 'What does a penguin think?', role: 'user', tokenCount: 6 },
      { text: 'Considering the ice, the penguin ponders fish.', role: 'model', isThought: true, tokenCount: 9 },
      { text: 'Penguins mostly think about fish and the cold.', role: 'model', tokenCount: 10, finishReason: 'STOP' },
    ],
    pendingInputs: [],
  },
};

const myActivity = [
  {
    header: 'Gemini Apps',
    title: 'Prompted with: Hello',
    time: '2026-05-04T10:00:00.000Z',
    subtitles: [{ name: 'You', value: 'Hello there' }],
    safeHtmlItem: [{ html: '<div>Hi! <b>How</b> can I help?</div>' }],
  },
  {
    header: 'Gemini Apps',
    title: 'Prompted with: Bye',
    time: '2026-05-04T10:05:00.000Z',
    subtitles: [{ name: 'You', value: 'Bye now' }],
    safeHtmlItem: [{ html: 'Goodbye &amp; good luck<br>See you' }],
  },
  { header: 'Search', title: 'Searched for', time: '2026-05-04T10:06:00.000Z' },
];

export const cases: ConformanceCase[] = [
  {
    name: 'generic-conversations-json',
    detection: 'content',
    async setup(home) {
      const dir = join(home, 'Downloads');
      await mkdir(dir, { recursive: true });
      const file = join(dir, 'gemini-conversations.json');
      await writeFile(file, JSON.stringify(exportData));
      return file;
    },
  },
  {
    name: 'ai-studio-prompt',
    detection: 'content',
    async setup(home) {
      const dir = join(home, 'Downloads');
      await mkdir(dir, { recursive: true });
      const file = join(dir, 'ai-studio-penguin.json');
      await writeFile(file, JSON.stringify(aiStudio));
      // AI Studio files carry no timestamps; the adapter uses the file mtime, so
      // pin it to keep the snapshot deterministic.
      const touched = new Date('2026-05-03T09:00:00.000Z');
      await utimes(file, touched, touched);
      return file;
    },
    check(conversations) {
      const [conversation] = conversations as { title?: string; model?: string; messages: { role: string; content: string; parts?: Record<string, any>[] }[] }[];
      if (conversation.model !== 'gemini-2.5-pro') throw new Error(`runSettings.model not read: ${conversation.model}`);
      if (!conversation.messages.some(m => m.role === 'system' && m.content.includes('penguin'))) throw new Error('systemInstruction dropped');
      const parts = conversation.messages.flatMap(m => m.parts ?? []);
      if (!parts.some(p => p.type === 'thinking' && p.text.includes('ponders fish'))) throw new Error('isThought chunk not mapped to reasoning');
      if (!conversation.messages.some(m => m.role === 'assistant' && m.content.includes('fish and the cold'))) throw new Error('model chunk dropped');
    },
  },
  {
    name: 'takeout-myactivity',
    detection: 'content',
    async setup(home) {
      const dir = join(home, 'Takeout', 'My Activity', 'Gemini Apps');
      await mkdir(dir, { recursive: true });
      const file = join(dir, 'MyActivity.json');
      await writeFile(file, JSON.stringify(myActivity));
      return file;
    },
    check(conversations) {
      const convs = conversations as { messages: { role: string; content: string }[] }[];
      if (convs.length !== 2) throw new Error(`expected 2 Gemini activity records, got ${convs.length}`);
      const first = convs.find(c => c.messages.some(m => m.content.includes('Hello there')))!;
      if (!first) throw new Error('prompt subtitles dropped');
      if (!first.messages.some(m => m.role === 'assistant' && m.content.includes('How') && !m.content.includes('<'))) throw new Error('response html not converted to text');
      const second = convs.find(c => c.messages.some(m => m.content.includes('Bye now')))!;
      if (!second.messages.some(m => m.content.includes('Goodbye & good luck'))) throw new Error('html entities not decoded');
    },
  },
];
