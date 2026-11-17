// node search.mjs "<query>" <shot.png>  — reader "Search books" flow; prints visible result text.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [query, shot] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  const page = (await browser.pages())[0];
  await page.keyboard.press('Escape');
  await page.evaluate(() => [...document.querySelectorAll('button')].find(b => b.getAttribute('aria-label') === 'Search books').click());
  await sleep(700);
  const field = await page.evaluate(() => {
    const a = document.activeElement;
    return a ? `${a.tagName} ${a.getAttribute('aria-label') || a.placeholder || ''}` : 'none';
  });
  console.log('focused:', field);
  await page.keyboard.type(query, { delay: 30 });
  await sleep(3000);
  const before = await page.evaluate(() => {
    const box = document.querySelector('[role=listbox], [role=dialog], .search-results, [class*=suggest]');
    return (box || document.body).innerText.split('\n').map(s => s.trim()).filter(Boolean).slice(0, 40);
  });
  console.log('suggestions:', JSON.stringify(before, null, 1));
  await page.keyboard.press('Enter');
  await sleep(3500);
  const after = await page.evaluate(() => document.body.innerText.split('\n').map(s => s.trim()).filter(Boolean).slice(0, 60));
  console.log('after enter:', JSON.stringify(after, null, 1));
  if (shot) await page.screenshot({ path: shot });
} finally {
  browser.disconnect();
}
