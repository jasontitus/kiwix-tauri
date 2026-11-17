// node export-menu.mjs "<action text>" — open reader panel containing "Print and export" and trusted-click an action.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [action] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const center = (page, fn, arg) => page.evaluate(fn, arg);
try {
  const page = (await browser.pages())[0];
  // Close the search panel if open.
  await page.evaluate(() => {
    const b = document.querySelector('button[aria-label="Search books"][aria-pressed="true"]');
    if (b) b.click();
  });
  await sleep(500);
  const hasSummary = () => page.evaluate(() => [...document.querySelectorAll('summary')].some(s => s.innerText.trim() === 'Print and export' && s.getBoundingClientRect().width));
  for (const label of ['Reading appearance', 'Archive permissions', 'Bookmark and add notes', 'Table of contents']) {
    if (await hasSummary()) break;
    const box = await center(page, l => {
      const b = [...document.querySelectorAll('button')].find(e => e.getAttribute('aria-label') === l && e.getBoundingClientRect().width);
      if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    }, label);
    if (!box) continue;
    await page.mouse.click(box.x, box.y);
    await sleep(800);
    if (await hasSummary()) { console.log('panel:', label); break; }
    await page.mouse.click(box.x, box.y); // close it again
    await sleep(400);
  }
  if (!(await hasSummary())) throw new Error('Print and export section not found');
  await page.evaluate(() => {
    const s = [...document.querySelectorAll('summary')].find(e => e.innerText.trim() === 'Print and export');
    if (!s.parentElement.open) s.click();
  });
  await sleep(500);
  const box = await center(page, t => {
    const b = [...document.querySelectorAll('button.rw-action-row')].find(e => e.innerText.trim().startsWith(t));
    if (!b) return null; b.scrollIntoView({ block: 'center' });
    const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, disabled: b.disabled };
  }, action);
  if (!box) throw new Error(`action ${action} not found`);
  console.log('action disabled:', box.disabled);
  await page.mouse.click(box.x, box.y);
  console.log('clicked', action);
} finally {
  browser.disconnect();
}
