import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/reader-storage.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function storage() {
  const listeners = new Map();
  const origin = "http://shell.localhost:8123";
  const window = {
    addEventListener(name, callback) {
      const callbacks = listeners.get(name) ?? [];
      callbacks.push(callback);
      listeners.set(name, callbacks);
    },
    dispatchEvent(event) {
      for (const callback of listeners.get(event.type) ?? []) callback(event);
    },
    postMessage(data) {
      queueMicrotask(() => {
        for (const callback of listeners.get("message") ?? [])
          callback({ data, origin, source: window });
      });
    },
  };
  const exports = {};
  const context = vm.createContext({
    exports, window, location: { origin }, URL, TextEncoder, crypto,
    AbortController, DOMException, CustomEvent, setTimeout, clearTimeout,
    require(name) {
      if (name === "./i18n") return { t: (text) => text };
      if (name === "./api") return {
        api: () => { throw new Error("This admission scenario must not issue a native write"); },
        registerPendingPreferenceDrain() {},
        registerPendingPreferenceCutoverDrain() {},
      };
      throw new Error(`Unexpected dependency ${name}`);
    },
  });
  vm.runInContext(compiled, context, { filename: "reader-storage.ts" });
  return exports;
}

const turn = () => new Promise((resolve) => setImmediate(resolve));

test("navigation waits for genuine in-flight document retirement", async () => {
  const root = storage();
  const retiring = root.retireReaderDocuments();
  assert.equal(root.canNavigateReaderStorage(), false);
  let admitted = false;
  const next = root.waitForReaderStorageNavigation(new AbortController().signal)
    .then(() => { admitted = true; });
  assert.equal(admitted, false);
  await retiring;
  await next;
  assert.equal(admitted, true);
  assert.equal(root.canNavigateReaderStorage(), true);
});

test("overlapping freezes retain admission until the last owner resumes", async () => {
  const root = storage();
  const first = await root.freezeReaderStorage();
  const second = await root.freezeReaderStorage();
  let admitted = false;
  const next = root.waitForReaderStorageNavigation(new AbortController().signal)
    .then(() => { admitted = true; });
  first();
  await turn();
  assert.equal(admitted, false);
  assert.equal(root.canNavigateReaderStorage(), false);
  second();
  await next;
  assert.equal(admitted, true);
});

test("cancelled navigation cannot cancel another owner's checkpoint", async () => {
  const root = storage();
  const resume = await root.freezeReaderStorage();
  const abort = new AbortController();
  const next = root.waitForReaderStorageNavigation(abort.signal);
  abort.abort();
  await assert.rejects(next, { name: "AbortError" });
  assert.equal(root.canNavigateReaderStorage(), false);
  resume();
  await root.waitForReaderStorageNavigation(new AbortController().signal);
});

test("a retired freeze cannot release a replacement generation's freeze", async () => {
  const root = storage();
  const oldResume = await root.freezeReaderStorage();
  const next = root.waitForReaderStorageNavigation(new AbortController().signal);
  root.retireReaderStorageGeneration(1);
  await next;
  const newResume = await root.freezeReaderStorage();
  oldResume();
  assert.equal(root.canNavigateReaderStorage(), false);
  newResume();
  assert.equal(root.canNavigateReaderStorage(), true);
});

test("unproved security detach rejects waiting navigation rather than bypassing storage", async () => {
  const root = storage();
  const producer = { postMessage() {} };
  assert.equal(typeof root.registerReaderStorage(producer, {
    origin: "http://archive.localhost:9000", bookId: "book", privacyGeneration: 0,
    retirementUrl: `http://archive.localhost:9000/.kiwix/${"a".repeat(32)}/retire.html`,
    retirementProof: "b".repeat(32),
  }), "function");
  root.quarantineReaderStorage(producer);
  await assert.rejects(root.waitForReaderStorageNavigation(new AbortController().signal),
    /could not confirm that the article finished saving/);
  root.retireReaderStorageGeneration(1);
  assert.equal(root.canNavigateReaderStorage(), false);
});
