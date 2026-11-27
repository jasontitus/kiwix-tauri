import assert from "node:assert/strict";
import test from "node:test";
import puppeteer from "puppeteer-core";
import { createServer } from "vite";

// Real-DOM checks of the shell's top layer, run against the development bundle
// so React StrictMode replays every effect. Same harness as
// test-frontend-surfaces.mjs: a disposable browser profile and a service
// fixture; nothing leaves the test origin.
const skip = !process.env.KIWIX_TEST_CHROME && !process.env.CI && "set KIWIX_TEST_CHROME to a Chromium executable";

// Installed before any page script. `native` simulates the Tauri bridge the
// way @tauri-apps/api/mocks does (IPC, events, plugin channels) so the mobile
// shell runs in the browser; without it the page uses the /api preview.
function fixture(native) {
  const filters = { languages: [], categories: [], content: [] };
  const metadata = {
    id: "fixture-archive", title: "Example archive", description: "An offline archive", language: "eng",
    creator: "Fixture", publisher: "Fixture", date: "2026-01-01", name: "example_eng",
    tags: ["_category:wikipedia", "Reference"],
    size: 1024, articleCount: 1, mediaCount: 0, hasFulltextIndex: false, hasTitleIndex: true, mainPath: "Example",
  };
  const snapshot = {
    activeSurface: "library", activeTab: null,
    books: [{ ...metadata, id: "installed", name: "installed_eng", installationId: "fixture-installation",
      path: "/fixture/installed.zim", addedAt: "2026-01-01", available: true, managed: true }],
    bookmarks: [{ id: "bookmark", bookId: "installed", path: "Example", title: "Saved example", note: "",
      tags: ["one"], highlights: [], createdAt: "2026-01-01" }],
    history: [], recentSearches: [], tabs: [], launchLocations: [], referenceTransitions: [],
    privacyGeneration: 1, privacyPolicyGeneration: 1, referenceRevision: 0,
    notifications: { available: false }, sharing: { running: false, books: [], bookIds: [] },
    settings: {
      restoreSession: true, locale: "en", theme: "system", privateMode: false, readOnly: false,
      catalogUrl: "https://catalog.fixture.invalid/entries", opdsSources: [], catalogFiltersCustomized: true,
      catalogFilters: filters, libraryFilters: filters, monitorDirectories: [], pinnedBookIds: [], bookZoom: {},
      sharingName: "", sharingBookIds: [], speechPreferences: {}, readerTheme: "original", fontSize: 18, zoom: 1,
      downloadDirectory: "/fixture/downloads",
    },
  };
  const results = {
    snapshot,
    catalog: { entries: [{ ...metadata, url: "https://archive.fixture.invalid/example.zim", thumbnailUrl: null }], next: null, total: 1 },
    catalog_facets: { languages: [{ value: "eng", label: "English", count: 1 }], categories: [] },
    downloads: [], edition_status: { intents: [], transactions: [] }, search_session: { sessionId: "fixture-session" },
    mobile_lifecycle_state: { active: true, interactive: true, backgroundRequestId: null, retirementRevision: 0, incoming: [], entries: [] },
  };
  window.calls = [];
  // Tests change these to make a command fail.
  window.failures = { save_bookmark: "Fixture write refused" };
  const dispatch = (command, args) => {
    window.calls.push({ command, args });
    if (window.failures[command]) throw new Error(window.failures[command]);
    return results[command] ?? null;
  };
  if (!native) {
    const original = window.fetch.bind(window);
    window.fetch = async (url, options) => {
      const target = new URL(url, location.href);
      if (target.origin !== location.origin) throw new Error("Unexpected external request");
      if (target.pathname !== "/api") return original(url, options);
      const { command, args } = JSON.parse(options.body);
      try {
        return Response.json({ result: dispatch(command, args) });
      } catch (error) {
        return Response.json({ error: error.message }, { status: 400 });
      }
    };
    return;
  }
  const callbacks = new Map();
  const events = new Map();
  // What the Android AppPlugin holds: the channels registered for "back-button".
  const backChannels = [];
  window.backLog = [];
  window.isTauri = true;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => callbacks.delete(id) };
  window.__TAURI_INTERNALS__ = {
    callbacks,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    convertFileSrc: (path, protocol = "asset") => `${protocol}://localhost/${encodeURIComponent(path)}`,
    transformCallback(callback) {
      const id = crypto.getRandomValues(new Uint32Array(1))[0];
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, data) => callbacks.get(id)?.(data),
    async invoke(command, args) {
      if (command === "plugin:event|listen") {
        if (!events.has(args.event)) events.set(args.event, []);
        events.get(args.event).push(args.handler);
        return args.handler;
      }
      if (command === "plugin:event|unlisten") {
        const handlers = events.get(args.event) ?? [];
        if (handlers.includes(args.eventId)) handlers.splice(handlers.indexOf(args.eventId), 1);
        return null;
      }
      if (command === "plugin:app|register_listener") {
        if (args.event === "back-button") backChannels.push({ id: args.handler.id, index: 0 });
        window.backLog.push("register");
        return null;
      }
      if (command === "plugin:app|remove_listener") {
        const index = backChannels.findIndex((channel) => channel.id === args.channelId);
        if (index >= 0) backChannels.splice(index, 1);
        window.backLog.push(index >= 0 ? "remove" : `remove-unknown:${args.channelId}`);
        return null;
      }
      if (command === "dispatch") return dispatch(args.command, args.args);
      return null;
    },
  };
  window.emitNative = (event, payload) => {
    for (const handler of events.get(event) ?? []) callbacks.get(handler)?.({ event, id: handler, payload });
  };
  // AppPlugin.handleOnBackPressed: deliver to the page while a listener is
  // registered; otherwise the platform default takes Back (leaves the app).
  window.pressBack = () => {
    if (!backChannels.length) return "system";
    for (const channel of [...backChannels])
      callbacks.get(channel.id)?.({ message: { canGoBack: false }, index: channel.index++ });
    return "page";
  };
  window.backListeners = () => backChannels.length;
}

