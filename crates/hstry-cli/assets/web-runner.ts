import { chromium, firefox, webkit, request, type BrowserType, type APIRequestContext } from 'playwright';
import { promises as fs } from 'fs';
import { dirname, join } from 'path';

type Provider = 'chatgpt' | 'claude' | 'gemini';

type Command = 'login' | 'sync';

type Workspace = { id: string; name?: string | null };

const providerUrls: Record<Provider, string> = {
  chatgpt: 'https://chatgpt.com',
  claude: 'https://claude.ai',
  gemini: 'https://gemini.google.com',
};

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const command = args.command as Command;
  const provider = args.provider as Provider;

  if (!command || !provider) {
    throw new Error('Missing command or provider');
  }

  const browserType = resolveBrowser(args.browser ?? 'chromium');
  const headless = args.headful ? false : true;
  const storageState = args.storageState;
  const outputPath = args.output;

  if (command === 'login') {
    if (!storageState) {
      throw new Error('storage state path required');
    }
    await login(provider, browserType, headless, storageState);
    return;
  }

  if (command === 'sync') {
    if (!storageState || !outputPath) {
      throw new Error('storage state and output path required');
    }
    await sync(provider, browserType, headless, storageState, outputPath);
    return;
  }
}

async function login(
  provider: Provider,
  browserType: BrowserType,
  headless: boolean,
  storageStatePath: string
): Promise<void> {
  const browser = await browserType.launch({ headless });
  const context = await browser.newContext();
  const page = await context.newPage();

  await page.goto(providerUrls[provider], { waitUntil: 'domcontentloaded' });

  if (provider === 'chatgpt') {
    await page.waitForSelector('textarea', { timeout: 0 });
  } else if (provider === 'claude') {
    await page.waitForSelector('textarea, [data-testid="composer"]', { timeout: 0 });
  } else {
    await page.waitForSelector('textarea', { timeout: 0 });
  }

  await ensureDir(storageStatePath);
  await context.storageState({ path: storageStatePath });
  await browser.close();
}

async function sync(
  provider: Provider,
  browserType: BrowserType,
  headless: boolean,
  storageStatePath: string,
  outputPath: string
): Promise<void> {
  if (provider === 'chatgpt') {
    const api = await request.newContext({
      storageState: storageStatePath,
    });
    const conversations = await fetchChatGPTConversations(api, providerUrls.chatgpt);
    await ensureDir(outputPath);
    await fs.writeFile(outputPath, JSON.stringify(conversations, null, 2), 'utf8');
    await api.dispose();
  } else {
    throw new Error(`${provider} sync not implemented yet`);
  }
}

async function fetchChatGPTConversations(api: APIRequestContext, baseUrl: string): Promise<any[]> {
  const workspaces = await fetchChatGPTWorkspaces(api, baseUrl);
  const entries: any[] = [];

  if (workspaces.length === 0) {
    const items = await fetchChatGPTConversationList(api, baseUrl, undefined);
    entries.push(...items);
  } else {
    for (const workspace of workspaces) {
      const items = await fetchChatGPTConversationList(api, baseUrl, workspace.id);
      entries.push(
        ...items.map(item => ({
          ...item,
          workspace_id: workspace.id,
          workspace_title: workspace.name ?? undefined,
        }))
      );
    }
  }

  const conversations: any[] = [];
  for (const entry of entries) {
    if (!entry.id) continue;
    const detail = await fetchChatGPTConversationDetail(api, baseUrl, entry.id);
    if (detail) {
      if (entry.workspace_id) {
        detail.workspace_id = entry.workspace_id;
      }
      conversations.push(detail);
    }
  }

  return conversations;
}

async function fetchChatGPTWorkspaces(api: APIRequestContext, baseUrl: string): Promise<Workspace[]> {
  try {
    const response = await api.get(`${baseUrl}/backend-api/workspaces`);
    if (!response.ok()) return [];
    const data = await response.json();
    if (!Array.isArray(data?.items)) return [];
    return data.items.map((item: any) => ({ id: item.id, name: item.name ?? null }));
  } catch {
    return [];
  }
}

async function fetchChatGPTConversationList(
  api: APIRequestContext,
  baseUrl: string,
  workspaceId?: string
): Promise<any[]> {
  const limit = 50;
  let offset = 0;
  const results: any[] = [];

  while (true) {
    const url = new URL(`${baseUrl}/backend-api/conversations`);
    url.searchParams.set('offset', offset.toString());
    url.searchParams.set('limit', limit.toString());
    if (workspaceId) {
      url.searchParams.set('workspace_id', workspaceId);
    }

    const response = await api.get(url.toString());
    if (!response.ok()) break;
    const data = await response.json();
    if (!Array.isArray(data?.items) || data.items.length === 0) break;
    results.push(...data.items);
    offset += data.items.length;
    if (data.items.length < limit) break;
  }

  return results;
}

async function fetchChatGPTConversationDetail(
  api: APIRequestContext,
  baseUrl: string,
  id: string
): Promise<any | null> {
  const response = await api.get(`${baseUrl}/backend-api/conversation/${id}`);
  if (!response.ok()) return null;
  return response.json();
}

function resolveBrowser(name: string): BrowserType {
  switch (name) {
    case 'firefox':
      return firefox;
    case 'webkit':
      return webkit;
    default:
      return chromium;
  }
}

function parseArgs(args: string[]): Record<string, string | boolean> {
  const result: Record<string, string | boolean> = {};
  if (args.length > 0) {
    result.command = args[0];
  }

  for (let i = 1; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === '--headful') {
      result.headful = true;
      continue;
    }
    if (arg.startsWith('--')) {
      const rawKey = arg.replace(/^--/, '');
      const value = args[i + 1];
      const key = toCamelCase(rawKey);
      result[key] = value;
      i += 1;
    }
  }
  return result;
}

function toCamelCase(input: string): string {
  return input.replace(/-([a-z])/g, (_, char: string) => char.toUpperCase());
}

async function ensureDir(filePath: string): Promise<void> {
  await fs.mkdir(dirname(filePath), { recursive: true });
}

main().catch(err => {
  console.error(String(err));
  process.exit(1);
});
