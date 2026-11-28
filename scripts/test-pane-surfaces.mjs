import assert from "node:assert/strict";
import test from "node:test";
import puppeteer from "puppeteer-core";
import { createServer } from "vite";

// Real-DOM checks of the Library, Discover, Maps and Downloads panes, run
// against the development bundle so React StrictMode replays every effect.
// Same harness as test-frontend-surfaces.mjs: a disposable browser profile and
// a service fixture; nothing leaves the test origin.
const skip = !process.env.KIWIX_TEST_CHROME && !process.env.CI && "set KIWIX_TEST_CHROME to a Chromium executable";

// Installed before any page script. Tests change `window.results`,
// `window.failures` and `window.delays` to steer the service.
function fixture() {
  const filters = { languages: [], categories: [], content: [] };
  const metadata = {
    id: "fixture-archive", title: "Example archive", description: "An offline archive", language: "eng",
    creator: "Fixture", publisher: "Fixture", date: "2026-01-01", name: "example_eng",
    tags: ["_category:wikipedia", "Reference"],
    size: 1024, articleCount: 1, mediaCount: 0, hasFulltextIndex: true, hasTitleIndex: true, mainPath: "Example",
  };
  const installed = { ...metadata, id: "installed", title: "Installed archive", name: "installed_eng",
    installationId: "fixture-installation", path: "/fixture/installed.zim", addedAt: "2026-01-01",
    available: true, managed: true };
  const snapshot = {
    activeSurface: "library", activeTab: null, books: [installed],
    bookmarks: [], history: [], recentSearches: [], tabs: [], launchLocations: [], referenceTransitions: [],
    privacyGeneration: 1, privacyPolicyGeneration: 1, referenceRevision: 0,
    notifications: { available: false }, sharing: { running: false, books: [], bookIds: [] },
    settings: {
      restoreSession: true, locale: "en", theme: "system", privateMode: false, readOnly: false,
      catalogUrl: "https://catalog.fixture.invalid/entries", opdsSources: [], catalogFiltersCustomized: true,
      catalogFilters: filters, libraryFilters: filters, monitorDirectories: [], pinnedBookIds: [], bookZoom: {},
      sharingName: "", sharingBookIds: [], speechPreferences: {}, readerTheme: "original", fontSize: 18, zoom: 1,
      downloadDirectory: "/fixture/downloads", trashDeletedFiles: false, showSearchSnippets: true,
    },
  };
  // Enough cards that the last one is far below the top of the scrolled page.
  const entries = Array.from({ length: 40 }, (_, index) => ({
    ...metadata, id: `entry-${index}`, title: `Entry ${index}`, name: `entry_${index}`,
    url: `https://archive.fixture.invalid/entry_${index}.zim`, thumbnailUrl: null,
  }));
  const regions = Array.from({ length: 40 }, (_, index) => ({
    id: `area-${String(index).padStart(2, "0")}`, title: `Area ${String(index).padStart(2, "0")}`,
    base: { file: "base", size: 10, download: "" },
    menu: [{ lang: "en", label: "English", name: `map_${index}`, recipe: "map", size: 20, downloadBytes: 10, md5: "fixture" }],
  }));
  window.results = {
    snapshot,
    catalog: { entries, next: null, total: entries.length },
    catalog_facets: { languages: [], categories: [] },
    map_catalog: { format: "fixture", regions },
    map_status: null, downloads: [],
    integrity_status: { job: { running: false, phase: "idle", bookId: null, completed: 0, total: 0,
      bytesChecked: 0, totalBytes: 0, targets: [], results: [] }, books: [] }, edition_status: { intents: [], transactions: [] },
    search_session: { sessionId: "fixture-session" }, search_owner: { ownerId: "fixture-owner" },
    suggest: { suggestions: [], hasMore: false, nextToken: null, errors: [] },
    search: { hits: [], total: 0, hasMore: false, errors: [], spellingStatus: [],
      corrections: [{ originalQuery: "animl", correctedQuery: "animal" }] },
  };
  window.failures = {};
  window.delays = {};
  window.calls = [];
  const original = window.fetch.bind(window);
  window.fetch = async (url, options) => {
    const target = new URL(url, location.href);
    if (target.origin !== location.origin) throw new Error("Unexpected external request");
    if (target.pathname !== "/api") return original(url, options);
    const { command, args } = JSON.parse(options.body);
    window.calls.push({ command, args });
    if (window.delays[command]) await new Promise((resolve) => setTimeout(resolve, window.delays[command]));
    if (window.failures[command]) return Response.json({ error: window.failures[command] }, { status: 400 });
    const result = window.results[command];
    return Response.json({ result: typeof result === "function" ? result(args) : result ?? null });
  };
}

