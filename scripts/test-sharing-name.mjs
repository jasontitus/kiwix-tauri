import assert from "node:assert/strict";
import test from "node:test";
import { validSharingName } from "../src/sharing-name.ts";

test("sharing names use the native UTF-8 byte bound, not input character count", () => {
  assert.equal(validSharingName("a".repeat(160)), true);
  assert.equal(validSharingName("a".repeat(161)), false);
  assert.equal(validSharingName("é".repeat(80)), true);
  assert.equal(validSharingName("é".repeat(81)), false);
  assert.equal(validSharingName("📚".repeat(40)), true);
  assert.equal(validSharingName("📚".repeat(41)), false);
  for (const name of ["", "図書館の端末", "مكتبة", "Jason’s phone"]) {
    assert.equal(validSharingName(name), true);
  }
});

test("unsafe names cannot enter the preference queue", () => {
  for (const name of [" padded", "trailing ", "two\nlines", "\u0085", "spoof\u202e", "\u2066name", "\ud800", "\udfff"]) {
    assert.equal(validSharingName(name), false);
  }
});
