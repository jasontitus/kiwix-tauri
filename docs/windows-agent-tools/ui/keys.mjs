// node keys.mjs "<tab title>" "Control+S" — activate a reader tab, then press a trusted key chord on the main page.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [tabTitle, chord] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  const page = (await browser.pages())[0];
  if (tabTitle) {
    const box = await page.evaluate(t => {
      const tab = [...document.querySelectorAll('[role=tab], button')].find(e => e.innerText.trim() === t);
      if (!tab) return null; tab.scrollIntoView({ block: 'nearest', inline: 'center' });
      const r = tab.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    }, tabTitle);
    if (!box) throw new Error(`tab ${tabTitle} not found`);
    await page.mouse.click(box.x, box.y);
    await sleep(1500);
  }
  const keys = chord.split('+');
  for (const k of keys.slice(0, -1)) await page.keyboard.down(k);
  await page.keyboard.press(keys.at(-1));
  for (const k of keys.slice(0, -1).reverse()) await page.keyboard.up(k);
  console.log('pressed', chord, 'on', tabTitle || '(current)');
} finally {
  browser.disconnect();
}
