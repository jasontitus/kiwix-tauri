import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";
import puppeteer from "puppeteer-core";
import { preview } from "vite";

// Run npm run build first, then set KIWIX_TEST_CHROME to a local Chromium executable.
// Without one the test is skipped locally; under CI a missing browser is a failure.
test("interactive shell navigation survives the first delayed snapshot", {
  skip: !process.env.KIWIX_TEST_CHROME && !process.env.CI && "set KIWIX_TEST_CHROME to a Chromium executable",
  timeout: 60000,
}, async (t) => {
  assert.ok(process.env.KIWIX_TEST_CHROME, "CI must set KIWIX_TEST_CHROME to a Chromium executable; this browser test may not be skipped");
  const server = await preview({
    configFile: false,
    build: { outDir: resolve(process.env.KIWIX_TEST_FRONTEND ?? "dist") },
    preview: { host: "127.0.0.1", port: 0 },
    logLevel: "silent",
  });
  t.after(() => server.close());
  const browser = await puppeteer.launch({ executablePath: process.env.KIWIX_TEST_CHROME, headless: true });
  t.after(() => browser.close());
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  for (const scenario of [
    { name: "Settings wins over saved Library", clicks: ["Settings"], saved: "library", expected: "Settings" },
    { name: "retry preserves the early choice", clicks: ["Settings"], saved: "library", retry: true, expected: "Settings" },
    { name: "the last choice, including Library, wins", clicks: ["Settings", "Library"], saved: "bookmarks", expected: "Library" },
    { name: "no click restores the saved pane", clicks: [], saved: "bookmarks", expected: "Reading list" },
    { name: "external search still wins", clicks: ["Settings"], saved: "bookmarks", search: true, expected: "Library" },
  ]) {
    await t.test(scenario.name, async () => {
      const page = await browser.newPage();
      const errors = [];
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("pageerror", error => errors.push(String(error)));
      try {
        await page.setViewport({ width: 412, height: 850, isMobile: true });
        await page.evaluateOnNewDocument(({ saved, search, retry }) => {
          window.failSnapshot = retry;
          let release;
          const pending = new Promise(resolve => { release = resolve; });
          window.releaseSnapshot = release;
          const filters = { languages: [], categories: [], content: [] };
          const snapshot = {
            activeSurface: saved, activeTab: null, books: [], bookmarks: [], history: [],
            recentSearches: [], tabs: [], launchLocations: [], referenceTransitions: [],
            launchSearch: search ? { launchId: "test-launch", query: "climate" } : null,
            privacyGeneration: 1, privacyPolicyGeneration: 1, referenceRevision: 0,
            notifications: { available: false }, sharing: { running: false, books: [], bookIds: [] },
            settings: {
              restoreSession: true, locale: "en", theme: "system", privateMode: false, readOnly: false,
              opdsSources: [], monitorDirectories: [], pinnedBookIds: [], bookZoom: {}, catalogFilters: filters, libraryFilters: filters,
              sharingName: "", sharingBookIds: [], speechPreferences: {}, readerTheme: "original", fontSize: 18, zoom: 1,
            },
          };
          const original = window.fetch.bind(window);
          window.fetch = async (url, options) => {
            const target = new URL(url, location.href);
            if (target.origin !== location.origin) throw new Error("unexpected external request");
            if (target.pathname !== "/api") return original(url, options);
            const { command } = JSON.parse(options.body);
            if (command === "search_session") return Response.json({ result: { sessionId: "test-session" } });
            if (command === "snapshot") {
              window.snapshotRequested = true;
              await pending;
              if (window.failSnapshot) return Response.json({ error: "test service unavailable" }, { status: 503 });
              return Response.json({ result: snapshot });
            }
            // Keep unrelated pane requests in flight while observing the first committed surface.
            return new Promise(() => {});
          };
        }, scenario);
        await page.goto(base + "/#token=test", { waitUntil: "domcontentloaded" });
        await page.waitForFunction(() => window.snapshotRequested);
        const tabDisabledAtStartup = await page.$eval(".new-tab", button => button.disabled);
        const current = '[aria-current="page"]';
        const sidebar = (label) => `aside.sidebar button[aria-label="${label}"]`;
        for (const label of scenario.clicks) {
          await page.click(sidebar(label));
          assert.equal(await page.$eval(sidebar(label), node => node.getAttribute("aria-current")), "page");
        }
        await page.evaluate(() => window.releaseSnapshot());
        if (scenario.retry) {
          const retryButton = "main .error-notice button";
          await page.waitForSelector(retryButton);
          await page.evaluate(() => { window.failSnapshot = false; });
          await page.click(retryButton);
        }
        await page.waitForFunction(() => window.__KIWIX_BOOT_STATE__?.ready);
        await page.waitForFunction(() => document.querySelector("main h1"));
        assert.equal(await page.$eval("main h1", node => node.textContent.trim()), scenario.expected, errors.join("\n"));
        assert.equal(await page.$eval(`aside.sidebar ${current}`, node => node.getAttribute("aria-label")), scenario.expected);
        assert.equal(tabDisabledAtStartup, true);
        assert.equal(await page.$eval(".new-tab", button => button.disabled), false);
      } finally {
        await page.close();
      }
    });
  }
});
