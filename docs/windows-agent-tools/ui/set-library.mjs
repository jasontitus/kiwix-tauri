// node set-library.mjs <dir> — set Settings > Download directory via the UI and accept the confirmation.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [dir] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  const page = (await browser.pages())[0];
  await page.evaluate(() => [...document.querySelectorAll('button,a')]
    .find(b => (b.getAttribute('aria-label') || b.innerText).trim() === 'Settings').click());
  await sleep(1000);
  await page.evaluate(() => { for (const s of document.querySelectorAll('summary')) if (/Files & downloads/i.test(s.innerText) && !s.parentElement.open) s.click(); });
  await sleep(600);
  const input = await page.evaluateHandle(() => [...document.querySelectorAll('label.field')]
    .find(l => /Download directory/i.test(l.innerText))?.querySelector('input'));
  await input.click();
  await page.keyboard.down('Control'); await page.keyboard.press('KeyA'); await page.keyboard.up('Control');
  await page.keyboard.press('Backspace');
  await input.type(dir, { delay: 10 });
  await page.keyboard.press('Tab');
  await sleep(2500);
  const prompt = await page.evaluate(() => [...document.querySelectorAll('[role=dialog],[role=alertdialog],.confirm,section')]
    .map(e => e.innerText.trim()).filter(t => /download directory|location|→|move/i.test(t)).slice(0, 3));
  console.log('confirmation UI:', JSON.stringify(prompt));
  const buttons = await page.evaluate(() => [...document.querySelectorAll('button')].map(b => b.innerText.trim()).filter(Boolean)
    .filter(t => /confirm|change|use|keep|apply|save|yes|move/i.test(t)));
  console.log('candidate buttons:', JSON.stringify(buttons));
  if (shot()) await page.screenshot({ path: 'E:/kiwix-win/ui/11-library-location.png' });
  function shot() { return true; }
} finally {
  browser.disconnect();
}
