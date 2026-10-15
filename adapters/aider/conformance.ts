// Format checked against Aider's native .aider.chat.history.md (aider v0.86.x,
// aider/io.py: "# aider chat started at", "#### " user lines, "> " tool output).
import { mkdir, utimes, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { ConformanceCase } from '../conformance.ts';

const history = `# aider chat started at 2025-01-01 12:00:00

> /usr/bin/aider --model sonnet
> Aider v0.86.1
> Main model: anthropic/claude-sonnet-4 with diff edit format
> Git repo: .git with 3 files

#### add a hello function to hello.py

Sure, here is the change.

hello.py
\`\`\`python
<<<<<<< SEARCH
=======
def hello():
    return "hi"
>>>>>>> REPLACE
\`\`\`

> Tokens: 1.2k sent, 80 received. Cost: $0.01 message, $0.01 session.
> Applied edit to hello.py

#### now add a test
#### and run it

I added the test.

> Tokens: 1.5k sent, 40 received.
`;

export const cases: ConformanceCase[] = [{
  name: 'native-history',
  async setup(home) {
    const dir = join(home, 'projects', 'demo');
    await mkdir(dir, { recursive: true });
    const file = join(dir, '.aider.chat.history.md');
    await writeFile(file, history);
    const t = new Date('2025-01-01T12:05:00Z');
    await utimes(file, t, t);
    return join(home, 'projects');
  },
}];