async function open(t) {
  assert.ok(process.env.KIWIX_TEST_CHROME, "CI must set KIWIX_TEST_CHROME to a Chromium executable; this browser test may not be skipped");
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
  await page.evaluateOnNewDocument(fixture);
  page.setDefaultTimeout(45000);
  await page.goto(origin + "/#token=fixture", { waitUntil: "domcontentloaded", timeout: 120000 });
  await page.waitForFunction(() => window.__KIWIX_BOOT_STATE__?.ready, { timeout: 120000 });
  // Every wait is on a condition; the limit only bounds a real defect.
  const until = (check, ...args) => page.waitForFunction(check, { timeout: 45000 }, ...args);
  const button = async (scope, text) => {
    const handle = await until((sel, value) => [...document.querySelectorAll(`${sel} button`)]
      .find((node) => !node.disabled && node.getClientRects().length && node.textContent.trim() === value), scope, text);
    await handle.asElement().click();
    await handle.dispose();
  };
  const pane = async (label, title) => {
    await page.click(`aside.sidebar button[aria-label^="${label}"]`);
    await until((value) => document.querySelector("main h1")?.textContent.includes(value), title);
  };
  const calls = (command) => page.evaluate((name) => window.calls.filter((call) => call.command === name), command);
  return { page, pageErrors, until, button, pane, calls };
}

// How many painted surfaces carry this text, and whether a top-layer alert does.
const painted = (text) => ({
  copies: [...document.querySelectorAll("[role=alert], [role=alertdialog], .global-error")]
    .filter((node) => node.getClientRects().length && node.textContent.includes(text) &&
      !node.parentElement?.closest("[role=alert], [role=alertdialog], .global-error")).length,
  alertDialog: !!document.querySelector("dialog[role=alertdialog][open]"),
});
// The band holds the text and is inside the viewport.
const bandVisible = (text) => {
  const band = document.querySelector(".global-error");
  if (!band || !band.textContent.includes(text)) return false;
  const rect = band.getBoundingClientRect();
  return rect.bottom > 0 && rect.top < window.innerHeight && rect.height > 0;
};

test("a card failure far down Discover or Maps is shown where it can be seen", { skip, timeout: 240000 }, async (t) => {
  const { page, pageErrors, until, button, pane, calls } = await open(t);
  await page.evaluate(() => {
    window.failures.start_download = "Fixture download rejected";
    window.failures.map_prepare = "Fixture preparation rejected";
  });
  await pane("Discover", "Discover");
  await until(() => document.querySelectorAll(".catalog-card").length === 40);
  await page.evaluate(() => document.getElementById("main-content").scrollTo(0, 1e6));
  await until(() => document.getElementById("main-content").scrollTop > 500);
  await page.click('button[aria-label="Download Entry 39"]');
  await until(bandVisible, "Fixture download rejected");
  assert.equal((await calls("start_download")).length, 1);
  assert.deepEqual(await page.evaluate(painted, "Fixture download rejected"), { copies: 1, alertDialog: false });
  assert.ok(await page.evaluate(() => document.getElementById("main-content").scrollTop > 500), "the page was not scrolled for it");
  await page.click('.global-error button[aria-label="Dismiss error"]');
  await until(() => !document.querySelector(".global-error"));

  await pane("Maps", "Maps");
  await until(() => document.querySelectorAll(".map-card").length === 40);
  await page.evaluate(() => document.getElementById("main-content").scrollTo(0, 1e6));
  await until(() => document.getElementById("main-content").scrollTop > 500);
  const last = await until(() => [...document.querySelectorAll(".map-card")].at(-1)
    ?.querySelector(".map-card-controls button:not(.map-language-button):not(:disabled)"));
  await last.asElement().click();
  await until(bandVisible, "Fixture preparation rejected");
  assert.deepEqual(await page.evaluate(painted, "Fixture preparation rejected"), { copies: 1, alertDialog: false });

  // Delete can keep a file it cannot prove is its own: the job stays listed.
  const job = {
    id: "map-job", regionId: "area-00", language: "en", status: "paused", written: 5, total: 20,
    incoming: 0, incomingTotal: 0, speed: 0, resumable: true, canRemoveFromList: false, error: null, path: null,
  };
  const kept = "/fixture/maps/.streetzim-map.part";
  await page.evaluate((value) => { window.results.map_status = value; }, job);
  await pane("Library", "Library");
  await pane("Maps", "Maps");
  await until(() => document.querySelector(".map-job"));
  await page.evaluate((value, path) => {
    window.results.map_delete_download = () => {
      window.results.map_status = { ...value, status: "cancelled", resumable: false, recoveryPaths: [path],
        error: "Some earlier map downloads could not be cleaned up. The files were kept here: " + path };
      return null;
    };
  }, job, kept);
  const remove = await until(() => [...document.querySelectorAll(".map-job button")]
    .find((node) => node.textContent.trim() === "Delete download" && !node.disabled));
  await remove.asElement().click();
  await until(() => document.querySelector("dialog[open] .download-cleanup-actions"));
  // A trusted click: the dialog ignores synthetic activation.
  await button("dialog[open] .download-cleanup-actions", "Delete download and files");
  await until(() => !document.querySelector("dialog[open]"));
  const dismiss = await until(() => [...document.querySelectorAll(".map-job .map-recovery button")]
    .find((node) => node.textContent.trim() === "Dismiss (keeps the file)" && !node.disabled));
  assert.equal(await dismiss.evaluate((node) => node.getAttribute("aria-describedby")
    .split(" ").map((id) => document.getElementById(id).textContent).join(" ").includes(".streetzim-map.part")), true);
  assert.equal((await calls("map_delete_download")).length, 1);
  await page.evaluate(() => { window.results.map_status = { ...window.results.map_status, recoveryPaths: undefined }; });
  await dismiss.asElement().click();
  await until(() => !document.querySelector(".map-recovery"));
  assert.deepEqual((await calls("map_forget_recovery")).map((call) => call.args), [{ path: kept, privacyGeneration: 1 }]);
  assert.deepEqual(pageErrors, []);
});

