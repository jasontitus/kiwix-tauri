import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

// Execute the complete production bridge and consume its public browser APIs.
// The fake shell transports authenticated messages; it cannot grant a sensor by
// changing the browser provider's deliberately denied Permissions-Policy.
const bridge = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const shellOrigin = "http://shell.localhost:8123";
function reader(t, { browserState = "denied", permissionsAvailable = true } = {}) {
  const listeners = new Map();
  const messages = [];
  const browserQueries = [];
  const browserStatus = { state: browserState };
  const parent = {
    location: { origin: shellOrigin },
    postMessage(message, origin) {
      assert.equal(origin, shellOrigin);
      messages.push(message);
    },
  };
  const navigator = permissionsAvailable ? {
    permissions: {
      async query(descriptor) {
        browserQueries.push(descriptor);
        return browserStatus;
      },
    },
  } : {};
  const document = {
    currentScript: { dataset: { parentOrigin: shellOrigin } },
    readyState: "loading",
    documentElement: { nodeType: 1, style: {}, querySelectorAll: () => [] },
    querySelectorAll: () => [],
    addEventListener() {},
  };
  const window = {
    parent,
    navigator,
    addEventListener(name, callback) {
      const callbacks = listeners.get(name) ?? [];
      callbacks.push(callback);
      listeners.set(name, callbacks);
    },
  };
  const context = vm.createContext({
    window, parent, navigator, document,
    location: { origin: "http://archive.localhost:8123", href: "http://archive.localhost:8123/index.html" },
    history: { pushState() {}, replaceState() {} },
    MutationObserver: class { observe() {} disconnect() {} },
    EventTarget, Event, WeakRef, DOMException, URL,
    MouseEvent: class extends Event { get ctrlKey() { return false; } },
    WheelEvent: class extends Event { get deltaY() { return 0; } },
    crypto: { randomUUID },
    HTMLAnchorElement: class { get href() { return this.url; } },
    HTMLAreaElement: class { get href() { return this.url; } },
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    setTimeout, clearTimeout, queueMicrotask,
    requestAnimationFrame: (callback) => setTimeout(callback, 0),
    cancelAnimationFrame: clearTimeout,
  });
  vm.runInContext(bridge, context, { filename: "reader-bridge.js" });
  t.after(() => {
    for (const callback of listeners.get("pagehide") ?? []) callback();
  });
  let sessionId = "first-session";
  let sensorStatus = { location: "notAsked", camera: "notAsked", nativeLocation: true };
  function receive(data, { source = parent, origin = shellOrigin } = {}) {
    for (const callback of listeners.get("message") ?? []) callback({
      source, origin, isTrusted: true,
      data: { channel: "kiwix-reader", version: 1, sessionId, ...data },
    });
  }
  return {
    navigator, messages, browserQueries, browserStatus, receive,
    initialize(status = {}, nextSession = sessionId) {
      sessionId = nextSession;
      sensorStatus = { ...sensorStatus, ...status };
      receive({ type: "reader-init", sensorStatus });
    },
    position() {
      let request;
      const result = new Promise((resolve, reject) => {
        navigator.geolocation.getCurrentPosition(resolve, reject, { timeout: 100 });
      });
      request = messages.findLast((message) => message.type === "sensor-request");
      assert.ok(request, "enabled consumer must reach archive consent/native provider");
      return {
        result,
        reply(data, status = {}) {
          sensorStatus = { ...sensorStatus, ...status };
          receive({ type: "sensor-response", requestId: request.requestId, sensorStatus, ...data });
        },
      };
    },
  };
}
const fix = { coords: { latitude: 37.4, longitude: -122.1, accuracy: 12 }, timestamp: 1000 };
const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

 test("native map permission probe remains usable before consent without inheriting browser denial", async (t) => {
  const frame = reader(t);
  const probe = frame.navigator.permissions.query({ name: "geolocation" });
  let resolved = false;
  probe.then(() => { resolved = true; });
  await Promise.resolve();
  assert.equal(resolved, false, "early map probes wait for trusted provider selection");
  frame.initialize();
  const permission = await probe;
  assert.equal(permission.state, "prompt");
  assert.equal(frame.browserQueries.length, 0);
  // Map consumers disable location only for denied, then ask via geolocation.
  assert.notEqual(permission.state, "denied");
  const position = frame.position();
  position.reply({ allowed: true, position: fix }, { location: "granted" });
  assert.deepEqual(await position.result, fix);
  assert.equal(permission.state, "granted");
});

