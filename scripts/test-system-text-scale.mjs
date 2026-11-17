import assert from "node:assert/strict";
import test from "node:test";
import { applySystemTextScale } from "../src/system-text-scale.ts";

function shell() {
  const properties = new Map([["--reader-font-size", "125%"]]);
  return {
    properties,
    style: { setProperty: (key, value) => properties.set(key, value) },
    dataset: { readerFontSize: "125" },
  };
}

test("accessibility Dynamic Type is not capped or persisted as article size", () => {
  const root = shell();
  applySystemTextScale({ category: "UICTContentSizeCategoryAccessibilityXXXL", scale: 3.12 }, root);
  assert.equal(root.properties.get("--system-text-scale"), "3.12");
  assert.equal(root.dataset.largeText, "true");
  assert.equal(root.dataset.readerFontSize, "125");
  assert.equal(root.properties.get("--reader-font-size"), "125%");
});

test("a live change back to ordinary text restores the ordinary layout", () => {
  const root = shell();
  applySystemTextScale({ category: "accessibility", scale: 2.5 }, root);
  applySystemTextScale({ category: "large", scale: 1 }, root);
  assert.equal(root.properties.get("--system-text-scale"), "1");
  assert.equal(root.dataset.contentSizeCategory, "large");
  assert.equal(root.dataset.largeText, "false");
});

test("reset and malformed native scales cannot leave chrome unusable", () => {
  for (const value of [null, { category: "invalid", scale: NaN }, { category: "invalid", scale: Infinity }, { category: "invalid", scale: 0 }, { category: "invalid", scale: -1 }]) {
    const root = shell();
    applySystemTextScale(value, root);
    assert.equal(root.properties.get("--system-text-scale"), "1");
    assert.equal(root.dataset.largeText, "false");
  }
  applySystemTextScale(null, undefined);
});
