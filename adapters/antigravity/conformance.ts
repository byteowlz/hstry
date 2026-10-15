// Format checked against Antigravity CLI (agy) 1.1.14 as documented by
// jazzyalex.github.io/agent-sessions/guides/antigravity-cli-local-history.html (2026-10):
// ~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/transcript.jsonl, one step per line
// {step_index,source:USER_EXPLICIT|MODEL|SYSTEM,type,status:DONE|RUNNING,created_at,content,tool_calls[{name,args}],
// thinking,truncated_fields}. USER_INPUT content is wrapped in <USER_REQUEST>; PLANNER_RESPONSE carries
// content/thinking/tool_calls; RUN_COMMAND/VIEW_FILE/... are the result records (output in content, no call id).
// No cwd or model is recorded. created_at is assumed ISO-8601 (the guide does not show a sample value).
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const jsonl = (records: unknown[]) => records.map(r => JSON.stringify(r)).join('\n') + '\n';

export const cases: ConformanceCase[] = [{
  name: 'cli-transcript-with-tools',
  async setup(home) {
    const root = join(home, '.gemini', 'antigravity-cli', 'brain');
    const logs = join(root, '4c5d6e7f-0000-4000-8000-0000000000f1', '.system_generated', 'logs');
    await mkdir(logs, { recursive: true });
    await writeFile(join(logs, 'transcript.jsonl'), jsonl([
      { step_index: 0, source: 'SYSTEM', type: 'CONVERSATION_HISTORY', status: 'DONE', created_at: '2026-07-01T10:00:00.000Z', content: 'previous conversations: none' },
      { step_index: 1, source: 'USER_EXPLICIT', type: 'USER_INPUT', status: 'DONE', created_at: '2026-07-01T10:00:01.000Z',
        content: '<USER_REQUEST>\nList the files here\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is 2026-07-01T12:00:01+02:00.\n</ADDITIONAL_METADATA>' },
      { step_index: 2, source: 'MODEL', type: 'PLANNER_RESPONSE', status: 'DONE', created_at: '2026-07-01T10:00:03.000Z',
        content: 'I will list the directory.', thinking: 'Use ls via run_command.',
        tool_calls: [{ name: 'run_command', args: { CommandLine: 'ls', Cwd: '/work/proj' } }] },
      { step_index: 3, source: 'MODEL', type: 'RUN_COMMAND', status: 'DONE', created_at: '2026-07-01T10:00:05.000Z', content: 'Cargo.toml\nsrc\n' },
      { step_index: 4, source: 'MODEL', type: 'PLANNER_RESPONSE', status: 'DONE', created_at: '2026-07-01T10:00:07.000Z', content: 'There are Cargo.toml and src.' },
      { step_index: 5, source: 'SYSTEM', type: 'CHECKPOINT', status: 'DONE', created_at: '2026-07-01T10:00:08.000Z', content: 'checkpoint' },
    ]));
    return root;
  },
}];
