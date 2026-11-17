// node download.mjs <url> <seconds> <shot.png> — start a URL download on the Downloads page and watch its status.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [url, seconds = '30', shot] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  const page = (await browser.pages())[0];
  await page.evaluate(() => [...document.querySelectorAll('button,a')]
    .find(b => (b.getAttribute('aria-label') || b.innerText).trim() === 'Downloads').click());
  await sleep(800);
  const input = await page.$('input[aria-label="Download from a URL or magnet link"]');
  await input.click();
  await page.keyboard.down('Control'); await page.keyboard.press('KeyA'); await page.keyboard.up('Control');
  await page.keyboard.press('Backspace');
  await input.type(url, { delay: 5 });
  console.log('input value:', await page.evaluate(e => e.value, input));
  await page.evaluate(() => [...document.querySelectorAll('button')].find(b => b.innerText.trim() === 'Start download').click());
  const main = () => page.evaluate(() => {
    const m = document.querySelector('main') || document.body;
    return m.innerText.split('\n').map(s => s.trim()).filter(Boolean)
      .filter(l => !/^(Search|Video formats|Walk test media|Hawaii — Offline Map)$/.test(l)).slice(0, 30);
  });
  let last = '';
  for (let t = 0; t < Number(seconds); t += 2) {
    await sleep(2000);
    const now = JSON.stringify(await main());
    if (now !== last) { console.log(`t=${t + 2}s`, now); last = now; }
  }
  const alerts = await page.evaluate(() => [...document.querySelectorAll('[role=alert],[role=status],.toast,[class*=error]')]
    .map(e => e.innerText.trim()).filter(Boolean));
  console.log('alerts/status:', JSON.stringify(alerts));
  if (shot) await page.screenshot({ path: shot });
} finally {
  browser.disconnect();
}
