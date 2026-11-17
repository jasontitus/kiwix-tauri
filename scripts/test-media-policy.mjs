import assert from "node:assert/strict";
import test from "node:test";
import { shouldKeepReaderForBackgroundMedia } from "../src/media-policy.ts";

const base = {
  audible: true,
  pictureInPicture: false,
  privateMode: false,
  readOnly: false,
  retiring: false,
  quitting: false,
};

test("keeps the reader for audible background media", () => {
  assert.equal(shouldKeepReaderForBackgroundMedia(base), true);
});

test("keeps the reader for an active Picture-in-Picture presentation", () => {
  assert.equal(
    shouldKeepReaderForBackgroundMedia({ ...base, audible: false, pictureInPicture: true }),
    true,
  );
});

test("does not keep the reader without active media", () => {
  assert.equal(
    shouldKeepReaderForBackgroundMedia({ ...base, audible: false, pictureInPicture: false }),
    false,
  );
});

test("privacy and read-only revocation always retire", () => {
  assert.equal(shouldKeepReaderForBackgroundMedia({ ...base, privateMode: true }), false);
  assert.equal(shouldKeepReaderForBackgroundMedia({ ...base, readOnly: true }), false);
});

test("explicit retirement and shutdown always retire", () => {
  assert.equal(shouldKeepReaderForBackgroundMedia({ ...base, retiring: true }), false);
  assert.equal(shouldKeepReaderForBackgroundMedia({ ...base, quitting: true }), false);
});
