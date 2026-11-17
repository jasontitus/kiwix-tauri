// SPDX-License-Identifier: GPL-3.0-or-later
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
const source = await readFile(new URL('../src-tauri/src/sharing_reader.js', import.meta.url), 'utf8');
function harness() {
  class Element {
    constructor() { this.value = ''; this.children = []; this.listeners = new Map(); this.attributes = new Map(); this.hidden = false; this.textContent = ''; }
    addEventListener(name, callback) { this.listeners.set(name, callback); }
    fire(name) { this.listeners.get(name)?.({ preventDefault() {} }); }
    append(child) { this.children.push(child); }
    replaceChildren() { this.children = []; }
    setAttribute(name, value) { this.attributes.set(name, value); }
    removeAttribute(name) { this.attributes.delete(name); }
  }
  const elements = Object.fromEntries(['search', 'q', 'book', 'mode', 'suggestions', 'results', 'status', 'more'].map(id => [id, new Element()]));
  elements.mode.value = 'combined';
  const pending = [];
  const timers = new Map();
  let nextTimer = 0;
  const context = vm.createContext({
    URL, URLSearchParams, AbortController,
    document: { currentScript: { src: 'http://127.0.0.1:8080/secret/reader.js' }, getElementById: id => elements[id], createElement: () => new Element() },
    window: { addEventListener() {} },
    setTimeout: callback => { timers.set(++nextTimer, callback); return nextTimer; },
    clearTimeout: timer => timers.delete(timer),
    fetch: (url, options) => new Promise(resolve => pending.push({ url, options, resolve: value => resolve({ ok: true, json: async () => value }) })),
  });
  vm.runInContext(source, context);
  return {
    elements, pending,
    type(value) { elements.q.value = value; elements.q.fire('input'); for (const [id, callback] of timers) { timers.delete(id); callback(); } },
    submit(value) { elements.q.value = value; elements.search.fire('submit'); },
  };
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const hit = title => ({ title, path: 'A/Article', href: '/secret/open/book?path=%2FA%2FArticle', snippet: '<em>plain text</em>' });
test('late typeahead responses cannot replace newer suggestions or revive an invalidated scope', async () => {
  const h = harness();
  h.type('old');
  h.type('new');
  assert.equal(h.pending[0].options.signal.aborted, true);
  h.pending[1].resolve({ suggestions: [hit('New title')] });
  await settle();
  h.pending[0].resolve({ suggestions: [hit('Old title')] });
  await settle();
  assert.equal(h.elements.suggestions.children.length, 1);
  assert.equal(h.elements.suggestions.children[0].children[0].textContent, 'New title');
  assert.equal(h.pending[1].url.pathname, '/secret/suggest');
  assert.equal(h.pending[1].url.searchParams.get('q'), 'new');
  h.elements.book.value = 'another-book';
  h.elements.book.fire('change');
  assert.equal(h.elements.suggestions.children.length, 0);
});
test('search renders real rows, uses the displayed offset for continuation, and preserves plain-text snippets', async () => {
  const h = harness();
  h.submit('subject');
  h.pending[0].resolve({ hits: [hit('First')], hasMore: true, errors: [] });
  await settle();
  assert.equal(h.elements.results.children[0].children[0].textContent, 'First');
  assert.equal(h.elements.results.children[0].children[1].textContent, '<em>plain text</em>');
  assert.equal(h.elements.more.hidden, false);
  h.elements.more.fire('click');
  assert.equal(h.pending[1].url.searchParams.get('offset'), '1');
  assert.equal(h.pending[1].url.searchParams.get('mode'), 'combined');
  h.pending[1].resolve({ hits: [hit('Second')], hasMore: false, errors: [] });
  await settle();
  assert.equal(h.elements.results.children.length, 2);
  assert.equal(h.elements.more.hidden, true);
  assert.equal(h.elements.search.attributes.has('aria-busy'), false);
});
test('typing retires an in-flight search and its busy state before stale results arrive', async () => {
  const h = harness();
  h.submit('old search');
  h.type('new query');
  assert.equal(h.pending[0].options.signal.aborted, true);
  assert.equal(h.elements.search.attributes.has('aria-busy'), false);
  h.pending[0].resolve({ hits: [hit('Stale result')], hasMore: true, errors: [] });
  await settle();
  assert.equal(h.elements.results.children.length, 0);
  assert.equal(h.elements.more.hidden, true);
});