async function open(t, { native = false, userAgent, viewport = { width: 1280, height: 900 }, prepare } = {}) {
  assert.ok(process.env.KIWIX_TEST_CHROME, "CI must set KIWIX_TEST_CHROME to a Chromium executable; this browser test may not be skipped");
  const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: true }, logLevel: "silent" });
  await server.listen();
  t.after(() => server.close());
  const browser = await puppeteer.launch({ executablePath: process.env.KIWIX_TEST_CHROME, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
  if (userAgent) await page.setUserAgent(userAgent);
  await page.setViewport(viewport);
  // Native key commands are only honoured by a focused document.
  const session = await page.createCDPSession();
  await session.send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await page.setRequestInterception(true);
  page.on("request", (request) => {
    if (new URL(request.url()).origin === origin || request.url().startsWith("data:")) void request.continue();
    else void request.abort();
  });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  if (prepare) await page.evaluateOnNewDocument(prepare);
  await page.evaluateOnNewDocument(fixture, native);
  await page.goto(origin + "/#token=fixture", { waitUntil: "domcontentloaded" });
  try {
    await page.waitForFunction(() => window.__KIWIX_BOOT_STATE__?.ready, { timeout: 20000 });
  } catch {
    // Say where startup stopped instead of only "waiting failed".
    const boot = await page.evaluate(() => ({ state: window.__KIWIX_BOOT_STATE__, calls: window.calls?.map((call) => call.command) }));
    assert.fail(`The shell did not start: ${JSON.stringify(boot)} ${JSON.stringify(pageErrors)}`);
  }
  const until = (check, ...args) => page.waitForFunction(check, { timeout: 10000 }, ...args);
  const button = async (scope, text) => {
    const handle = await until((sel, value) => [...document.querySelectorAll(`${sel} button`)]
      .find((node) => !node.disabled && node.getClientRects().length && node.textContent.trim() === value), scope, text);
    await handle.asElement().click();
    await handle.dispose();
  };
  const pane = async (label, title) => {
    const handle = await until((value) => [...document.querySelectorAll(".sidebar button")]
      .find((node) => node.getClientRects().length && node.getAttribute("aria-label")?.startsWith(value)), label);
    await handle.asElement().click();
    await handle.dispose();
    await until((value) => document.querySelector("main h1")?.textContent.includes(value), title);
  };
  return { page, pageErrors, until, button, pane };
}

// Everything about the alert that a user or assistive technology depends on.
const alertState = () => {
  const alert = document.querySelector("dialog[role=alertdialog][open]");
  const text = (id) => id ? document.getElementById(id)?.textContent.trim() ?? null : null;
  return {
    open: !!alert,
    count: document.querySelectorAll("dialog[role=alertdialog][open]").length,
    name: text(alert?.getAttribute("aria-labelledby")),
    description: text(alert?.getAttribute("aria-describedby")),
    modal: !!alert?.matches(":modal"),
    // The backdrop of the top-most modal covers every point of the viewport.
    topMost: !!alert && document.elementFromPoint(1, 1)?.closest("dialog[open]") === alert,
    focus: document.activeElement?.textContent.trim() ?? "",
    focusInside: !!alert && alert.contains(document.activeElement),
    band: document.querySelectorAll(".global-error").length,
    // A live region inside the dialog would be read a second time.
    liveRegions: alert?.querySelectorAll("[role=alert], [role=status], [aria-live]").length ?? 0,
    openDialogs: document.querySelectorAll("dialog[open]").length,
  };
};
const FAILED = "The operation could not be completed.";

test("a failure inside a modal is shown, announced and dismissed above it, with one owner", { skip, timeout: 90000 }, async (t) => {
  const { page, pageErrors, until, button, pane } = await open(t);
  const report = (message) => page.evaluate((text) => {
    window.dispatchEvent(new CustomEvent("kiwix-desktop-workflow-error", { detail: { error: new Error(text) } }));
  }, message);

  // No dialog open: the band owns the failure, as before.
  await report("Background failure");
  await until(() => document.querySelector(".global-error [role=alert]")?.textContent.includes("Background failure"));
  assert.equal((await page.evaluate(alertState)).open, false);
  await page.click('.global-error button[aria-label="Dismiss error"]');
  await until(() => !document.querySelector(".global-error"));

  // A save started inside the bookmark editor is refused.
  await pane("Reading list", "Reading list");
  await button(".reading-card", "Edit note and tags");
  const note = "dialog[open] textarea";
  await page.waitForSelector(note);
  await page.type(note, "Draft that must survive");
  await button("dialog[open]", "Save changes");
  await until(() => !!document.querySelector("dialog[role=alertdialog][open]"));
  await until(() => document.activeElement?.textContent.trim() === "Dismiss");
  let state = await page.evaluate(alertState);
  assert.deepEqual(state, {
    open: true, count: 1, name: FAILED, description: "Fixture write refused", modal: true, topMost: true,
    focus: "Dismiss", focusInside: true, band: 0, liveRegions: 0, openDialogs: 2,
  });
  assert.equal(await page.$$eval("*", (nodes) => nodes.filter((node) =>
    [...node.childNodes].some((child) => child.nodeType === 3 && child.textContent.includes("Fixture write refused"))).length), 1,
  "the failure text is painted exactly once");
  // The accessibility tree exposes it as an alert dialog with its message; an
  // inert subtree (the band under a modal) would not be exposed at all.
  const tree = await page.accessibility.snapshot({ interestingOnly: false });
  const exposed = [];
  (function visit(node) {
    if (node.role === "alertdialog") exposed.push(node);
    for (const child of node.children ?? []) visit(child);
  })(tree);
  assert.equal(exposed.length, 1);
  assert.equal(exposed[0].name, FAILED);
  assert.equal(exposed[0].description, "Fixture write refused");
  // The editor underneath keeps its draft and stays open.
  assert.equal(await page.$eval(note, (node) => node.value), "Draft that must survive");

  // Dismissing returns focus to the control that started the action.
  await page.keyboard.press("Enter");
  await until(() => !document.querySelector("dialog[role=alertdialog][open]"));
  state = await page.evaluate(alertState);
  assert.equal(state.band, 0, "dismissed in the top layer means dismissed, not moved to the band");
  assert.equal(state.openDialogs, 1);
  await until(() => document.activeElement?.textContent.trim() === "Save changes");
  assert.equal(await page.$eval(note, (node) => node.value), "Draft that must survive");

  // Escape dismisses only the alert; the editor needs its own Escape.
  await page.keyboard.press("Enter");
  await until(() => !!document.querySelector("dialog[role=alertdialog][open]"));
  // A second failure while the alert is open replaces its message: still one alert.
  await report("A newer failure");
  await until(() => document.querySelector("dialog[role=alertdialog][open] p")?.textContent === "A newer failure");
  state = await page.evaluate(alertState);
  assert.equal(state.count, 1);
  assert.equal(state.band, 0);
  assert.equal(state.openDialogs, 2);
  await page.keyboard.press("Escape");
  await until(() => !document.querySelector("dialog[role=alertdialog][open]"));
  assert.equal(await page.$$eval("dialog[open]", (nodes) => nodes.length), 1);
  assert.equal(await page.$eval(note, (node) => node.value), "Draft that must survive");
  // The retry works once the write is accepted, and the editor then closes.
  await page.evaluate(() => { delete window.failures.save_bookmark; });
  await until(() => document.activeElement?.textContent.trim() === "Save changes");
  await page.keyboard.press("Enter");
  await until(() => !document.querySelector("dialog[open]"));
  assert.equal((await page.evaluate(() => window.calls.filter((call) => call.command === "save_bookmark").at(-1)))
    .args.bookmark.note, "Draft that must survive");

  // Nested dialogs: a confirmation over the catalog detail, then a failure.
  await pane("Discover", "Discover");
  await button(".catalog-card-content", "Example archive");
  await button("dialog[open]", "Download archive");
  await page.waitForSelector("dialog.modal-confirmation[open] button:focus");
  assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), "Cancel");
  await report("Failure over two dialogs");
  await until(() => document.activeElement?.textContent.trim() === "Dismiss");
  state = await page.evaluate(alertState);
  assert.deepEqual({ ...state, focus: undefined }, {
    open: true, count: 1, name: FAILED, description: "Failure over two dialogs", modal: true, topMost: true,
    focus: undefined, focusInside: true, band: 0, liveRegions: 0, openDialogs: 3,
  });
  await button("dialog[role=alertdialog][open]", "Dismiss");
  await until(() => !document.querySelector("dialog[role=alertdialog][open]"));
  // Focus is back on the confirmation's own default, and both dialogs are intact.
  await until(() => document.activeElement?.textContent.trim() === "Cancel" &&
    !!document.activeElement.closest("dialog.modal-confirmation[open]"));
  assert.equal(await page.$$eval("dialog[open]", (nodes) => nodes.length), 2);
  assert.equal((await page.evaluate(() => window.calls.filter((call) => call.command === "start_download").length)), 0);
  await page.keyboard.press("Escape");
  await until(() => document.activeElement?.textContent.trim() === "Download archive");
  assert.deepEqual(pageErrors, []);
});

