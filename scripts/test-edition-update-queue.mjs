import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(
  new URL("../src/edition-update-queue.ts", import.meta.url),
  "utf8",
);
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exports = {};
vm.runInNewContext(compiled, { exports }, { filename: "edition-update-queue.ts" });
const { queueEditionUpdates, selectBulkEditionUpdates } = exports;

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}
function update(bookId, installationId, overrides = {}) {
  return {
    bookId,
    installationId,
    associationRequired: false,
    bulkEligible: true,
    selection: {
      sourceId: "c".repeat(64),
      sourceUrl: "https://example.org/catalog",
      entryId: `entry-${bookId}`,
      catalogGeneration: 2,
      entry: {
        title: `Title ${bookId}`,
        date: "2026-10-16T00:00:00Z",
        size: 1024,
      },
    },
    ...overrides,
  };
}
function installed(bookId, installationId) {
  return { id: bookId, installationId };
}
function counts(progress) {
  return {
    total: progress.total,
    processed: progress.processed,
    started: progress.started,
    failed: progress.failed,
    skipped: progress.skipped,
    stopped: progress.stopped,
  };
}

test("bulk selection excludes ambiguous, retired, and duplicate installations", () => {
  const updates = [
    update("alpha", "install-alpha"),
    update("beta", "install-beta", { bulkEligible: false, associationRequired: true }),
    update("gamma", "install-gamma"),
    update("delta", "install-delta"),
    update("alpha-again", "install-alpha"),
  ];
  const selected = selectBulkEditionUpdates(updates, [
    installed("alpha", "install-alpha"),
    installed("beta", "install-beta"),
    installed("gamma", "install-gamma"),
    installed("alpha-again", "install-alpha"),
  ]);
  assert.deepEqual(Array.from(selected, (candidate) => candidate.bookId), ["alpha", "gamma"]);
  assert.equal(selected[0].title, "Title alpha");
  assert.equal(selected[0].size, 1024);
  assert.equal(selected[0].selection.entryId, "entry-alpha");
  assert.equal(
    selectBulkEditionUpdates(updates, [installed("alpha", "stale-installation")]).length,
    0,
  );
});

test("queue serializes the prepare-start handoff and starts every edition", async () => {
  const candidates = ["a", "b", "c", "d", "e"].map((id) => ({
    bookId: id,
    installationId: `install-${id}`,
    title: `Title ${id}`,
    size: 10,
    selection: update(id, `install-${id}`).selection,
  }));
  let handoffActive = 0;
  let handoffPeak = 0;
  const started = [];
  const progress = await queueEditionUpdates(candidates, {
    isCurrent: () => true,
    concurrency: 2,
    prepare: async (candidate) => {
      handoffActive++;
      handoffPeak = Math.max(handoffPeak, handoffActive);
      await delay(5);
      handoffActive--;
      return { token: `token-${candidate.bookId}` };
    },
    start: async (token, candidate) => {
      handoffActive++;
      handoffPeak = Math.max(handoffPeak, handoffActive);
      await delay(5);
      handoffActive--;
      started.push(candidate.bookId);
      return { kind: "download", id: token };
    },
    onProgress: () => {},
  });
  // Native preparation retains one decision per window and download admission
  // shares one disk-space confirmation, so prepare and start never overlap.
  assert.equal(handoffPeak, 1);
  assert.deepEqual(counts(progress), {
    total: 5,
    processed: 5,
    started: 5,
    failed: 0,
    skipped: 0,
    stopped: false,
  });
  assert.deepEqual(started.sort(), ["a", "b", "c", "d", "e"]);
});

test("two eligible editions both reach download start after a serialized handoff", async () => {
  const candidates = ["a", "b"].map((id) => ({
    bookId: id,
    installationId: `install-${id}`,
    title: `Title ${id}`,
    size: 10,
    selection: update(id, `install-${id}`).selection,
  }));
  const firstPrepared = deferred();
  const events = [];
  const queued = queueEditionUpdates(candidates, {
    isCurrent: () => true,
    concurrency: 2,
    prepare: async (candidate) => {
      events.push(`prepare:${candidate.bookId}`);
      if (candidate.bookId === "a") await firstPrepared.promise;
      return { token: `token-${candidate.bookId}` };
    },
    start: async (token, candidate) => {
      events.push(`start:${candidate.bookId}`);
      return { kind: "download", id: token };
    },
    onProgress: () => {},
  });
  await delay(1);
  // The second worker is admitted only after the first prepare-start handoff.
  assert.deepEqual(events, ["prepare:a"]);
  firstPrepared.resolve();
  const progress = await queued;
  assert.deepEqual(events, ["prepare:a", "start:a", "prepare:b", "start:b"]);
  assert.deepEqual(counts(progress), {
    total: 2,
    processed: 2,
    started: 2,
    failed: 0,
    skipped: 0,
    stopped: false,
  });
});

