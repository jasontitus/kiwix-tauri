// node click-text.mjs "<button text>" [scopeSelector] — trusted mouse click on a visible button by its text.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [text, scope = 'body'] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
try {
  const page = (await browser.pages())[0];
  const box = await page.evaluate((t, s) => {
    const roots = [...document.querySelectorAll(s)];
    for (const root of roots) {
      const b = [...root.querySelectorAll('button')].find(e => (e.getAttribute('aria-label') || e.innerText).trim() === t);
      if (b) { b.scrollIntoView({ block: 'center' }); const r = b.getBoundingClientRect(); if (r.width) return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; }
    }
    return null;
  }, text, scope);
  if (!box) throw new Error(`no visible button "${text}" in ${scope}`);
  await page.mouse.click(box.x, box.y);
  console.log('clicked', text, 'at', Math.round(box.x), Math.round(box.y));
} finally {
  browser.disconnect();
}