test("desktop-only Settings copy is not shown where there are no windows or File menu", { skip, timeout: 60000 }, async (t) => {
  // The browser preview is not a desktop window; it gets the same copy as phones.
  const { page, pageErrors, until } = await open(t);
  await page.click(".sidebar-settings");
  await until(() => document.querySelector("main h1")?.textContent.includes("Settings"));
  await until(() => document.body.textContent.includes("Restore open tabs when the app starts"));
  const text = await page.evaluate(() => document.body.textContent);
  assert.match(text, /Reader tabs and reading positions reopen at launch\. Private and read-only profiles are never restored\./);
  assert.doesNotMatch(text, /File → Restore previous session|Windows, reader tabs/);
  assert.doesNotMatch(text, /Shift \+ N/);
  assert.match(text, /⌘ \/ Ctrl \+ T/, "the shortcuts that do work are still listed");
  assert.deepEqual(pageErrors, []);
});

const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const ANDROID = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36";
const escapeKey = (page) => page.evaluate(() => window.emitNative("kiwix-mobile-shortcut", { action: "escape" }));

test("a hardware Escape consumed by the native layer closes shell popovers outside the reader", { skip, timeout: 90000 }, async (t) => {
  const { page, pageErrors, until, button } = await open(t, { native: true, userAgent: IPHONE, viewport: { width: 820, height: 1100 } });
  assert.equal(await page.evaluate(() => document.querySelector(".app-shell").classList.contains("is-mobile")), true);

  // A facet picker on Discover.
  await page.evaluate(() => [...document.querySelectorAll(".main-nav button")].find((node) => node.getAttribute("aria-label")?.startsWith("Discover")).click());
  await until(() => document.querySelector("main h1")?.textContent.includes("Discover"));
  const summary = await until(() => document.querySelector("details.facet-picker > summary"));
  await summary.asElement().click();
  await until(() => !!document.querySelector("details.facet-picker[open]"));
  await escapeKey(page);
  await until(() => !document.querySelector("details.facet-picker[open]"));
  assert.equal(await page.evaluate(() => document.activeElement?.closest("details.facet-picker") !== null &&
    document.activeElement.tagName), "SUMMARY", "focus returns to the picker's own control");
  // With nothing open, Escape outside the reader changes nothing.
  await escapeKey(page);
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(await page.evaluate(() => document.querySelector("main h1")?.textContent.includes("Discover")), true);

  // A dialog keeps its own cancel policy and closes before anything under it.
  await button(".catalog-card-content", "Example archive");
  await page.waitForSelector("dialog[open]");
  await escapeKey(page);
  await until(() => !document.querySelector("dialog[open]"));
  assert.deepEqual(pageErrors, []);
});

