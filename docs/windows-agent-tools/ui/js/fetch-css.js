const out = {};
for (const p of ['/assets/index.css', '/application.css']) {
  try {
    const r = await fetch(p, { cache: 'no-store' });
    const body = await r.text();
    out[p] = { status: r.status, redirected: r.redirected, finalUrl: r.url, type: r.type, ctype: r.headers.get('content-type'),
      acao: r.headers.get('access-control-allow-origin'), len: body.length, head: body.slice(0, 120) };
  } catch (e) { out[p] = 'fetch error: ' + e; }
}
const link = [...document.querySelectorAll('link[rel=stylesheet]')].map(l => ({ href: l.getAttribute('href'), crossorigin: l.getAttribute('crossorigin') }));
const entries = performance.getEntriesByType('resource').filter(e => /index\.css|application\.css/.test(e.name))
  .map(e => ({ name: e.name, status: e.responseStatus, initiator: e.initiatorType, transfer: e.transferSize, redirectEnd: e.redirectEnd }));
return { out, link, entries };
