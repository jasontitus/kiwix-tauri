// node fulltext.mjs "<query>" <shot.png> â€” open reader search, set Full text mode, run query, report results.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [query, shot] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  const page = (await browser.pages())[0];
  let input = await page.$('input[placeholder="Search books"]');
  if (!input) {
    const btn = await page.$('button[aria-label="Search books"]');
    await btn.click();
    input = await page.waitForSelector('input[placeholder="Search books"]', { timeout: 5000 });
  }
  // Search options live in a <details>; open it if needed and pick Full text.
  await page.evaluate(() => {
    const s = [...document.querySelectorAll('summary')].find(e => /^Search options/.test(e.innerText.trim()));
    if (s && !s.parentElement.open) s.click();
  });
  await sleep(400);
  const sel = await page.evaluateHandle(() => [...document.querySelectorAll('select')]
    .find(s => [...s.options].some(o => o.text.trim() === 'Full text')));
  const value = await page.evaluate(s => [...s.options].find(o => o.text.trim() === 'Full text').value, sel);
  await sel.select(value);
  console.log('mode:', await page.evaluate(s => s.options[s.selectedIndex].text, sel));
  input = await page.$('input[placeholder="Search books"]');
  await input.click();
  await page.keyboard.down('Control'); await page.keyboard.press('KeyA'); await page.keyboard.up('Control');
  await page.keyboard.type(query, { delay: 25 });
  await page.keyboard.press('Enter');
  const t0 = Date.now();
  let summary = '';
  while (Date.now() - t0 < 20000) {
    await sleep(500);
    summary = await page.evaluate(() => (document.body.innerText.match(/About [\d,]+ results?|No results|\d+–\d+ of [\d,]+/g) || []).join(' | '));
    if (/About [1-9]|About 0/.test(summary)) break;
  }
  console.log(`summary after ${Date.now() - t0} ms:`, summary);
  const lines = await page.evaluate(q => document.body.innerText.split('\n').map(s => s.trim())
    .filter(l => l && (l.toLowerCase().includes(q) || /Docs$|Documentation$/.test(l))).slice(0, 14), query.toLowerCase());
  console.log('result lines:', JSON.stringify(lines, null, 1));
  if (shot) await page.screenshot({ path: shot });
} finally {
  browser.disconnect();
}