test("remembered archive consent alone does not claim OS permission or a fresh fix", async (t) => {
  const frame = reader(t);
  frame.initialize({ location: "granted" });
  const permission = await frame.navigator.permissions.query({ name: "geolocation" });
  assert.equal(permission.state, "prompt");
  for (const code of [2, 3]) {
    const position = frame.position();
    position.reply({ allowed: false, error: { code, message: "No fresh fix" } });
    await assert.rejects(position.result, (error) => error.code === code);
    assert.equal(permission.state, "prompt");
  }
  const position = frame.position();
  position.reply({ allowed: true, position: fix });
  await position.result;
  const unavailable = frame.position();
  unavailable.reply({ allowed: false, error: { code: 2, message: "Provider unavailable" } });
  await assert.rejects(unavailable.result, (error) => error.code === 2);
  assert.equal(permission.state, "granted", "provider availability is not permission");
});

test("OS denial and explicit archive block/revoke are denied; reset and replacement return to prompt", async (t) => {
  const frame = reader(t);
  frame.initialize({ location: "granted" });
  const permission = await frame.navigator.permissions.query({ name: "geolocation" });
  const position = frame.position();
  position.reply({ allowed: false, error: { code: 1, message: "OS denied access" } });
  await assert.rejects(position.result, (error) => error.code === 1);
  assert.equal(permission.state, "denied");
  frame.initialize({ location: "granted" }, "reopened-session");
  assert.equal(permission.state, "prompt", "old OS observation is not new-session authority");
  frame.receive({ type: "sensor-revoke", sensor: "location", decision: "denied" });
  assert.equal(permission.state, "denied");
  frame.receive({ type: "sensor-revoke", sensor: "location", decision: "notAsked" });
  assert.equal(permission.state, "prompt");
  frame.initialize({ location: "denied" }, "blocked-reopen");
  assert.equal(permission.state, "denied", "remembered archive denial survives reopening");
});

test("permission change consumers receive independent handlers and cannot forge authority", async (t) => {
  const frame = reader(t);
  frame.initialize({ location: "granted" });
  const first = await frame.navigator.permissions.query({ name: "geolocation" });
  const second = await frame.navigator.permissions.query({ name: "geolocation" });
  assert.notEqual(first, second);
  const changed = [];
  first.onchange = () => changed.push(["first", first.state]);
  second.addEventListener("change", () => changed.push(["second", second.state]));
  frame.receive({ type: "sensor-revoke", sensor: "location", decision: "denied" }, { source: {} });
  frame.receive({ type: "sensor-revoke", sensor: "location", decision: "denied" }, { origin: "http://other.localhost:8123" });
  frame.receive({ type: "sensor-revoke", sensor: "location", decision: "denied", sessionId: "stale" });
  assert.equal(first.state, "prompt");
  const position = frame.position();
  position.reply({ allowed: true, position: fix });
  await position.result;
  await nextTask();
  assert.deepEqual(changed, [["first", "granted"], ["second", "granted"]]);
});

test("lifecycle cancellation and late fixes cannot become permission decisions", async (t) => {
  const frame = reader(t);
  frame.initialize({ location: "granted" });
  const permission = await frame.navigator.permissions.query({ name: "geolocation" });
  const acquired = frame.position();
  acquired.reply({ allowed: true, position: fix });
  await acquired.result;
  assert.equal(permission.state, "granted");
  const pending = frame.position();
  frame.receive({ type: "sensor-revoke", sensor: "location" });
  await assert.rejects(pending.result, (error) => error.code === 1);
  assert.equal(permission.state, "prompt", "synthetic cancellation is not OS denial");
  pending.reply({ allowed: true, position: fix });
  assert.equal(permission.state, "prompt", "retired response cannot restore observed authority");
  const watch = frame.navigator.geolocation.watchPosition(
    () => assert.fail("cancelled watch delivered a fix"),
    () => assert.fail("clearWatch should not notify its removed consumer"),
  );
  const request = frame.messages.findLast((message) => message.type === "sensor-request");
  frame.navigator.geolocation.clearWatch(watch);
  frame.receive({ type: "sensor-response", requestId: request.requestId,
    sensorStatus: { location: "granted", nativeLocation: true },
    allowed: true, position: fix });
  await Promise.resolve();
  assert.equal(permission.state, "prompt");
});

