// Smoke test that the options module creates dynamic controls before wiring them.
// Usage: bun extension/test/options-init.js

import assert from 'node:assert/strict';

const elements = new Map();

class FakeElement {
  constructor(id = '') {
    this.id = id;
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.dataset = {};
    this.className = '';
    this.textContent = '';
    this.children = [];
    this.queries = new Map();
    this.listeners = new Map();
    if (id) elements.set(id, this);
  }

  set innerHTML(html) {
    const providerId = html.match(/id="(provider-[^"]+)"/)?.[1];
    if (providerId) new FakeElement(providerId);
    for (const selector of ['small', '.provider-domain', 'p', 'strong', 'button']) {
      this.queries.set(selector, new FakeElement());
    }
  }

  querySelector(selector) {
    return this.queries.get(selector) ?? null;
  }

  replaceChildren() {
    this.children = [];
  }

  appendChild(child) {
    this.children.push(child);
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  setAttribute(name, value) {
    this[name] = value;
  }
}

for (const id of [
  'port',
  'token',
  'interval',
  'provider-list',
  'status-list',
  'resync-list',
  'resync-result',
  'connection-result',
  'connection-dot',
  'test-connection',
  'setup-command',
  'saved',
  'save',
  'sync-now',
]) {
  new FakeElement(id);
}

globalThis.document = {
  getElementById: id => elements.get(id) ?? null,
  createElement: () => new FakeElement(),
};
globalThis.chrome = {
  runtime: {
    sendMessage: async message =>
      message.type === 'checkApi' ? { ok: true, url: 'http://127.0.0.1:3000' } : { ok: true },
  },
  storage: {
    local: {
      get: async () => ({ settings: undefined, status: {} }),
      set: async () => {},
    },
    onChanged: { addListener: () => {} },
  },
};

await import(`../options.js?options-init-test=${Date.now()}`);

for (const provider of ['chatgpt', 'claude', 'gemini', 'perplexity']) {
  const checkbox = elements.get(`provider-${provider}`);
  assert.ok(checkbox, `${provider} checkbox was created`);
  assert.ok(checkbox.listeners.has('change'), `${provider} change listener was registered`);
}

console.log('PASS options page initializes dynamic provider controls');