test("the native Escape closes the phone's tab sheet wherever focus is", { skip, timeout: 90000 }, async (t) => {
  const { page, pageErrors, until } = await open(t, { native: true, userAgent: IPHONE, viewport: { width: 390, height: 800 } });
  await page.click('[data-mobile-tabs-trigger="true"]');
  await until(() => !!document.querySelector("#reader-tabs.mobile-tabs-open"));
  // Focus somewhere else in the shell: the sheet's own key handler cannot see the key.
  await page.evaluate(() => document.getElementById("main-content").focus());
  await escapeKey(page);
  await until(() => !document.querySelector("#reader-tabs.mobile-tabs-open"));
  assert.deepEqual(pageErrors, []);
});

test("Android Back closes the top-most surface, returns to the library, and only then leaves to the system", { skip, timeout: 120000 }, async (t) => {
  const { page, pageErrors, until, button } = await open(t, {
    native: true, userAgent: ANDROID, viewport: { width: 390, height: 800 },
    // A listener that an earlier document of this process left registered.
    prepare: () => { try { localStorage.setItem("kiwix-android-back-listener", "4242"); } catch { /* opaque origin */ } },
  });
  const back = () => page.evaluate(() => window.pressBack());
  const listeners = () => page.evaluate(() => window.backListeners());
  const settled = (count) => until((expected) => window.backListeners() === expected, count);
  const nav = (label) => page.evaluate((value) => [...document.querySelectorAll(".main-nav button, .sidebar-settings")]
    .find((node) => node.getAttribute("aria-label")?.startsWith(value)).click(), label);
  assert.equal(await page.evaluate(() => document.querySelector(".app-shell").classList.contains("is-mobile")), true);

  // At the library with nothing open the shell holds no listener: Back is the
  // platform's own (it backgrounds the task). The stale record was cleared.
  await until(() => window.backLog.includes("remove-unknown:4242"));
  assert.equal(await listeners(), 0);
  assert.equal(await back(), "system");
  assert.equal(await page.evaluate(() => localStorage.getItem("kiwix-android-back-listener")), null);

  // Another pane: Back returns to the library, then belongs to the system again.
  await nav("Settings");
  await until(() => document.querySelector("main h1")?.textContent.includes("Settings"));
  await settled(1);
  assert.equal(await back(), "page");
  await until(() => document.querySelector("main h1")?.textContent.includes("Library"));
  assert.equal(await page.evaluate(() => document.activeElement?.id), "main-content", "focus follows the pane change");
  await settled(0);
  assert.equal(await back(), "system");

  // The reader-tab sheet.
  await page.click('[data-mobile-tabs-trigger="true"]');
  await until(() => !!document.querySelector("#reader-tabs.mobile-tabs-open"));
  await settled(1);
  assert.equal(await back(), "page");
  await until(() => !document.querySelector("#reader-tabs.mobile-tabs-open"));
  await settled(0);

  // A picker, a dialog over it, a confirmation over that: one Back each.
  await nav("Discover");
  await until(() => document.querySelector("main h1")?.textContent.includes("Discover"));
  await settled(1);
  const summary = await until(() => document.querySelector("details.facet-picker > summary"));
  await summary.asElement().click();
  await until(() => !!document.querySelector("details.facet-picker[open]"));
  assert.equal(await back(), "page");
  await until(() => !document.querySelector("details.facet-picker[open]"));
  assert.equal(await page.evaluate(() => document.querySelector("main h1")?.textContent.includes("Discover")), true,
    "closing the picker did not also navigate");
  await button(".catalog-card-content", "Example archive");
  await button("dialog[open]", "Download archive");
  await page.waitForSelector("dialog.modal-confirmation[open]");
  assert.equal(await back(), "page");
  await until(() => document.querySelectorAll("dialog[open]").length === 1);
  assert.equal(await page.evaluate(() => window.calls.filter((call) => call.command === "start_download").length), 0,
    "Back on a confirmation is Cancel");
  assert.equal(await back(), "page");
  await until(() => !document.querySelector("dialog[open]"));
  assert.equal(await page.evaluate(() => document.querySelector("main h1")?.textContent.includes("Discover")), true);
  assert.equal(await back(), "page");
  await until(() => document.querySelector("main h1")?.textContent.includes("Library"));
  await settled(0);
  assert.equal(await back(), "system");

  // A dialog on the library pane itself is covered too.
  await page.click(".workspace-actions .global-search");
  await page.waitForSelector("dialog[open]");
  await settled(1);
  assert.equal(await back(), "page");
  await until(() => !document.querySelector("dialog[open]"));
  await settled(0);
  assert.equal(await back(), "system");
  assert.ok((await page.evaluate(() => window.backLog)).filter((entry) => entry === "register").length >= 4);
  assert.deepEqual(pageErrors, []);
});
