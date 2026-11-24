import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/reader-storage.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function storage(options = {}) {
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
    queueMicrotask,
    require(name) {
      if (name === "./i18n") return { t: (text) => text };
      if (name === "./api") return {
        api: options.api ?? (() => { throw new Error("This admission scenario must not issue a native write"); }),
        registerPendingPreferenceDrain() {},
        registerPendingPreferenceCutoverDrain() {},
      };
      throw new Error(`Unexpected dependency ${name}`);
    },
  });
  vm.runInContext(compiled, context, { filename: "reader-storage.ts" });
  exports.window = window;
  exports.origin = origin;
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

const archiveOrigin = "http://archive.localhost:9000";
const retirementUrl = `${archiveOrigin}/.kiwix/${"a".repeat(32)}/retire.html`;
const binding = () => ({
  origin: archiveOrigin, bookId: "book", privacyGeneration: 0,
  retirementUrl, retirementProof: "b".repeat(32),
});
const documentId = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const batchId = (n) => `11111111-1111-4111-8111-${String(n).padStart(12, "0")}`;
function frame() {
  const messages = [];
  return { messages, source: { postMessage(data) { messages.push(data); } } };
}
function ready(root, producer, id, sequence = 0) {
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: producer.source,
    data: { type: "kiwix-storage-ready", documentId: id, sequence, privacyGeneration: 0 },
  });
}
const batchBytes = (batch) => Buffer.byteLength(JSON.stringify(batch));

test("idle static frames spend no shared credit and never block later frames", async () => {
  const root = storage({ api: async () => ({ status: "committed" }) });
  const frames = [];
  // The eager 2 MiB reservation admitted only eight frames from the 16 MiB
  // root budget; an idle frame must now cost nothing.
  for (let index = 0; index < 24; index++) {
    const producer = frame();
    assert.equal(typeof root.registerReaderStorage(producer.source, binding()), "function",
      `static frame ${index} is admitted`);
    frames.push(producer);
  }
  frames.forEach((producer, index) => ready(root, producer, documentId(index)));
  for (const producer of frames) {
    const credit = producer.messages.filter((message) => message.type === "kiwix-storage-credit");
    assert.equal(credit.length, 1);
    assert.equal(credit[0].bytes, 2 * 1024 * 1024);
    assert.equal(producer.messages.some((message) => message.type === "kiwix-storage-failed"), false);
  }
  // The twenty-fourth frame still has its full allowance and can persist.
  const last = frames.at(-1);
  last.messages.length = 0;
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: last.source,
    data: {
      type: "kiwix-storage-batch", privacyGeneration: 0,
      batch: {
        id: batchId(99), documentId: documentId(23), sequence: 1,
        operations: [{ kind: "local-set", key: "key", value: "value" }],
      },
    },
  });
  await turn();
  await turn();
  assert.ok(last.messages.some((message) => message.type === "kiwix-storage-ack" && message.sequence === 1));
  assert.equal(root.canNavigateReaderStorage(), true);
});

test("root capacity backpressure retries an accepted write instead of losing it", async () => {
  let open = false;
  const waiters = [];
  const root = storage({
    api: () => open
      ? Promise.resolve({ status: "committed" })
      : new Promise((resolve) => waiters.push(resolve)),
  });
  const writers = [];
  for (let index = 0; index < 17; index++) {
    const producer = frame();
    assert.equal(typeof root.registerReaderStorage(producer.source, binding()), "function");
    ready(root, producer, documentId(index));
    writers.push(producer);
  }
  // More than 16 MiB of accepted batches; the excess waits for capacity.
  for (let index = 0; index < writers.length; index++) {
    root.window.dispatchEvent({
      type: "message", origin: archiveOrigin, source: writers[index].source,
      data: {
        type: "kiwix-storage-batch", privacyGeneration: 0,
        batch: {
          id: batchId(index), documentId: documentId(index), sequence: 1,
          operations: [{ kind: "local-set", key: "key", value: "x".repeat(1024 * 1024) }],
        },
      },
    });
  }
  const failed = () => writers.some((producer) =>
    producer.messages.some((message) => message.type === "kiwix-storage-failed"));
  assert.equal(failed(), false, "capacity pressure must not fail an accepted write");
  open = true;
  for (const waiter of waiters) waiter({ status: "committed" });
  for (let attempt = 0; attempt < 50; attempt++) {
    if (writers.every((producer) =>
      producer.messages.some((message) => message.type === "kiwix-storage-ack"))) break;
    await turn();
  }
  writers.forEach((producer, index) => {
    assert.ok(producer.messages.some((message) =>
      message.type === "kiwix-storage-ack" && message.sequence === 1),
    `writer ${index} eventually commits`);
  });
  assert.equal(failed(), false);
});

