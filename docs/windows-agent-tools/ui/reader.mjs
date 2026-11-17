// node reader.mjs "<tab title>" "<find text>" <shot.png> — W4 TOC + find-in-article checks.
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');
const [tabTitle, findText, shot] = process.argv.slice(2);
const browser = await puppeteer.connect({ browserURL: 'http://127.0.0.1:9222', defaultViewport: null });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const clickLabel = (page, label) => page.evaluate(l => {
  const b = [...document.querySelectorAll('button,[role=tab]')].find(e => (e.getAttribute('aria-label') || e.innerText).trim() === l);
  if (!b) return false; b.click(); return true;
}, label);
try {
  const page = (await browser.pages())[0];
  console.log('tab clicked:', await clickLabel(page, tabTitle));
  await sleep(1500);
  console.log('toc clicked:', await clickLabel(page, 'Table of contents'));
  await sleep(1000);
  const toc = await page.evaluate(() => {
    const panel = [...document.querySelectorAll('[role=dialog],[role=navigation],aside,nav')]
      .find(e => /contents/i.test(e.getAttribute('aria-label') || e.innerText.slice(0, 40)));
    return panel ? panel.innerText.split('\n').filter(Boolean).slice(0, 15) : 'no toc panel found';
  });
  console.log('toc:', JSON.stringify(toc));
  await page.keyboard.press('Escape');
  console.log('find clicked:', await clickLabel(page, 'Find in article'));
  await sleep(700);
  await page.keyboard.type(findText, { delay: 30 });
  await sleep(1200);
  const find = await page.evaluate(() => {
    const a = document.activeElement;
    const bar = a && a.closest('div,form,section');
    return { focused: a && (a.getAttribute('aria-label') || a.placeholder), bar: bar ? bar.innerText.replace(/\s+/g, ' ').slice(0, 120) : null };
  });
  console.log('find:', JSON.stringify(find));
  await page.keyboard.press('Enter');
  await sleep(600);
  const frame = page.frames().find(f => f !== page.mainFrame());
  const marks = frame ? await frame.evaluate(() => ({
    highlights: document.querySelectorAll('mark, .kiwix-find-match, [data-find-match]').length,
    selection: String(getSelection()),
  })) : null;
  console.log('frame:', JSON.stringify(marks));
  if (shot) await page.screenshot({ path: shot });
} finally {
  browser.disconnect();
}
