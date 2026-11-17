import assert from "node:assert/strict";
import test from "node:test";
import {
  sensorDecisionWriteOutcome,
  sensorRequestDisposition,
} from "../src/sensor-policy.ts";
import { ERROR_SITES, shouldRenderError } from "../src/error-visibility.ts";

test("a failed sensor decision is a blocking error and does not reload", () => {
  const outcome = sensorDecisionWriteOutcome(false);
  assert.deepEqual(outcome, { reload: false, blocking: true });
  // The site must paint even with "Show technical error details" off.
  assert.equal(ERROR_SITES["reader.sensor-revoke"], "blocking");
  assert.equal(
    shouldRenderError(ERROR_SITES["reader.sensor-revoke"], false),
    true,
  );
});

test("a saved sensor decision reloads without a blocking error", () => {
  assert.deepEqual(sensorDecisionWriteOutcome(true), {
    reload: true,
    blocking: false,
  });
});

test("a stale granted status cannot auto-allow after the user blocked", () => {
  // The revoke failed, so the live store still reports granted, but the user's
  // last explicit choice for this installation was block.
  assert.equal(sensorRequestDisposition("granted", "denied"), "deny");
  // Asking again (reset) must never replay a grant the store still holds.
  assert.equal(sensorRequestDisposition("granted", "notAsked"), "prompt");
  // A live denial wins over an old cached grant even without an intent.
  assert.equal(sensorRequestDisposition("denied", undefined), "deny");
  assert.equal(sensorRequestDisposition("denied", "granted"), "deny");
});

test("the successful path still auto-grants a previously allowed installation", () => {
  // No decision made in this mount: the live store grant is the user's earlier
  // explicit choice, so it may be replayed.
  assert.equal(sensorRequestDisposition("granted", undefined), "allow");
  assert.equal(sensorRequestDisposition("granted", "granted"), "allow");
  // A store that lost the grant falls back to a prompt, never a silent allow.
  assert.equal(sensorRequestDisposition("notAsked", "granted"), "prompt");
  assert.equal(sensorRequestDisposition("notAsked", undefined), "prompt");
});
