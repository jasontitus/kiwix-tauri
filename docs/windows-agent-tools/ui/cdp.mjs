// Drive the running Kiwix WebView2 over CDP. Usage:
//   node cdp.mjs shot <out.png>            screenshot main page
//   node cdp.mjs eval "<js expression>"    evaluate in main page, print JSON
//   node cdp.mjs pages                     list targets
import { createRequire } from 'node:module';
const require = createRequire('E:/kiwix-win/lf-check/package.json');
const puppeteer = require('puppeteer-core');

const [cmd, arg, arg2] = process.argv.slice(2);
const port = process.env.CDP_PORT || '9222';
const browser = await puppeteer.connect({ browserURL: `http://127.0.0.1:${port}`, defaultViewport: null });
try {
  const pages = await browser.pages();
  const pick = process.env.CDP_PAGE ? pages.find(p => p.url().includes(process.env.CDP_PAGE)) : pages[0];
  if (cmd === 'pages') {
    for (const p of pages) console.log(p.url(), '|', await p.title());
  } else if (cmd === 'shot') {
    await pick.screenshot({ path: arg });
    console.log('saved', arg, pick.url());
  } else if (cmd === 'eval') {
    const out = await pick.evaluate(new Function(`return (async () => (${arg}))()`));
    console.log(JSON.stringify(out, null, 2));
  } else if (cmd === 'evalfile') {
    const { readFileSync } = await import('node:fs');
    const out = await pick.evaluate(new Function(`return (async () => {\n${readFileSync(arg, 'utf8')}\n})()`));
    console.log(JSON.stringify(out, null, 2));
  } else if (cmd === 'fevalfile') {
    const { readFileSync } = await import('node:fs');
    const frames = pick.frames().filter(f => f !== pick.mainFrame());
    const frame = process.env.CDP_FRAME ? frames.find(f => f.url().includes(process.env.CDP_FRAME)) : frames[0];
    for (const f of frames) console.error('frame:', f.url());
    const out = await frame.evaluate(new Function(`return (async () => {\n${readFileSync(arg, 'utf8')}\n})()`));
    console.log(JSON.stringify(out, null, 2));
  } else if (cmd === 'click') {
    await pick.click(arg);
    console.log('clicked', arg);
  } else if (cmd === 'type') {
    await pick.type(arg, arg2, { delay: 20 });
    console.log('typed into', arg);
  } else if (cmd === 'key') {
    await pick.keyboard.press(arg);
    console.log('pressed', arg);
  } else if (cmd === 'frames') {
    for (const f of pick.frames()) console.log(f === pick.mainFrame() ? '[main]' : '[child]', f.url());
  } else if (cmd === 'feval') {
    // CDP_FRAME selects the frame by URL substring; default = first child frame.
    const frames = pick.frames().filter(f => f !== pick.mainFrame());
    const frame = process.env.CDP_FRAME ? frames.find(f => f.url().includes(process.env.CDP_FRAME)) : frames[0];
    if (!frame) throw new Error('no matching frame');
    const out = await frame.evaluate(new Function(`return (async () => (${arg}))()`));
    console.log(JSON.stringify(out, null, 2));
  } else {
    console.error('unknown command');
    process.exitCode = 2;
  }
} finally {
  browser.disconnect();
}
