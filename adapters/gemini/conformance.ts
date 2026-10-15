// CAVEAT: Google ships no official "conversations" export with this shape.
// The adapter parses a generic JSON array of {id,title,create_time,messages[
// {role: user|model,content,create_time,model}]} (the hstry test-data shape).
// The real Google formats (AI Studio chunkedPrompt files, Takeout
// MyActivity.json) are NOT supported yet; see trx-5ay4.
// This case only locks in the currently-correct handling of the generic shape.
import { mkdir, writeFile } from 'node:fs/promises';
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

export const cases: ConformanceCase[] = [{
  name: 'generic-conversations-json',
  detection: 'content',
  async setup(home) {
    const dir = join(home, 'Downloads');
    await mkdir(dir, { recursive: true });
    const file = join(dir, 'gemini-conversations.json');
    await writeFile(file, JSON.stringify(exportData));
    return file;
  },
}];