test("a resumed BFCache document regains its own credit with its queued batch charged", async () => {
  let releaseWrite;
  const writeGate = new Promise((resolve) => { releaseWrite = resolve; });
  const root = storage({ api: () => writeGate.then(() => ({ status: "committed" })) });
  const producer = frame();
  assert.equal(typeof root.registerReaderStorage(producer.source, binding()), "function");
  ready(root, producer, documentId(1));
  const batch = {
    id: batchId(1), documentId: documentId(1), sequence: 1,
    operations: [{ kind: "local-set", key: "key", value: "value" }],
  };
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: producer.source,
    data: { type: "kiwix-storage-batch", privacyGeneration: 0, batch },
  });
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: producer.source,
    data: { type: "kiwix-storage-closed", documentId: documentId(1), sequence: 1 },
  });
  producer.messages.length = 0;
  // BFCache restore replays ready with the persisted document identity.
  ready(root, producer, documentId(1), 1);
  const credit = producer.messages.find((message) => message.type === "kiwix-storage-credit");
  assert.ok(credit, "the restored document receives a credit snapshot");
  assert.equal(credit.bytes, 2 * 1024 * 1024 - batchBytes(batch));
  assert.equal(credit.sequence, 0);
  assert.equal(producer.messages.some((message) => message.type === "kiwix-storage-failed"), false);
  releaseWrite();
  await turn();
  await turn();
  const ack = producer.messages.find((message) => message.type === "kiwix-storage-ack");
  assert.ok(ack, "the queued pre-BFCache batch still commits");
  assert.equal(ack.sequence, 1);
});

test("a BFCache resume without a close keeps the outstanding batch charged", async () => {
  let releaseWrite;
  const writeGate = new Promise((resolve) => { releaseWrite = resolve; });
  const root = storage({ api: () => writeGate.then(() => ({ status: "committed" })) });
  const producer = frame();
  assert.equal(typeof root.registerReaderStorage(producer.source, binding()), "function");
  ready(root, producer, documentId(2));
  const batch = {
    id: batchId(2), documentId: documentId(2), sequence: 1,
    operations: [{ kind: "local-set", key: "key", value: "value" }],
  };
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: producer.source,
    data: { type: "kiwix-storage-batch", privacyGeneration: 0, batch },
  });
  producer.messages.length = 0;
  // A plain pagehide is advisory; the same document restores with sequence 1.
  ready(root, producer, documentId(2), 1);
  const credit = producer.messages.find((message) => message.type === "kiwix-storage-credit");
  assert.ok(credit);
  assert.equal(credit.bytes, 2 * 1024 * 1024 - batchBytes(batch));
  assert.equal(credit.sequence, 0);
  assert.equal(producer.messages.some((message) => message.type === "kiwix-storage-failed"), false);
  releaseWrite();
  await turn();
  await turn();
  assert.ok(producer.messages.some((message) => message.type === "kiwix-storage-ack" && message.sequence === 1));
});

test("a removed frame's idle stream is released on registration dispose", async () => {
  let writes = 0;
  const root = storage({ api: async () => { writes += 1; return { status: "committed" }; } });
  const producer = frame();
  const frameNode = {
    isConnected: true,
    contentWindow: producer.source,
    addEventListener() {},
    removeEventListener() {},
  };
  const dispose = root.registerReaderStorage(producer.source, binding(), frameNode);
  assert.equal(typeof dispose, "function");
  ready(root, producer, documentId(3));
  frameNode.isConnected = false;
  dispose();
  await turn();
  root.window.dispatchEvent({
    type: "message", origin: archiveOrigin, source: producer.source,
    data: {
      type: "kiwix-storage-batch", privacyGeneration: 0,
      batch: {
        id: batchId(3), documentId: documentId(3), sequence: 1,
        operations: [{ kind: "local-set", key: "key", value: "value" }],
      },
    },
  });
  await turn();
  assert.equal(writes, 0, "the released stream cannot accept further mutations");
  assert.equal(root.canNavigateReaderStorage(), true);
});

test("generation retirement fails blocked retries instead of committing them", async () => {
  let open = false;
  const waiters = [];
  const root = storage({
    api: () => open
      ? Promise.resolve({ status: "committed" })
      : new Promise((resolve) => waiters.push(resolve)),
  });
  const writers = [];
  for (let index = 0; index < 17; index++) {
    const producer = frame();
    assert.equal(typeof root.registerReaderStorage(producer.source, binding()), "function");
    ready(root, producer, documentId(index));
    writers.push(producer);
  }
  // More than the root budget; at least one accepted batch waits for capacity.
  for (let index = 0; index < writers.length; index++) {
    root.window.dispatchEvent({
      type: "message", origin: archiveOrigin, source: writers[index].source,
      data: {
        type: "kiwix-storage-batch", privacyGeneration: 0,
        batch: {
          id: batchId(index), documentId: documentId(index), sequence: 1,
          operations: [{ kind: "local-set", key: "key", value: "x".repeat(1024 * 1024) }],
        },
      },
    });
  }
  root.retireReaderStorageGeneration(1);
  writers.forEach((producer, index) => {
    assert.ok(producer.messages.some((message) => message.type === "kiwix-storage-failed"),
      `writer ${index} is retired`);
  });
  open = true;
  for (const waiter of waiters) waiter({ status: "committed" });
  for (let attempt = 0; attempt < 20; attempt++) await turn();
  assert.equal(writers.some((producer) =>
    producer.messages.some((message) => message.type === "kiwix-storage-ack")), false,
  "a retired blocked batch must not commit later");
});