test("native retirement before shell background is cancellation, not denial, and terminates its watch", async (t) => {
  const frame = reader(t);
  frame.initialize({ location: "granted" });
  const permission = await frame.navigator.permissions.query({ name: "geolocation" });
  const acquired = frame.position();
  acquired.reply({ allowed: true, position: fix });
  await acquired.result;
  assert.equal(permission.state, "granted");
  const pending = frame.position();
  pending.reply({ allowed: false, cancelled: true,
    error: { code: 2, message: "Location request was cancelled" } });
  await assert.rejects(pending.result, (error) => error.code === 2 && error.cancelled === true);
  assert.equal(permission.state, "prompt");
  let failures = 0;
  frame.navigator.geolocation.watchPosition(
    () => assert.fail("retired watch delivered a fix"),
    (error) => {
      failures++;
      assert.equal(error.code, 2);
      assert.equal(error.cancelled, true);
    },
  );
  const request = frame.messages.findLast((message) => message.type === "sensor-request");
  frame.receive({ type: "sensor-response", requestId: request.requestId,
    sensorStatus: { location: "granted", nativeLocation: true },
    allowed: false, cancelled: true,
    error: { code: 2, message: "Location request was cancelled" } });
  const requests = frame.messages.filter((message) => message.type === "sensor-request").length;
  await new Promise((resolve) => setTimeout(resolve, 1100));
  assert.equal(failures, 1);
  assert.equal(frame.messages.filter((message) => message.type === "sensor-request").length, requests,
    "retired native watch must not reacquire while shell lifecycle notification is delayed");
  assert.equal(permission.state, "prompt");
  const denied = frame.position();
  denied.reply({ allowed: false, cancelled: true,
    error: { code: 1, message: "System denied location access" } });
  await assert.rejects(denied.result, (error) => error.code === 1 && error.cancelled !== true);
  assert.equal(permission.state, "denied", "a real denial must never become cancellation");
});

test("a platform without the app's provider reports unavailable once and does not keep polling", async (t) => {
  // Windows, Linux and Android: the shell answers at once, without consent.
  const frame = reader(t);
  frame.initialize({ nativeLocation: false });
  const unavailable = { allowed: false, cancelled: true,
    error: { code: 2, message: "Location is unavailable on this platform" } };
  const once = frame.position();
  once.reply(unavailable, { nativeLocation: false });
  await assert.rejects(once.result, (error) => error.code === 2,
    "POSITION_UNAVAILABLE, not PERMISSION_DENIED");
  let failures = 0;
  frame.navigator.geolocation.watchPosition(
    () => assert.fail("no provider can deliver a fix"),
    (error) => { failures++; assert.equal(error.code, 2); },
  );
  const request = frame.messages.findLast((message) => message.type === "sensor-request");
  frame.receive({ type: "sensor-response", requestId: request.requestId,
    sensorStatus: { location: "notAsked", nativeLocation: false }, ...unavailable });
  const requests = frame.messages.filter((message) => message.type === "sensor-request").length;
  await new Promise((resolve) => setTimeout(resolve, 1100));
  assert.equal(failures, 1, "the page hears about it once");
  assert.equal(frame.messages.filter((message) => message.type === "sensor-request").length, requests,
    "the watch ends instead of asking every second");
});

test("browser provider and unrelated permissions retain the genuine Permissions API", async (t) => {
  const frame = reader(t, { browserState: "prompt" });
  const descriptor = { name: "camera" };
  assert.equal(await frame.navigator.permissions.query(descriptor), frame.browserStatus);
  assert.equal(frame.browserQueries[0], descriptor);
  frame.initialize({ nativeLocation: false });
  assert.equal(await frame.navigator.permissions.query({ name: "geolocation" }), frame.browserStatus);
  assert.equal(frame.browserQueries.length, 2);
});

test("native provider exposes a truthful probe even when WebKit exposes no Permissions API", async (t) => {
  const frame = reader(t, { permissionsAvailable: false });
  frame.initialize();
  assert.equal((await frame.navigator.permissions.query({ name: "geolocation" })).state, "prompt");
  await assert.rejects(frame.navigator.permissions.query({ name: "camera" }), /not supported/);
});
