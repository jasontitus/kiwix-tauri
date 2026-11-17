// node nearby-dl.mjs "<archive title>" [nth]: click Download next to a title in the open nearby-device dialog.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [title, nth = '0'] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
try {
  const page = (await browser.pages())[0];
  const box = await page.evaluate((title, nth) => {
    const root = document.querySelector('dialog[open]') || document.body;
    const buttons = [...root.querySelectorAll('button')].filter(b => b.innerText.trim() === 'Download');
    const matches = buttons.filter(b => {
      // The row is the largest ancestor that still holds only this button.
      let p = b.parentElement;
      while (p.parentElement && [...p.parentElement.querySelectorAll('button')]
        .filter(x => x.innerText.trim() === 'Download').length === 1) p = p.parentElement;
      return p.innerText.split('\n').map(s => s.trim()).includes(title);
    });
    const b = matches[Number(nth)];
    if (!b) return null;
    b.scrollIntoView({ block: 'center' });
    const r = b.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2, count: matches.length };
  }, title, nth);
  if (!box) throw new Error(`no Download button for "${title}"`);
  await page.mouse.click(box.x, box.y);
  console.log('clicked Download for', title, `(${box.count} candidate(s))`);
} finally {
  browser.disconnect();
}
