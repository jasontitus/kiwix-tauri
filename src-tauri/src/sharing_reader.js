// SPDX-License-Identifier: GPL-3.0-or-later
(() => {
  'use strict';
  const form = document.getElementById('search');
  const q = document.getElementById('q');
  const book = document.getElementById('book');
  const mode = document.getElementById('mode');
  const suggestions = document.getElementById('suggestions');
  const results = document.getElementById('results');
  const status = document.getElementById('status');
  const more = document.getElementById('more');
  const limit = document.getElementById('limit');
  const base = new URL('./', document.currentScript.src);
  // A deep search must not grow the live list without bound; stop appending
  // and point the reader at refining the query instead.
  const MAX_RESULTS = 500;
  let timer;
  let suggestController;
  let searchController;
  let generation = 0;
  let offset = 0;
  let rendered = 0;
  let submitted;
  const params = () => new URLSearchParams({ q: q.value.trim(), book: book.value, mode: mode.value });
  const rows = (target, hits, cap = Infinity) => {
    let added = 0;
    for (const hit of hits) {
      if (added >= cap) break;
      if (typeof hit.href !== 'string') continue;
      const li = document.createElement('li');
      const a = document.createElement('a');
      a.href = hit.href;
      a.textContent = hit.title || hit.path;
      a.rel = 'noreferrer';
      li.append(a);
      if (hit.snippet) {
        const excerpt = document.createElement('small');
        excerpt.textContent = hit.snippet;
        li.append(excerpt);
      }
      target.append(li);
      added += 1;
    }
    return added;
  };
  const get = async (path, query, signal) => {
    const url = new URL(path, base);
    url.search = query.toString();
    const response = await fetch(url, { signal, credentials: 'same-origin', cache: 'no-store', referrerPolicy: 'no-referrer' });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  };
  const invalidate = () => {
    generation += 1;
    clearTimeout(timer);
    suggestController?.abort();
    status.textContent = '';
    form.removeAttribute('aria-busy');
    searchController?.abort();
    suggestions.replaceChildren();
    more.hidden = true;
    if (limit) limit.hidden = true;
  };
  const typeahead = () => {
    invalidate();
    const current = generation;
    if (!q.value.trim()) return;
    timer = setTimeout(async () => {
      suggestController = new AbortController();
      try {
        const page = await get('suggest', params(), suggestController.signal);
        if (current === generation) rows(suggestions, page.suggestions);
      } catch (error) {
        if (error.name !== 'AbortError' && current === generation) status.textContent = error.message;
      }
    }, 180);
  };
  const search = async (append) => {
    if (!append) {
      invalidate();
      submitted = params();
      offset = 0;
      rendered = 0;
      results.replaceChildren();
    }
    const current = generation;
    searchController = new AbortController();
    more.hidden = true;
    form.setAttribute('aria-busy', 'true');
    const query = new URLSearchParams(submitted);
    query.set('offset', String(offset));
    try {
      const page = await get('search', query, searchController.signal);
      if (current !== generation) return;
      rendered += rows(results, page.hits, MAX_RESULTS - rendered);
      offset += page.hits.length;
      status.textContent = page.errors.map(error => error.error).join(' · ') || page.message || '';
      more.hidden = !page.hasMore || offset >= 10000 || rendered >= MAX_RESULTS;
      if (limit) limit.hidden = rendered < MAX_RESULTS || !page.hasMore;
    } catch (error) {
      if (error.name !== 'AbortError' && current === generation) status.textContent = error.message;
    } finally {
      if (current === generation) form.removeAttribute('aria-busy');
    }
  };
  q.addEventListener('input', typeahead);
  book.addEventListener('change', typeahead);
  mode.addEventListener('change', typeahead);
  form.addEventListener('submit', event => { event.preventDefault(); void search(false); });
  more.addEventListener('click', () => { void search(true); });
  window.addEventListener('pagehide', invalidate);
})();
