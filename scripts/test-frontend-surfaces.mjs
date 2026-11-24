import assert from "node:assert/strict";
import test from "node:test";
import puppeteer from "puppeteer-core";
import { createServer } from "vite";

// Run against the development bundle to exercise React StrictMode's effect replay.
// The browser owns a disposable profile and all service calls use this test fixture.
test("archive dialogs and editors recover without losing the current interaction", {
  skip: !process.env.KIWIX_TEST_CHROME && "set KIWIX_TEST_CHROME to a Chromium executable",
  timeout: 60000,
}, async (t) => {
  const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: true }, logLevel: "silent" });
  await server.listen();
  t.after(() => server.close());
  const browser = await puppeteer.launch({ executablePath: process.env.KIWIX_TEST_CHROME, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
  await page.setViewport({ width: 1280, height: 900 });
  await page.setRequestInterception(true);
  page.on("request", (request) => {
    if (new URL(request.url()).origin === origin || request.url().startsWith("data:")) void request.continue();
    else void request.abort();
  });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    const filters = { languages: [], categories: [], content: [] };
    const metadata = {
      id: "fixture-archive", title: "Example archive", description: "An offline archive", language: "eng",
      creator: "Fixture", publisher: "Fixture", date: "2026-01-01", name: "example_eng",
      tags: ["_category:wikipedia", "_pictures:yes", "_sw:yes", "Reference"],
      size: 1024, articleCount: 1, mediaCount: 0, hasFulltextIndex: false, hasTitleIndex: true, mainPath: "Example",
    };
    const snapshot = {
      activeSurface: "catalog", activeTab: null,
      books: [{ ...metadata, id: "installed", name: "installed_eng", installationId: "fixture-installation",
        path: "/fixture/installed.zim", addedAt: "2026-01-01", available: true, managed: true }],
      bookmarks: [{ id: "bookmark", bookId: "installed", path: "Example", title: "Saved example", note: "",
        tags: ["one", "two"], highlights: [], createdAt: "2026-01-01" }],
      history: [], recentSearches: [], tabs: [], launchLocations: [], referenceTransitions: [],
      privacyGeneration: 1, privacyPolicyGeneration: 1, referenceRevision: 0,
      notifications: { available: false }, sharing: { running: false, books: [], bookIds: [] },
      settings: {
        restoreSession: true, locale: "en", theme: "system", privateMode: false, readOnly: false,
        catalogUrl: "broken URL", opdsSources: ["https://catalog.fixture.invalid/entries"], catalogFiltersCustomized: true,
        catalogFilters: filters, libraryFilters: filters, monitorDirectories: [], pinnedBookIds: [], bookZoom: {},
        sharingName: "", sharingBookIds: [], speechPreferences: {}, readerTheme: "original", fontSize: 18, zoom: 1,
      },
    };
    const map = { format: "fixture", regions: [{ id: "fixture-area", title: "Fixture area",
      base: { file: "base", size: 10, download: "" },
      menu: [{ lang: "en", label: "English", name: "map_en", recipe: "map", size: 20, downloadBytes: 10, md5: "fixture" }],
    }] };
    const original = window.fetch.bind(window);
    window.calls = [];
    window.fetch = async (url, options) => {
      const target = new URL(url, location.href);
      if (target.origin !== location.origin) throw new Error("Unexpected external request");
      if (target.pathname !== "/api") return original(url, options);
      const { command, args } = JSON.parse(options.body);
      window.calls.push({ command, args });
      const results = {
        snapshot, catalog: { entries: [{ ...metadata, url: "https://archive.fixture.invalid/example.zim", thumbnailUrl: null }], next: null, total: 1 },
        catalog_facets: { languages: [], categories: [] }, map_catalog: map, map_status: null,
        downloads: [], edition_status: { intents: [], transactions: [] }, search_session: { sessionId: "fixture-session" },
      };
      const errors = { start_download: "Fixture download rejected", map_recipe_metadata: "Fixture estimate unavailable",
        map_prepare: "Fixture preparation rejected" };
      return errors[command] ? Response.json({ error: errors[command] }, { status: 400 }) : Response.json({ result: results[command] ?? null });
    };
  });
  await page.goto(origin + "/#token=fixture", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.__KIWIX_BOOT_STATE__?.ready);

  const visibleText = (selector, text) => page.waitForFunction((sel, value) =>
    [...document.querySelectorAll(sel)].some((node) => node.textContent.includes(value)), {}, selector, text);
  const button = async (scope, text) => {
    const handle = await page.waitForFunction((sel, value) => [...document.querySelectorAll(`${sel} button`)]
      .find((node) => !node.disabled && node.textContent.trim() === value), {}, scope, text);
    await handle.asElement().click();
    await handle.dispose();
  };
  const calls = (command) => page.evaluate((name) => window.calls.filter((call) => call.command === name), command);
  const pane = async (label, title) => {
    await page.click(`aside.sidebar button[aria-label^="${label}"]`);
    await visibleText("main h1", title);
  };

  await pane("Discover", "Discover");
  await visibleText(".catalog-page [role=alert]", "The catalog source URL is invalid.");
  assert.equal((await calls("catalog")).length + (await calls("catalog_facets")).length, 0);
  await page.click(".catalog-options > summary");
  await page.select(".catalog-source select", "https://catalog.fixture.invalid/entries");
  await button(".catalog-card-content", "Example archive");
  assert.deepEqual(await page.$$eval("dialog[open] .tag-list .badge", (nodes) => nodes.map((node) => node.textContent)), ["Reference"]);
  await button("dialog[open]", "Download archive");
  await page.waitForSelector("dialog.modal-confirmation[open] button:focus");
  assert.equal(await page.$eval("dialog.modal-confirmation[open] button:focus", (node) => node.textContent.trim()), "Cancel");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.activeElement?.textContent.trim() === "Download archive");
  assert.equal((await calls("start_download")).length, 0);
  await button("dialog[open]", "Download archive");
  await button("dialog.modal-confirmation[open]", "Continue");
  await visibleText("dialog[open] [role=alert]", "Fixture download rejected");
  assert.equal(await page.$$eval("[role=alert]", (nodes) => nodes.filter((node) => node.textContent.includes("Fixture download rejected")).length), 1);
  await page.click('dialog[open] button[aria-label="Dismiss error"]');
  await page.waitForFunction(() => !document.querySelector("dialog[open] [role=alert]"));
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.activeElement?.textContent.trim() === "Example archive");

  await pane("Reading list", "Reading list");
  assert.equal(await page.$eval(".reading-card h2 > button", (node) => node.textContent.trim()), "Saved example");
  assert.equal(await page.$eval(".reading-list .swipe-row", (node) => node.getAttribute("tabindex")), null);
  await button(".reading-card", "Edit note and tags");
  const tags = "dialog[open] input[placeholder='research, travel, favorites']";
  await page.click(tags);
  await page.keyboard.press("End");
  for (let n = 0; n < " two".length; n++) await page.keyboard.press("Backspace");
  assert.equal(await page.$eval(tags, (node) => node.value), "one,");
  await page.keyboard.press("Backspace");
  assert.equal(await page.$eval(tags, (node) => node.value), "one");
  await page.type(tags, ", three, one");
  await button("dialog[open]", "Save changes");
  assert.deepEqual((await calls("save_bookmark"))[0]?.args.bookmark.tags, ["one", "three"]);

  await pane("Maps", "Maps");
  await page.waitForSelector(".map-card");
  assert.equal(await page.$$eval(".map-card select option", (nodes) => nodes.length), 0);
  await button(".map-card", "Fixture area");
  await visibleText("dialog[open] [role=alert]", "Fixture estimate unavailable");
  const estimates = (await calls("map_recipe_metadata")).length;
  const registries = (await calls("map_catalog")).length;
  await button("dialog[open]", "Try again");
  await page.waitForFunction((count) => window.calls.filter((call) => call.command === "map_recipe_metadata").length > count, {}, estimates);
  assert.equal((await calls("map_catalog")).length, registries);
  await button("dialog[open]", "Get map");
  await visibleText("dialog[open] [role=alert]", "Fixture preparation rejected");
  assert.equal((await calls("map_start")).length, 0);
  assert.deepEqual(pageErrors, []);
});
