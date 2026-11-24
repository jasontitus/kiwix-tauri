import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../src/storage-bridge.js", import.meta.url), "utf8");

// A tiny synchronous Storage plus a fake parent window is enough to exercise
// the real bridge: quotas, coalesced production, and BFCache credit recovery.
function bridge() {
  const values = new Map();
  class MemoryStorage {
    get length() {
      return values.size;
    }
    key(index) {
      return [...values.keys()][index] ?? null;
    }
    getItem(key) {
      return values.has(String(key)) ? values.get(String(key)) : null;
    }
    setItem(key, value) {
      let total = 0;
      for (const [stored, storedValue] of values)
        total += stored.length + storedValue.length;
      if (total + String(key).length + String(value).length > 2 * 1024 * 1024)
        throw new DOMException("full", "QuotaExceededError");
      values.set(String(key), String(value));
    }
    removeItem(key) {
      values.delete(String(key));
    }
  }
  const nativeStorage = new MemoryStorage();
  const listeners = new Map();
  const parentOrigin = "http://shell.localhost";
  const origin = "http://archive.localhost:9000";
  const messages = [];
  const errors = [];
  let failPosts = false;
  const parent = {
    postMessage(data, target) {
      if (failPosts) throw new Error("parent is unavailable");
      messages.push({ data, target });
    },
  };
  const listen = (name, callback) => {
    const callbacks = listeners.get(name) ?? [];
    callbacks.push(callback);
    listeners.set(name, callbacks);
  };
  listen("kiwix-reader-storage-error", (event) => errors.push(event));
  const dispatch = (event) => {
    for (const callback of listeners.get(event.type) ?? []) callback(event);
  };
  const window = {
    parent: null,
    localStorage: nativeStorage,
    addEventListener: listen,
    dispatchEvent: dispatch,
  };
  window.parent = window;
  const document = {};
  const context = vm.createContext({
    window, document, parent, listeners, location: { origin, hostname: "archive.localhost", pathname: "/book/A/Page" },
    Storage: MemoryStorage, Event, EventTarget, CustomEvent, DOMException, TextEncoder, crypto,
    queueMicrotask, addEventListener: listen, dispatchEvent: dispatch,
    Symbol, Object, Map, Set, JSON, Date, Number, String, Array, RegExp,
  });
  window.__kiwixStorageBootstrap = {
    parentOrigin, readOnly: false, privacyGeneration: 0,
    values: { localStorage: {}, cookies: [] },
  };
  vm.runInContext(source, context, { filename: "storage-bridge.js" });
  return {
    storage: context.window.localStorage,
    messages,
    errors,
    batches: () => messages.filter(({ data }) => data.type === "kiwix-storage-batch"),
    failPosts(value) {
      failPosts = value;
    },
    deliver(data) {
      dispatch({ type: "message", source: parent, origin: parentOrigin, data });
    },
    pageshow(persisted) {
      dispatch({ type: "pageshow", persisted });
    },
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("synchronous writes coalesce into one bounded IPC batch", async () => {
  const { storage, batches } = bridge();
  storage.setItem("first", "1");
  storage.setItem("second", "2");
  assert.equal(batches().length, 0, "delivery is deferred to a microtask");
  await settle();
  const batchesAfter = batches();
  assert.equal(batchesAfter.length, 1);
  assert.equal(batchesAfter[0].data.batch.sequence, 1);
  assert.deepEqual(
    Array.from(batchesAfter[0].data.batch.operations, (operation) => operation.key),
    ["first", "second"],
  );
  storage.setItem("third", "3");
  await settle();
  assert.equal(batches()[1].data.batch.sequence, 2);
});

test("quota failures stay synchronous and preserve already-admitted writes", async () => {
  const { storage, batches } = bridge();
  storage.setItem("small", "value");
  // The compatibility quota is checked before native memory is touched.
  assert.throws(
    () => storage.setItem("huge", "x".repeat(1024 * 1024)),
    (error) => error.name === "QuotaExceededError",
  );
  assert.equal(storage.getItem("huge"), null);
  await settle();
  const [batch] = batches();
  assert.deepEqual(Array.from(batch.data.batch.operations, (operation) => operation.key), ["small"]);
});

test("a BFCache pageshow keeps credit for synchronous writes and reconciles the snapshot", async () => {
  const { storage, messages, deliver, pageshow, batches } = bridge();
  const documentId = messages.find(({ data }) => data.type === "kiwix-storage-ready").data.documentId;
  pageshow(true);
  const restored = messages.filter(({ data }) => data.type === "kiwix-storage-ready").at(-1).data;
  assert.equal(restored.documentId, documentId);
  assert.equal(restored.sequence, 0);
  // The retained allowance lets a synchronous pageshow write succeed.
  storage.setItem("during-restore", "1");
  await settle();
  const [first] = batches();
  assert.equal(first.data.batch.sequence, 1);
  // The shell replied to ready before it saw this batch, so the snapshot must
  // stay reduced by the post-ready payload.
  deliver({ documentId, type: "kiwix-storage-credit", bytes: 2 * 1024 * 1024, sequence: 0 });
  storage.setItem("after-credit", "2");
  await settle();
  const delivered = batches();
  assert.deepEqual(
    delivered.map(({ data }) => data.batch.sequence),
    [1, 2],
  );
  assert.deepEqual(
    Array.from(delivered[1].data.batch.operations, (operation) => operation.key),
    ["after-credit"],
  );
  assert.equal(messages.some(({ data }) => data.type === "kiwix-storage-failed"), false);
});

test("a deferred post failure restores the mirror and surfaces the error", async () => {
  const { storage, errors, failPosts } = bridge();
  storage.setItem("key", "old");
  await settle();
  failPosts(true);
  storage.setItem("key", "new");
  storage.setItem("other", "value");
  // The writes were accepted synchronously and only delivery failed later.
  await settle();
  assert.equal(storage.getItem("key"), "old");
  assert.equal(storage.getItem("other"), null);
  assert.equal(errors.length, 1);
  assert.throws(
    () => storage.setItem("later", "value"),
    (error) => error.name === "InvalidStateError",
  );
});