test("Library removal, search and spelling suggestions each have one visible outcome", { skip, timeout: 240000 }, async (t) => {
  const { page, pageErrors, until, button, calls } = await open(t);
  await until(() => document.querySelector(".book-card"));
  await page.evaluate(() => { window.failures.remove_book = "Fixture removal refused"; });
  await page.click('button[aria-label="Details and actions for Installed archive"]');
  await button("dialog[open] .archive-detail-actions", "Remove from library");
  await until(() => document.querySelector("dialog[open] .archive-removal-actions"));
  await button("dialog[open] .archive-removal-actions", "Remove from library");
  await until((text) => document.querySelector("dialog[open]")?.textContent.includes(text), "Fixture removal refused");
  // Let a second owner, if any, appear before counting.
  await until(() => window.calls.filter((call) => call.command === "remove_book").length === 1 &&
    !document.querySelector("dialog[open] .archive-removal-actions button:disabled"));
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.deepEqual(await page.evaluate(painted, "Fixture removal refused"), { copies: 1, alertDialog: false });
  await button("dialog[open] .archive-removal-actions", "Cancel");
  await until(() => document.querySelector("dialog[open]")?.textContent.includes("Read archive"));
  await page.keyboard.press("Escape");
  await until(() => !document.querySelector("dialog[open]"));

  // Enter right after typing: the suggestion debounce must not cancel the search.
  await page.evaluate(() => { window.delays.search = 600; });
  await page.click(".search-archives > summary");
  await until(() => document.querySelector('input[aria-label="Include Installed archive in search"]'));
  await page.click('input[aria-label="Include Installed archive in search"]');
  await page.click('input[aria-label="Search articles"]');
  await page.keyboard.type("animl");
  await page.keyboard.press("Enter");
  await until(() => [...document.querySelectorAll(".search-archives p")].some((node) => node.textContent === "No matching articles found."));
  const afterSearch = await page.evaluate(() => {
    const first = window.calls.findIndex((call) => call.command === "search");
    return window.calls.slice(first + 1).map((call) => call.command).filter((command) => ["suggest", "cancel_search"].includes(command));
  });
  assert.deepEqual(afterSearch, []);
  assert.equal(await page.evaluate(() => document.querySelector(".global-error, dialog[role=alertdialog][open]")?.textContent ?? null), null);
  assert.equal((await calls("search"))[0].args.query, "animl");
  // A spelling correction for a search with no results.
  await button(".search-archives", "Search instead for “animal”");
  await until(() => window.calls.filter((call) => call.command === "search").length === 2);
  assert.equal((await calls("search"))[1].args.query, "animal");
  assert.equal(await page.$eval('input[aria-label="Search articles"]', (node) => node.value), "animal");
  assert.deepEqual(pageErrors, []);
});

