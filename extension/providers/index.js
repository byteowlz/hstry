import { syncChatGPT } from './chatgpt.js';
import { syncClaude } from './claude.js';
import { syncGemini } from './gemini.js';
import { syncPerplexity } from './perplexity.js';
import { syncCopilot } from './copilot.js';

// Shared provider seam for the worker and both extension surfaces. A provider
// owns its sync implementation; callers only need this small descriptor.
export const PROVIDERS = {
  chatgpt: {
    name: 'ChatGPT',
    site: 'chatgpt.com',
    description: 'Personal and workspace conversations',
    defaultEnabled: true,
    sourceId: 'chatgpt-web',
    adapter: 'chatgpt-web',
    sync: syncChatGPT,
  },
  claude: {
    name: 'Claude',
    site: 'claude.ai',
    description: 'Available organizations',
    defaultEnabled: true,
    sourceId: 'claude-web',
    adapter: 'claude-web',
    sync: syncClaude,
  },
  gemini: {
    name: 'Gemini',
    site: 'gemini.google.com',
    description: 'Conversations from your Google account',
    defaultEnabled: false,
    sourceId: 'gemini-web',
    adapter: 'gemini',
    sync: syncGemini,
  },
  perplexity: {
    name: 'Perplexity',
    site: 'perplexity.ai',
    description: 'Threads from your Perplexity library',
    defaultEnabled: false,
    sourceId: 'perplexity-web',
    adapter: 'perplexity',
    sync: syncPerplexity,
  },
  copilot: {
    name: 'M365 Copilot Chat',
    site: 'm365.cloud.microsoft',
    description: 'Work Copilot conversations from your signed-in session',
    defaultEnabled: false,
    sourceId: 'm365-copilot',
    adapter: 'm365-copilot',
    sync: syncCopilot,
  },
};

export const DEFAULT_PROVIDER_SETTINGS = Object.fromEntries(
  Object.entries(PROVIDERS).map(([id, provider]) => [id, provider.defaultEnabled])
);
