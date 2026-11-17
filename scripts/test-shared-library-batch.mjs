import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/shared-library-batch.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exports = {};
vm.runInNewContext(compiled, { exports }, { filename: "shared-library-batch.ts" });
const { queueSharedLibrary } = exports;

const session = "a".repeat(32);
const foreignSession = "b".repeat(32);
const uuid = (suffix) => `00000000-0000-4000-8000-${String(suffix).padStart(12, "0")}`;
function offered(id, archiveId, overrides = {}) {
  return {
    id, title: `Title ${id}`, fileName: `${id}.zim`, size: 1024,
    downloadIdentity: `kiwix-library://${session}/${id}`,
    ...(archiveId ? { archiveId } : {}), ...overrides,
  };
}
function inventory(books) {
  return { receiptId: "receipt", sessionId: session, name: "Nearby library", books };
}
function job(bookId, overrides = {}) {
  return {
    id: `job-${bookId}`, url: `kiwix-library://${session}/${bookId}`,
    fileName: "archive.zim", destination: "/downloads/archive.zim", status: "queued",
    downloaded: 0, total: 1024, error: null, speed: 0, transport: "quic-library",
    ...overrides,
  };
}
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}
function counts(progress) {
  return {
    total: progress.total, processed: progress.processed, queued: progress.queued,
    installed: progress.installed, active: progress.active, unavailable: progress.unavailable,
    duplicates: progress.duplicates, stopped: progress.stopped,
  };
}
function coherent(progress) {
  assert.equal(progress.processed,
    progress.queued + progress.installed + progress.active + progress.unavailable + progress.duplicates);
  assert.ok(progress.processed <= progress.total);
}

// These tests execute the production queue, not a duplicate admission algorithm.
test("mixed inventory classifies installed, unavailable, exact active, duplicate, and retryable rows", async () => {
  const books = [
    offered("installed", uuid(1)), offered("installed-alias", uuid(1)),
    offered("unavailable", uuid(2)), offered("unavailable-alias", uuid(2)),
    offered("active-queued", uuid(3)), offered("active-alias", uuid(3)),
    offered("active-downloading", uuid(4)), offered("active-verifying", uuid(5)),
    offered("foreign", uuid(6)), offered("new", uuid(7)), offered("new-alias", uuid(7)),
    offered("paused", uuid(8)), offered("failed", uuid(9)),
    offered("complete", uuid(10)), offered("connecting", uuid(11)),
  ];
  const receipt = Object.freeze({ ...inventory(books), books: Object.freeze(books.map(Object.freeze)) });
  const installed = Object.freeze([
    Object.freeze({ id: uuid(1), available: true }),
    Object.freeze({ id: uuid(2), available: false }),
  ]);
  const active = Object.freeze([
    job("active-queued"), job("active-downloading", { status: "downloading" }),
    job("active-verifying", { status: "verifying" }),
    job("foreign", { url: `kiwix-library://${foreignSession}/foreign` }),
    job("paused", { status: "paused" }), job("failed", { status: "failed" }),
    job("complete", { status: "complete" }), job("connecting", { status: "connecting" }),
  ].map(Object.freeze));
  const started = [];
  const tracked = [];
  const snapshots = [];
  const result = await queueSharedLibrary(receipt, installed, active, {
    isCurrent: () => true,
    start: async (id) => { started.push(id); return job(id); },
    onQueued: (accepted) => tracked.push(accepted),
    onProgress: (progress) => { coherent(progress); snapshots.push(progress); },
  });
  assert.deepEqual(started, ["foreign", "new", "paused", "failed", "complete", "connecting"]);
  assert.deepEqual(tracked.map((accepted) => accepted.id), [
    "job-active-queued", "job-active-downloading", "job-active-verifying",
    "job-foreign", "job-new", "job-paused", "job-failed", "job-complete", "job-connecting",
  ]);
  assert.equal(tracked[0], active[0], "existing active jobs reach root tracking without fabrication");
  assert.equal(tracked[1], active[1]);
  assert.equal(tracked[2], active[2]);
  assert.deepEqual(counts(result), {
    total: 15, processed: 15, queued: 6, installed: 1, active: 3,
    unavailable: 1, duplicates: 4, stopped: false,
  });
  assert.equal(result.failure, undefined);
  assert.equal(snapshots[0].processed, 0);
  assert.equal(snapshots.at(-1).processed, 15);
});