test("Downloads shows kept progress, retained originals and one owner per failure", { skip, timeout: 240000 }, async (t) => {
  const { page, pageErrors, until, button, pane, calls } = await open(t);
  const original = { id: "original", title: "Old edition", name: "old_eng", installationId: "old-installation",
    path: "/fixture/old_eng_2025-01.zim", date: "2025-01-01", tags: [], sourceReference: null };
  const transaction = { id: "transaction-1", intentId: "intent", original, revision: 1,
    replacement: { id: "installed", installationId: "fixture-installation", title: "Installed archive", date: "2026-01-01" },
    rolledBackInstallationId: null, rollbackBook: null, originalDiscarded: false };
  await page.evaluate((value) => {
    window.results.downloads = [{
      id: "interrupted", fileName: "big.zim", status: "paused", transport: "http", downloaded: 32e6,
      total: 40e6, error: "Download interrupted by application shutdown; resume when ready",
      destination: "/fixture/downloads/big.zim", createdAt: "2026-01-01T00:00:00Z",
    }];
    window.results.map_status = {
      id: "map-job", regionId: "area-00", language: "en", status: "cancelled", written: 5, total: 20,
      incoming: 0, incomingTotal: 0, speed: 0, resumable: false, canRemoveFromList: false, error: null, path: null,
    };
    window.results.edition_status = { intents: [], transactions: [value] };
    window.failures.map_delete_download = "Kiwix could not remove a partial map download, so it was kept at /fixture/maps/x.part. Fixture reason";
  }, transaction);
  await pane("Downloads", "Downloads");
  // A paused row with an error still says how much Resume continues from.
  await until(() => [...document.querySelectorAll('.download-card[aria-label="big.zim"] .download-live-bytes')]
    .some((node) => /32.*of.*40/.test(node.textContent)));
  assert.equal(await page.$eval('.download-card[aria-label="big.zim"] progress', (node) => node.value / node.max), 0.8);

  // A failing Delete inside the cleanup dialog is painted there only.
  await button(".map-job .section-heading", "Delete download");
  await until(() => document.querySelector("dialog[open] .download-cleanup-actions"));
  // A trusted click: the dialog ignores synthetic activation.
  await button("dialog[open] .download-cleanup-actions", "Delete download and files");
  await until((text) => document.querySelector("dialog[open] [role=alert]")?.textContent.includes(text), "Fixture reason");
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.deepEqual(await page.evaluate(painted, "Fixture reason"), { copies: 1, alertDialog: false });
  await button("dialog[open]", "Cancel");
  await until(() => !document.querySelector("dialog[open]"));

  // Deleting a retained original asks first, names the file and closes rollback.
  await page.evaluate((value) => {
    window.results.edition_discard_original = () => {
      const discarded = { ...value, originalDiscarded: true, revision: 2 };
      window.results.edition_status = { intents: [], transactions: [discarded] };
      return { transaction: discarded, originalFile: "deleted" };
    };
  }, transaction);
  await button('section[aria-label="Retained original editions"]', "Delete retained original");
  await until(() => document.querySelector("dialog.modal-confirmation[open]"));
  assert.match(await page.$eval("dialog.modal-confirmation[open]", (node) => node.textContent),
    /Permanently delete the retained original “Old edition” \(old_eng_2025-01\.zim\)/);
  await page.keyboard.press("Escape");
  await until(() => !document.querySelector("dialog[open]"));
  assert.equal((await calls("edition_discard_original")).length, 0);
  await button('section[aria-label="Retained original editions"]', "Delete retained original");
  await button("dialog.modal-confirmation[open]", "Delete retained original");
  await until(() => document.activeElement?.id === "edition-original-transaction-1");
  assert.deepEqual((await calls("edition_discard_original")).map((call) => call.args),
    [{ transactionId: "transaction-1", privacyGeneration: 1 }]);
  const panel = await page.$eval('section[aria-label="Retained original editions"]', (node) => ({
    text: node.textContent, buttons: [...node.querySelectorAll("button")].map((button) => button.textContent.trim()),
    status: node.querySelector("[role=status]")?.textContent,
  }));
  assert.deepEqual(panel.buttons, []);
  assert.equal(panel.status, "The retained original was deleted. This edition can no longer be rolled back.");
  assert.match(panel.text, /no longer kept, so this edition cannot be rolled back/);
  assert.deepEqual(pageErrors, []);
});