test("partial prepare and start failures keep later editions queued", async () => {
  const candidates = ["a", "b", "c", "d", "e"].map((id) => ({
    bookId: id,
    installationId: `install-${id}`,
    title: `Title ${id}`,
    size: 10,
    selection: update(id, `install-${id}`).selection,
  }));
  const started = [];
  const progress = await queueEditionUpdates(candidates, {
    isCurrent: () => true,
    concurrency: 1,
    prepare: async (candidate) => {
      if (candidate.bookId === "b") throw new Error("catalog changed");
      return { token: `token-${candidate.bookId}` };
    },
    start: async (token, candidate) => {
      if (candidate.bookId === "c") throw new Error("disk full");
      started.push(candidate.bookId);
      return { kind: "download", id: token };
    },
    onProgress: () => {},
  });
  assert.deepEqual(started, ["a", "d", "e"]);
  assert.deepEqual(counts(progress), {
    total: 5,
    processed: 5,
    started: 3,
    failed: 2,
    skipped: 0,
    stopped: false,
  });
  assert.deepEqual(
    Array.from(progress.failures, (failure) => [failure.bookId, failure.error.message]),
    [["b", "catalog changed"], ["c", "disk full"]],
  );
});

test("cancelling the queue starts nothing further and accounts for skipped editions", async () => {
  const candidates = ["a", "b", "c", "d"].map((id) => ({
    bookId: id,
    installationId: `install-${id}`,
    title: `Title ${id}`,
    size: 10,
    selection: update(id, `install-${id}`).selection,
  }));
  let current = true;
  const started = [];
  const accepted = [];
  const progress = await queueEditionUpdates(candidates, {
    isCurrent: () => current,
    concurrency: 1,
    prepare: async (candidate) => ({ token: `token-${candidate.bookId}` }),
    start: async (token, candidate) => {
      started.push(candidate.bookId);
      current = false;
      return { kind: "download", id: token };
    },
    onStarted: (candidate, target) => accepted.push([candidate.bookId, target.id]),
    onProgress: () => {},
  });
  assert.deepEqual(started, ["a"]);
  assert.deepEqual(accepted, [["a", "token-a"]]);
  assert.deepEqual(counts(progress), {
    total: 4,
    processed: 4,
    started: 1,
    failed: 0,
    skipped: 3,
    stopped: true,
  });
});

test("cancellation after preparation never starts the prepared transfer", async () => {
  const candidate = {
    bookId: "a",
    installationId: "install-a",
    title: "Title a",
    size: 10,
    selection: update("a", "install-a").selection,
  };
  let current = true;
  const prepared = deferred();
  let starts = 0;
  const queued = queueEditionUpdates([candidate], {
    isCurrent: () => current,
    concurrency: 1,
    prepare: () => prepared.promise,
    start: async (token) => {
      starts++;
      return { kind: "download", id: token };
    },
    onProgress: () => {},
  });
  await delay(1);
  current = false;
  prepared.resolve({ token: "token-a" });
  const progress = await queued;
  assert.equal(starts, 0);
  assert.deepEqual(counts(progress), {
    total: 1,
    processed: 1,
    started: 0,
    failed: 0,
    skipped: 1,
    stopped: true,
  });
});

test("an empty candidate list settles without work", async () => {
  let calls = 0;
  const progress = await queueEditionUpdates([], {
    isCurrent: () => true,
    prepare: async () => {
      calls++;
      return { token: "unused" };
    },
    start: async () => {
      calls++;
      return { kind: "download", id: "unused" };
    },
    onProgress: () => {},
  });
  assert.equal(calls, 0);
  assert.equal(progress.total, 0);
  assert.equal(progress.processed, 0);
});