test("UUID skips require archiveId, not the offered route, title, or filename", async () => {
  const books = [
    offered(uuid(1), undefined, { title: "Same title", fileName: "same.zim" }),
    offered("another-route", undefined, { title: "Same title", fileName: "same.zim" }),
    offered("route-for-new-uuid", uuid(2), { title: "Same title", fileName: "same.zim" }),
  ];
  const started = [];
  const result = await queueSharedLibrary(inventory(books), [{ id: uuid(1), available: true }], [
    job(uuid(2)),
    job("another-route", { url: "https://example.org/another-route" }),
  ], {
    isCurrent: () => true,
    start: async (id) => { started.push(id); return job(id); },
    onQueued() {}, onProgress: coherent,
  });
  assert.deepEqual(started, [uuid(1), "another-route", "route-for-new-uuid"]);
  assert.equal(result.queued, 3);
  assert.equal(result.installed, 0);
  assert.equal(result.active, 0);
  assert.equal(result.duplicates, 0);
});

test("active dedup compares the authoritative inventory identity exactly, including escaped fixtures", async () => {
  // Literal API fixtures: this test does not reimplement backend URL escaping.
  const matching = [
    ["slash/question?#percent% space", `kiwix-library://${session}/slash%2Fquestion%3F%23percent%25%20space`],
    ["literal%2F", `kiwix-library://${session}/literal%252F`],
    ["café/📚", `kiwix-library://${session}/caf%C3%A9%2F%F0%9F%93%9A`],
    ["opaque-book", "backend-authoritative:opaque/identity"],
  ];
  const different = [
    ["wrong-session", `kiwix-library://${session}/wrong-session`, `kiwix-library://${foreignSession}/wrong-session`],
    ["literal/slash", `kiwix-library://${session}/literal%2Fslash`, `kiwix-library://${session}/literal/slash`],
    ["raw%2F", `kiwix-library://${session}/raw%252F`, `kiwix-library://${session}/raw%2F`],
    ["query", `kiwix-library://${session}/query`, `kiwix-library://${session}/query?token=anything`],
    ["empty-query", `kiwix-library://${session}/empty-query`, `kiwix-library://${session}/empty-query?`],
    ["fragment", `kiwix-library://${session}/fragment`, `kiwix-library://${session}/fragment#anything`],
    ["credentials", `kiwix-library://${session}/credentials`, `kiwix-library://user@${session}/credentials`],
    ["port", `kiwix-library://${session}/port`, `kiwix-library://${session}:80/port`],
    ["extra-path", `kiwix-library://${session}/extra-path`, `kiwix-library://${session}/prefix/extra-path`],
    ["overencoded", `kiwix-library://${session}/overencoded`, `kiwix-library://${session}/%6Fverencoded`],
    ["uppercase-scheme", `kiwix-library://${session}/uppercase-scheme`, `KIWIX-LIBRARY://${session}/uppercase-scheme`],
  ];
  const books = [...matching, ...different].map(([id, downloadIdentity], index) =>
    offered(id, uuid(index + 1), { downloadIdentity }));
  const active = [
    ...matching.map(([id, url]) => job(id, { url })),
    ...different.map(([id, , url]) => job(id, { url })),
  ];
  const started = [];
  const tracked = [];
  const result = await queueSharedLibrary(inventory(books), [], active, {
    isCurrent: () => true,
    start: async (id) => { started.push(id); return job(id); },
    onQueued: (accepted) => tracked.push(accepted), onProgress: coherent,
  });
  assert.deepEqual(started, different.map(([id]) => id));
  assert.equal(result.active, matching.length);
  assert.equal(result.queued, different.length);
  matching.forEach((_, index) => assert.equal(tracked[index], active[index]));
});

test("equal book and session never merge plaintext or different TLS pins; an exact secure identity skips active", async () => {
  const plaintext = `kiwix-library://${session}/same-book`;
  const secureA = `kiwix-library://tls-${"1".repeat(64)}/${session}/same-book`;
  const secureB = `kiwix-library://tls-${"2".repeat(64)}/${session}/same-book`;
  for (const [offeredIdentity, activeIdentity, shouldSkip] of [
    [secureA, plaintext, false], [plaintext, secureA, false],
    [secureA, secureB, false], [secureA, secureA, true],
  ]) {
    const active = job("same-book", { url: activeIdentity, status: "downloading" });
    const started = [];
    const tracked = [];
    const accepted = job("same-book", { url: offeredIdentity });
    const result = await queueSharedLibrary(inventory([
      offered("same-book", uuid(1), { downloadIdentity: offeredIdentity }),
    ]), [], [active], {
      isCurrent: () => true,
      start: async (id) => { started.push(id); return accepted; },
      onQueued: (value) => tracked.push(value), onProgress: coherent,
    });
    assert.deepEqual(started, shouldSkip ? [] : ["same-book"]);
    assert.equal(result.active, shouldSkip ? 1 : 0);
    assert.equal(result.queued, shouldSkip ? 0 : 1);
    assert.equal(tracked.length, 1);
    assert.equal(tracked[0], shouldSkip ? active : accepted);
  }
});

