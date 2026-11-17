import assert from "node:assert/strict";
import test from "node:test";
import { SuspendedError, parseSuspendedError } from "../src/error-visibility.ts";
import { ForegroundQueue, MAX_FALLBACK_RETRIES, retrySuspended } from "../src/suspended-retry.ts";

const suspended = () =>
  new SuspendedError("The application is not interactive");

test("a suspended command retries exactly once when the foreground resumes", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  let receipts = 0;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      if (attempts === 1) throw suspended();
      return Promise.resolve("ok");
    },
    gate,
    { enabled: true, onSuspend: () => (receipts += 1) },
  );
  assert.equal(attempts, 1);
  assert.equal(receipts, 1);
  assert.equal(gate.resume(), true);
  // A duplicate native/visibility signal must not fire a second retry.
  assert.equal(gate.resume(), false);
  assert.equal(await pending, "ok");
  assert.equal(attempts, 2);
  assert.equal(receipts, 1);
});

test("a second suspension propagates without a third attempt", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      throw suspended();
    },
    gate,
    { enabled: true },
  );
  assert.equal(gate.resume(), true);
  await assert.rejects(pending, SuspendedError);
  assert.equal(attempts, 2);
});

test("a genuine failure is never retried", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  await assert.rejects(
    retrySuspended(
      () => {
        attempts += 1;
        throw new Error("disk full");
      },
      gate,
      { enabled: true },
    ),
    /disk full/,
  );
  assert.equal(attempts, 1);
  assert.equal(gate.resume(), false);
});

test("a request whose context moved on is dropped quietly", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  let relevant = true;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      throw suspended();
    },
    gate,
    { enabled: true, stillRelevant: () => relevant },
  );
  relevant = false;
  assert.equal(gate.resume(), true);
  await assert.rejects(pending, SuspendedError);
  assert.equal(attempts, 1);
});

test("desktop never waits for a foreground gate", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  await assert.rejects(
    retrySuspended(
      () => {
        attempts += 1;
        throw suspended();
      },
      gate,
      { enabled: false },
    ),
    SuspendedError,
  );
  assert.equal(attempts, 1);
  assert.equal(gate.resume(), false);
});

test("an ended checkpoint settles without queuing or replaying on foreground", async () => {
  const gate = new ForegroundQueue();
  const ended = parseSuspendedError(JSON.stringify({
    code: "KIWIX_SUSPENDED", message: "Background checkpoint ended",
    retry: false, reason: "background-checkpoint-ended",
  }));
  let attempts = 0;
  const pending = retrySuspended(() => {
    attempts++;
    throw ended;
  }, gate, { enabled: true });
  const resumed = gate.resume();
  await assert.rejects(pending, SuspendedError);
  assert.equal(resumed, false);
  assert.equal(attempts, 1);
  assert.equal(gate.resume(), false);
});

test("a speculative fallback wake does not spend the command's retry", async () => {
  // The WebView can be visible while the native shell is still inactive
  // (Android Activity obscured by a system dialog). The visibility fallback
  // fires, the retry is suspended again, and the command must keep waiting
  // for the authoritative native foreground instead of being dropped.
  const gate = new ForegroundQueue();
  let attempts = 0;
  let interactive = false;
  let settled = false;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      if (!interactive) throw suspended();
      return Promise.resolve("saved");
    },
    gate,
    { enabled: true },
  ).finally(() => (settled = true));
  assert.equal(attempts, 1);
  assert.equal(gate.resume("fallback"), true);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(attempts, 2);
  assert.equal(settled, false, "a fallback re-suspension must not drop the command");
  interactive = true;
  assert.equal(gate.resume("native"), true);
  assert.equal(await pending, "saved");
  assert.equal(attempts, 3);
});

test("fallback guesses are bounded so a missing native event cannot poll forever", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      throw suspended();
    },
    gate,
    { enabled: true },
  );
  const rejected = assert.rejects(pending, SuspendedError);
  const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
  for (let i = 0; i < MAX_FALLBACK_RETRIES; i++) {
    assert.equal(gate.resume("fallback"), true);
    await tick();
  }
  await rejected;
  assert.equal(attempts, 1 + MAX_FALLBACK_RETRIES);
  assert.equal(gate.resume("fallback"), false, "nothing is left waiting");
});

test("a fallback wake still drops a request whose context moved on", async () => {
  const gate = new ForegroundQueue();
  let attempts = 0;
  let relevant = true;
  const pending = retrySuspended(
    () => {
      attempts += 1;
      throw suspended();
    },
    gate,
    { enabled: true, stillRelevant: () => relevant },
  );
  gate.resume("fallback");
  await new Promise((resolve) => setTimeout(resolve, 0));
  relevant = false;
  gate.resume("fallback");
  await assert.rejects(pending, SuspendedError);
  assert.equal(attempts, 2);
});
