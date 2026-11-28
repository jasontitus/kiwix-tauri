import assert from "node:assert/strict";
import test from "node:test";
import puppeteer from "puppeteer-core";
import { createServer } from "vite";

// The sharing, Nearby and device-pairing surfaces against the development
// bundle. The browser owns a disposable profile; every service call is answered
// by the fixture below, which the test changes the way the device would.
const skip = !process.env.KIWIX_TEST_CHROME && !process.env.CI && "set KIWIX_TEST_CHROME to a Chromium executable";

async function settings(page) {
  const handle = await page.waitForFunction(() => [...document.querySelectorAll("aside.sidebar button")]
    .find((node) => node.getAttribute("aria-label")?.startsWith("Settings") || node.textContent.trim() === "Settings"));
  await handle.asElement().click();
  await handle.dispose();
}

async function open(t, initial = {}) {
  const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: true }, logLevel: "silent" });
  await server.listen();
  t.after(() => server.close());
  const browser = await puppeteer.launch({ executablePath: process.env.KIWIX_TEST_CHROME, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
  await page.setViewport({ width: 1280, height: 900 });
  // Device pairing is foreground-only; a headless page is otherwise unfocused.
  const session = await page.createCDPSession();
  await session.send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await page.setRequestInterception(true);
  page.on("request", (request) => {
    if (new URL(request.url()).origin === origin || request.url().startsWith("data:")) void request.continue();
    else void request.abort();
  });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.evaluateOnNewDocument((initial) => {
    const filters = { languages: [], categories: [], content: [] };
    const stopped = { running: false, scope: "selected", url: null, urls: [], port: null, binding: null, bindings: [],
      books: [], token: null, bookIds: [], catalogUrl: null, catalogUrls: [], libraryCount: 0 };
    const privateSync = { identity: "fixture-identity", enabled: true, categories: { bookmarks: true, positions: true },
      session: null, peers: [], conflicts: [], positions: [], pending: 0 };
    // Everything the test changes lives here.
    window.fixture = {
      sharing: initial.sharing ?? stopped,
      localswarm: { discovery: false, peers: [], seeds: [], error: null,
        nearby: { supported: false, nativeReady: true, discovery: false, consumer: null, publisher: null, error: null } },
      privateSync,
      browse: null,
    };
    const snapshot = () => ({
      activeSurface: "library", activeTab: null,
      books: [{ id: "installed", title: "Example archive", description: "", language: "eng", creator: "Fixture",
        publisher: "Fixture", date: "2026-01-01", name: "installed_eng", tags: [], size: 1024, articleCount: 1,
        mediaCount: 0, hasFulltextIndex: false, hasTitleIndex: true, mainPath: "Example",
        installationId: "fixture-installation", path: "/fixture/installed.zim", addedAt: "2026-01-01",
        available: true, managed: true }],
      bookmarks: [], history: [], recentSearches: [], tabs: [], launchLocations: [], referenceTransitions: [],
      privacyGeneration: 1, privacyPolicyGeneration: 1, referenceRevision: 0,
      notifications: { available: false }, sharing: window.fixture.sharing,
      settings: {
        restoreSession: true, locale: "en", theme: "system", privateMode: false, readOnly: false,
        catalogUrl: "https://catalog.fixture.invalid/root.xml", opdsSources: [], catalogFiltersCustomized: true,
        catalogFilters: filters, libraryFilters: filters, monitorDirectories: [], pinnedBookIds: [], bookZoom: {},
        sharingName: "", sharingBookIds: [], speechPreferences: {}, readerTheme: "original", fontSize: 18, zoom: 1,
        downloadDirectory: "/fixture/downloads",
      },
    });
    const original = window.fetch.bind(window);
    window.calls = [];
    window.fetch = async (url, options) => {
      const target = new URL(url, location.href);
      if (target.origin !== location.origin) throw new Error("Unexpected external request");
      if (target.pathname !== "/api") return original(url, options);
      const { command, args } = JSON.parse(options.body);
      window.calls.push({ command, args });
      const fixture = window.fixture;
      const handlers = {
        snapshot,
        sharing_status: () => fixture.sharing,
        sharing_interfaces: () => [],
        localswarm_status: () => fixture.localswarm,
        localswarm_discovery: () => fixture.localswarm,
        downloads: () => [],
        edition_status: () => ({ intents: [], transactions: [] }),
        search_session: () => ({ sessionId: "fixture-session" }),
        catalog: () => ({ entries: [], next: null, total: 0 }),
        catalog_facets: () => ({ languages: [], categories: [] }),
        map_status: () => null,
        private_sync_status: () => fixture.privateSync,
        private_sync_pair_start: () => {
          fixture.privateSync = { ...fixture.privateSync, session: { id: `session-${window.calls.length}`,
            invitation: "kiwix-private:fixture-invitation", localConfirmed: false, remoteConfirmed: false,
            expiresAt: new Date(Date.now() + 600000).toISOString(), phase: "waiting" } };
          return fixture.privateSync;
        },
        private_sync_pair_cancel: () => {
          fixture.privateSync = { ...fixture.privateSync, session: null };
          return fixture.privateSync;
        },
        // A Bluetooth sender is joined before its list arrives; the test
        // decides when, if ever, that finishes.
        shared_library_browse: () => new Promise((resolve) => { fixture.browse = resolve; }),
      };
      return Response.json({ result: handlers[command] ? await handlers[command]() : null });
    };
  }, initial);
  await page.goto(origin + "/#token=fixture", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.__KIWIX_BOOT_STATE__?.ready);
  const text = (selector, value) => page.waitForFunction((sel, expected) =>
    [...document.querySelectorAll(sel)].some((node) => node.textContent.includes(expected)), {}, selector, value);
  const absent = async (selector, value) => assert.equal(await page.evaluate((sel, expected) =>
    [...document.querySelectorAll(sel)].some((node) => node.textContent.includes(expected)), selector, value), false,
    `"${value}" must not be shown`);
  const button = async (scope, label) => {
    const handle = await page.waitForFunction((sel, value) => [...document.querySelectorAll(`${sel} button`)]
      .find((node) => !node.disabled && node.textContent.trim() === value), {}, scope, label);
    await handle.asElement().click();
    await handle.dispose();
  };
  const disclose = async (label) => {
    const handle = await page.waitForFunction((value) => [...document.querySelectorAll("details > summary")]
      .find((node) => node.textContent.trim().startsWith(value)), {}, label);
    // Some sections open themselves, for example while sharing is on.
    if (!(await handle.evaluate((node) => node.parentElement.open))) await handle.asElement().click();
    await handle.dispose();
  };
  const calls = (command) => page.evaluate((name) => window.calls.filter((call) => call.command === name), command);
  const called = (command, count = 1) => page.waitForFunction((name, expected) =>
    window.calls.filter((call) => call.command === name).length >= expected, {}, command, count);
  const change = (update) => page.evaluate(`(${update})(window.fixture)`);
  return { page, pageErrors, text, absent, button, disclose, calls, called, change };
}

test("web sharing shows the addresses the device has now, not the ones at start", { skip, timeout: 90000 }, async (t) => {
  const sharing = { running: true, scope: "library", sessionId: "session", locked: false,
    url: "http://192.168.1.50:8080/nearby/session", urls: ["http://192.168.1.50:8080/nearby/session"], port: 8080,
    binding: { mode: "interface", address: "192.168.1.50" }, bindings: ["192.168.1.50:8080"], books: [],
    token: "secret", bookIds: [], catalogUrl: "http://192.168.1.50:8080/secret/catalog/v2/root.xml",
    catalogUrls: ["http://192.168.1.50:8080/secret/catalog/v2/root.xml"], libraryCount: 1, addressLost: false };
  const { page, pageErrors, text, absent, disclose, called, change } = await open(t, { sharing });
  const lost = "This device is no longer connected at the address this share uses";
  await settings(page);
  await disclose("Web sharing (browsers and OPDS)");
  await text(".sharing-controls .badge", "Sharing is on");
  await disclose("Manual connection details");
  await text(".sharing-address code", "http://192.168.1.50:8080/nearby/session");
  await called("sharing_status");
  await absent(".sharing-controls [role=alert]", lost);

  // The laptop changes Wi-Fi: the bound address is gone.
  await change((fixture) => { fixture.sharing = { ...fixture.sharing, addressLost: true }; });
  await text(".sharing-controls [role=alert]", lost);
  await text(".sharing-controls [role=alert]", "stop web sharing and enable it again");
  // It comes back (same lease): the warning goes away by itself.
  await change((fixture) => { fixture.sharing = { ...fixture.sharing, addressLost: false }; });
  await page.waitForFunction((value) => ![...document.querySelectorAll(".sharing-controls [role=alert]")]
    .some((node) => node.textContent.includes(value)), {}, lost);

  // A share on all interfaces follows the device's new address.
  await change((fixture) => {
    fixture.sharing = { ...fixture.sharing, binding: { mode: "ipv4All" }, addressLost: undefined,
      url: "http://10.0.0.7:8080/nearby/session", urls: ["http://10.0.0.7:8080/nearby/session", "http://127.0.0.1:8080/nearby/session"],
      catalogUrl: "http://10.0.0.7:8080/secret/catalog/v2/root.xml" };
  });
  await text(".sharing-address code", "http://10.0.0.7:8080/nearby/session");
  await text(".sharing-address code", "http://10.0.0.7:8080/secret/catalog/v2/root.xml");
  await absent(".sharing-address code", "192.168.1.50");
  assert.deepEqual(await page.$$eval(".sharing-qr select option", (nodes) => nodes.map((node) => node.value)),
    ["http://10.0.0.7:8080/nearby/session", "http://127.0.0.1:8080/nearby/session"]);

  // The device loses every network: the share still runs, for this device only.
  const local = "only this device can open the links below";
  await absent(".sharing-controls [role=alert]", local);
  await change((fixture) => {
    fixture.sharing = { ...fixture.sharing, localOnly: true, url: "http://127.0.0.1:8080/nearby/session",
      urls: ["http://127.0.0.1:8080/nearby/session"], catalogUrl: "http://127.0.0.1:8080/secret/catalog/v2/root.xml" };
  });
  await text(".sharing-controls [role=alert]", local);
  await absent(".sharing-controls [role=alert]", lost);
  await change((fixture) => { fixture.sharing = { ...fixture.sharing, localOnly: false }; });
  await page.waitForFunction((value) => ![...document.querySelectorAll(".sharing-controls [role=alert]")]
    .some((node) => node.textContent.includes(value)), {}, local);

  // One poll chain: hiding and showing the page repeatedly must not multiply the requests.
  const started = await page.evaluate(() => window.calls.filter((call) => call.command === "sharing_status").length);
  for (let n = 0; n < 6; n++) await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
  await new Promise((resolve) => setTimeout(resolve, 4600));
  const polls = (await page.evaluate(() => window.calls.filter((call) => call.command === "sharing_status").length)) - started;
  assert.ok(polls >= 1 && polls <= 3, `expected one poll chain, saw ${polls} requests in one interval`);
  assert.deepEqual(pageErrors, []);
});

test("Nearby says when Bluetooth or sharing stopped, and closing a pending join abandons it", { skip, timeout: 90000 }, async (t) => {
  const { page, pageErrors, text, absent, button, calls, called, change } = await open(t);
  const nearby = { supported: true, nativeReady: true, discovery: true,
    consumer: { owner: "finder-owner", peerId: null, phase: "discovering", error: null }, publisher: null, error: null };
  const sender = { id: "bluetooth-sender", peerId: "bluetooth-sender", transport: "quic-library", discovery: "bluetooth",
    publisherName: "Sender phone" };
  await change(`(fixture) => { fixture.localswarm = { discovery: true, peers: [], seeds: [], error: null,
    librarySharing: { running: true, sessionId: "s", publisherName: "This phone", libraryCount: 1, uploaded: 0, peerCount: 0, error: null },
    nearby: { ...${JSON.stringify(nearby)}, discovery: false,
      publisher: { owner: "share-owner", peerId: null, phase: "failed", error: "BLE_POWERED_OFF" }, error: "BLE_POWERED_OFF" } }; }`);
  await page.click('aside.sidebar button[aria-label^="Nearby"]');
  await text("main h1", "Nearby");

  // Bluetooth is off: both halves say so, and say what still works.
  await text(".localswarm-share-card [role=alert]", "Devices on other Wi-Fi networks cannot find this share over Bluetooth");
  await text(".localswarm-share-card [role=alert]", "stop sharing and choose Share all");
  await text(".localswarm-receive [role=alert]", "Finding devices over Bluetooth stopped");
  const before = (await calls("localswarm_discovery")).length;
  await button(".localswarm-receive", "Try Bluetooth again");
  await called("localswarm_discovery", before + 1);
  assert.deepEqual((await calls("localswarm_discovery")).at(-1).args.enabled, true);

  // "Share all" ended by itself: the card says why and what to do.
  const reason = "Nearby sharing stopped because an archive in your library changed or the connection ended. Choose Share all to start again.";
  await change(`(fixture) => { fixture.localswarm = { discovery: true, peers: [${JSON.stringify(sender)}], seeds: [], error: null,
    librarySharing: { running: false, libraryCount: 0, uploaded: 0, peerCount: 0, error: ${JSON.stringify(reason)} },
    nearby: ${JSON.stringify(nearby)} }; }`);
  await text(".localswarm-share-card [role=alert]", reason);
  await text(".localswarm-share-card h2", "Share your library");
  await absent(".localswarm [role=alert]", "over Bluetooth");

  // Choose the Bluetooth sender, then close while it is still being joined.
  await button(".localswarm-peer", "Choose");
  await called("shared_library_browse");
  await text("dialog[open] [role=status]", "Connecting to the sender");
  assert.equal((await calls("nearby_stop")).length, 0);
  const finding = (await calls("localswarm_discovery")).length;
  await button("dialog[open] .modal-actions", "Close");
  await called("nearby_stop");
  assert.deepEqual((await calls("nearby_stop"))[0].args.owner, "finder-owner");
  assert.deepEqual((await calls("nearby_stop"))[0].args.pendingOnly, true);
  // Abandoning the connection must not leave Bluetooth finding off.
  await called("localswarm_discovery", finding + 1);
  assert.deepEqual((await calls("localswarm_discovery")).at(-1).args.enabled, true);
  await page.waitForFunction(() => !document.querySelector("dialog[open]"));

  // A list that loaded keeps its connection when the dialog closes.
  await page.evaluate(() => { window.fixture.browse = null; });
  await button(".localswarm-peer", "Choose");
  await page.waitForFunction(() => typeof window.fixture.browse === "function");
  await page.evaluate(() => window.fixture.browse({ receiptId: "receipt", sessionId: "s", name: "Sender phone",
    books: [{ id: "offered", title: "Offered archive", fileName: "offered.zim", size: 10, downloadIdentity: "kiwix-library://public/s/p/offered" }] }));
  await text("dialog[open]", "Offered archive");
  await button("dialog[open] .modal-actions", "Close");
  await page.waitForFunction(() => !document.querySelector("dialog[open]"));
  await new Promise((resolve) => setTimeout(resolve, 500));
  assert.equal((await calls("nearby_stop")).length, 1);
  assert.deepEqual(pageErrors, []);
});

test("a pairing invitation that ends by itself is explained; cancelling it is not", { skip, timeout: 90000 }, async (t) => {
  const { page, pageErrors, text, absent, button, disclose, called, change } = await open(t);
  const ended = "The invitation is no longer active. Create a new invitation to connect your other device.";
  await settings(page);
  await disclose("Connect my devices");
  await button(".private-reading-sync", "Create private invitation");
  await text(".private-reading-sync [role=status]", "Waiting for your other device");
  await absent(".private-reading-sync [role=alert]", ended);

  // The device retires the session (a phone does when the app is hidden).
  const polled = (await page.evaluate(() => window.calls.filter((call) => call.command === "private_sync_status").length));
  await change((fixture) => { fixture.privateSync = { ...fixture.privateSync, session: null }; });
  await called("private_sync_status", polled + 1);
  await text(".private-reading-sync [role=alert]", ended);
  await text(".private-reading-sync button", "Create private invitation");

  // The person's own Cancel is not an unexplained loss.
  await button(".private-reading-sync", "Create private invitation");
  await text(".private-reading-sync [role=status]", "Waiting for your other device");
  await absent(".private-reading-sync [role=alert]", ended);
  await button(".private-reading-sync", "Cancel connection");
  await text(".private-reading-sync button", "Create private invitation");
  const after = (await page.evaluate(() => window.calls.filter((call) => call.command === "private_sync_status").length));
  await called("private_sync_status", after + 2);
  await absent(".private-reading-sync [role=alert]", ended);
  assert.deepEqual(pageErrors, []);
});
