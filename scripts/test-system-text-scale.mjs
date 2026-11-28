import assert from "node:assert/strict";
import test from "node:test";
import { ANDROID_TEXT_CATEGORY, applySystemTextScale } from "../src/system-text-scale.ts";
import { readFile } from "node:fs/promises";

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

test("Android font scale selects the large-text layout without scaling text twice", () => {
  // The WebView already enlarges text by the system font scale.
  const root = shell();
  applySystemTextScale({ category: ANDROID_TEXT_CATEGORY, scale: 1.5 }, root);
  assert.equal(root.properties.get("--system-text-scale"), "1");
  assert.equal(root.dataset.largeText, "true");
  assert.equal(root.dataset.contentSizeCategory, "android");
  assert.equal(root.dataset.readerFontSize, "125");

  // Android 14 offers up to 200%; the default and modest sizes keep the ordinary layout.
  applySystemTextScale({ category: ANDROID_TEXT_CATEGORY, scale: 2 }, root);
  assert.equal(root.properties.get("--system-text-scale"), "1");
  assert.equal(root.dataset.largeText, "true");
  for (const scale of [0.85, 1, 1.15, 1.3]) {
    applySystemTextScale({ category: ANDROID_TEXT_CATEGORY, scale }, root);
    assert.equal(root.properties.get("--system-text-scale"), "1");
    assert.equal(root.dataset.largeText, "false");
  }
});

test("the Android plugin reports the category that suppresses double scaling", async () => {
  const plugin = await readFile(new URL(
    "../src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/KiwixMobilePlugin.kt",
    import.meta.url), "utf8");
  assert.match(plugin, new RegExp(`SYSTEM_TEXT_CATEGORY = "${ANDROID_TEXT_CATEGORY}"`));
  assert.match(plugin, /fun dynamicTypeState\(invoke: Invoke\)[\s\S]{0,200}put\("category", SYSTEM_TEXT_CATEGORY\)[\s\S]{0,120}put\("scale", /);
});

test("large-text reflow rules cover Android as well as iOS", async () => {
  const css = await readFile(new URL("../src/platform.css", import.meta.url), "utf8");
  const rules = css.match(/^[^\n{}]*\[data-large-text="true"\][^\n{]*\{/gm) ?? [];
  assert.ok(rules.length >= 16, `expected the large-text rules, found ${rules.length}`);
  for (const rule of rules) {
    assert.match(rule, /:root:is\(\[data-platform="ios"\], \[data-platform="android"\]\)\[data-large-text="true"\]/,
      `large-text rule does not apply on Android: ${rule}`);
  }
});

test("a font-scale change still recreates the Android Activity", async () => {
  // The shell reads the scale once per document. That is only correct while
  // Android answers a font-scale change by recreating the Activity and WebView.
  const manifest = await readFile(new URL(
    "../src-tauri/gen/android/app/src/main/AndroidManifest.xml", import.meta.url), "utf8");
  const handled = /android:configChanges="([^"]*)"/.exec(manifest)?.[1].split("|") ?? [];
  assert.ok(handled.includes("uiMode"), "the manifest lists its handled configuration changes");
  assert.ok(!handled.includes("fontScale"));
});
