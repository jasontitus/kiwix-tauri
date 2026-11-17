import assert from "node:assert/strict";
import test from "node:test";
import { BYTE_UNITS, byteUnitIndex } from "../src/byte-units.ts";

test("decimal units switch at exact thousand boundaries", () => {
  assert.equal(byteUnitIndex(0), 0);
  assert.equal(byteUnitIndex(1), 0);
  assert.equal(byteUnitIndex(999), 0);
  assert.equal(byteUnitIndex(1000), 1);
  assert.equal(byteUnitIndex(1023), 1);
  assert.equal(byteUnitIndex(1_000_000), 2);
  assert.equal(byteUnitIndex(1_000_000_000), 3);
});

test("rounding never prints 1000 of the smaller unit", () => {
  // 999.9 kB keeps kilobytes; 999.95 kB rounds to 1000.0 and must become 1 MB
  // (Android #5246 reported the same 1,000 MB / 1 GB boundary).
  assert.equal(byteUnitIndex(999_940), 1);
  assert.equal(byteUnitIndex(999_950), 2);
  assert.equal(byteUnitIndex(999_999), 2);
  assert.equal(byteUnitIndex(999_950_000), 3);
  assert.equal(byteUnitIndex(999_999_999), 3);
});

test("the largest unit is capped", () => {
  assert.equal(byteUnitIndex(1e18), BYTE_UNITS.length - 1);
  assert.equal(byteUnitIndex(Number.MAX_SAFE_INTEGER), BYTE_UNITS.length - 1);
});

test("invalid input stays on bytes", () => {
  assert.equal(byteUnitIndex(-1), 0);
  assert.equal(byteUnitIndex(Number.NaN), 0);
  assert.equal(byteUnitIndex(Number.POSITIVE_INFINITY), 0);
});
