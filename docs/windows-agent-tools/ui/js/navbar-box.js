const el = document.querySelector('devdocs-navbar');
const r = el.getBoundingClientRect();
return { rects: el.getClientRects().length, w: r.width, h: r.height, children: el.childNodes.length,
  display: getComputedStyle(el).display, scripts: [...document.scripts].map(s => s.src || 'inline').slice(0, 10) };