test("admission is strictly sequential and each accepted job is tracked before the next starts", async () => {
  const first = deferred();
  const second = deferred();
  const secondStarted = deferred();
  const started = [];
  const tracked = [];
  let inFlight = 0;
  let maximumInFlight = 0;
  const pending = queueSharedLibrary(inventory([offered("first", uuid(1)), offered("second", uuid(2))]), [], [], {
    isCurrent: () => true,
    start(id) {
      started.push(id);
      maximumInFlight = Math.max(maximumInFlight, ++inFlight);
      if (id === "second") {
        assert.deepEqual(tracked, ["job-first"]);
        secondStarted.resolve();
      }
      return (id === "first" ? first.promise : second.promise).then((accepted) => {
        inFlight--;
        return accepted;
      });
    },
    onQueued: (accepted) => tracked.push(accepted.id), onProgress: coherent,
  });
  assert.deepEqual(started, ["first"]);
  first.resolve(job("first"));
  await secondStarted.promise;
  assert.deepEqual(started, ["first", "second"]);
  assert.equal(inFlight, 1);
  second.resolve(job("second"));
  const result = await pending;
  assert.equal(maximumInFlight, 1);
  assert.deepEqual(tracked, ["job-first", "job-second"]);
  assert.equal(result.queued, 2);
  assert.equal(result.stopped, false);
});

test("the first admission error returns a coherent partial result without retrying or advancing", async () => {
  const error = new Error("LOW_DISK_SPACE: confirmation declined");
  const started = [];
  const tracked = [];
  const snapshots = [];
  const result = await queueSharedLibrary(inventory([
    offered("installed", uuid(1)), offered("accepted", uuid(2)),
    offered("rejected", uuid(3), { title: "", fileName: "Rejected archive.zim" }),
    offered("must-not-start", uuid(4)),
  ]), [{ id: uuid(1), available: true }], [], {
    isCurrent: () => true,
    async start(id) {
      started.push(id);
      if (id === "rejected") throw error;
      return job(id);
    },
    onQueued: (accepted) => tracked.push(accepted),
    onProgress: (progress) => { coherent(progress); snapshots.push(progress); },
  });
  assert.deepEqual(started, ["accepted", "rejected"]);
  assert.deepEqual(tracked.map((accepted) => accepted.id), ["job-accepted"]);
  assert.deepEqual(counts(result), {
    total: 4, processed: 2, queued: 1, installed: 1, active: 0,
    unavailable: 0, duplicates: 0, stopped: true,
  });
  assert.equal(result.failure.bookId, "rejected");
  assert.equal(result.failure.title, "Rejected archive.zim");
  assert.equal(result.failure.error, error);
  assert.ok(Object.isFrozen(result.failure));
  assert.notEqual(result.failure, snapshots.at(-1).failure);
  assert.equal(snapshots.at(-1).failure.error, error);
  assert.equal(snapshots[1].failure, undefined);
});

test("retiring the owner during admission never orphans an accepted job or starts another", async () => {
  const admission = deferred();
  const accepted = job("first");
  const started = [];
  const tracked = [];
  let current = true;
  const pending = queueSharedLibrary(inventory([
    offered("first", uuid(1)), offered("second", uuid(2)), offered("third", uuid(3)),
  ]), [], [], {
    isCurrent: () => current,
    start(id) { started.push(id); return admission.promise; },
    onQueued: (queued) => { assert.equal(current, false); tracked.push(queued); },
    onProgress: coherent,
  });
  assert.deepEqual(started, ["first"]);
  current = false;
  admission.resolve(accepted);
  const result = await pending;
  assert.deepEqual(started, ["first"]);
  assert.equal(tracked.length, 1);
  assert.equal(tracked[0], accepted);
  assert.deepEqual(counts(result), {
    total: 3, processed: 1, queued: 1, installed: 0, active: 0,
    unavailable: 0, duplicates: 0, stopped: true,
  });
  assert.equal(result.failure, undefined);
});

test("a failure arriving after owner retirement stays a failure and admits no later item", async () => {
  const admission = deferred();
  const error = new Error("The shared-library receipt expired");
  let current = true;
  const started = [];
  const tracked = [];
  const pending = queueSharedLibrary(inventory([offered("first", uuid(1)), offered("second", uuid(2))]), [], [], {
    isCurrent: () => current,
    start(id) { started.push(id); return admission.promise; },
    onQueued: (accepted) => tracked.push(accepted), onProgress: coherent,
  });
  current = false;
  admission.reject(error);
  const result = await pending;
  assert.deepEqual(started, ["first"]);
  assert.deepEqual(tracked, []);
  assert.equal(result.processed, 0);
  assert.equal(result.queued, 0);
  assert.equal(result.stopped, true);
  assert.equal(result.failure.error, error);
});

test("cancellation after a classified skip stops before any later admission", async () => {
  let current = true;
  const started = [];
  const result = await queueSharedLibrary(inventory([
    offered("installed", uuid(1)), offered("missing", uuid(2)),
  ]), [{ id: uuid(1), available: true }], [], {
    isCurrent: () => current,
    start: async (id) => { started.push(id); return job(id); },
    onQueued() {},
    onProgress(progress) {
      coherent(progress);
      if (progress.processed === 1) current = false;
    },
  });
  assert.deepEqual(started, []);
  assert.equal(result.installed, 1);
  assert.equal(result.processed, 1);
  assert.equal(result.stopped, true);
});

test("ownership is checked both before an item and immediately before asynchronous admission", async () => {
  let ownershipChecks = 0;
  const started = [];
  const result = await queueSharedLibrary(inventory([offered("first", uuid(1))]), [], [], {
    isCurrent: () => ++ownershipChecks === 1,
    start: async (id) => { started.push(id); return job(id); },
    onQueued() {}, onProgress: coherent,
  });
  assert.equal(ownershipChecks, 2);
  assert.deepEqual(started, []);
  assert.equal(result.processed, 0);
  assert.equal(result.stopped, true);
});

test("an already retired owner neither admits nor tracks existing active jobs", async () => {
  const started = [];
  const tracked = [];
  const result = await queueSharedLibrary(inventory([offered("first", uuid(1))]), [], [job("first")], {
    isCurrent: () => false,
    start: async (id) => { started.push(id); return job(id); },
    onQueued: (accepted) => tracked.push(accepted), onProgress: coherent,
  });
  assert.deepEqual(started, []);
  assert.deepEqual(tracked, []);
  assert.equal(result.processed, 0);
  assert.equal(result.stopped, true);
});

test("retirement with no unprocessed items does not report a partial stop", async () => {
  let current = true;
  const tracked = [];
  const complete = await queueSharedLibrary(inventory([offered("only", uuid(1))]), [], [], {
    isCurrent: () => current,
    async start(id) { current = false; return job(id); },
    onQueued: (accepted) => tracked.push(accepted), onProgress: coherent,
  });
  assert.equal(tracked.length, 1);
  assert.equal(complete.processed, 1);
  assert.equal(complete.stopped, false);
  const empty = await queueSharedLibrary(inventory([]), [], [], {
    isCurrent: () => false,
    start: async () => { assert.fail("empty inventories never admit"); },
    onQueued() {}, onProgress: coherent,
  });
  assert.equal(empty.total, 0);
  assert.equal(empty.processed, 0);
  assert.equal(empty.stopped, false);
});

test("inventory membership, route session, and book metadata are fixed before the first await", async () => {
  const receipt = inventory([offered("first", uuid(1)), offered("second", uuid(2))]);
  const admission = deferred();
  const secondActive = job("second");
  const started = [];
  const tracked = [];
  const pending = queueSharedLibrary(receipt, [], [secondActive], {
    isCurrent: () => true,
    start(id) { started.push(id); return admission.promise; },
    onQueued: (accepted) => tracked.push(accepted), onProgress: coherent,
  });
  receipt.sessionId = foreignSession;
  receipt.books[1].id = "replacement";
  receipt.books[1].archiveId = uuid(1);
  receipt.books[1].downloadIdentity = "replacement-authority";
  receipt.books.push(offered("late", uuid(3)));
  admission.resolve(job("first"));
  const result = await pending;
  assert.deepEqual(started, ["first"]);
  assert.equal(tracked[1], secondActive);
  assert.deepEqual(counts(result), {
    total: 2, processed: 2, queued: 1, installed: 0, active: 1,
    unavailable: 0, duplicates: 0, stopped: false,
  });
});

test("each progress notification and the result are detached immutable snapshots", async () => {
  const snapshots = [];
  const result = await queueSharedLibrary(inventory([
    offered("first", uuid(1)), offered("alias", uuid(1)), offered("second", uuid(2)),
  ]), [], [], {
    isCurrent: () => true,
    start: async (id) => job(id), onQueued() {},
    onProgress(progress) {
      coherent(progress);
      assert.ok(Object.isFrozen(progress));
      assert.throws(() => { progress.queued = 999; }, TypeError);
      snapshots.push(progress);
    },
  });
  assert.deepEqual(snapshots.map((progress) => progress.processed), [0, 1, 2, 3]);
  assert.deepEqual(snapshots.map((progress) => progress.queued), [0, 1, 1, 2]);
  assert.equal(new Set(snapshots).size, snapshots.length);
  assert.notEqual(result, snapshots.at(-1));
  assert.ok(Object.isFrozen(result));
  assert.deepEqual(counts(snapshots[0]), {
    total: 3, processed: 0, queued: 0, installed: 0, active: 0,
    unavailable: 0, duplicates: 0, stopped: false,
  });
  assert.equal(result.duplicates, 1);
});
