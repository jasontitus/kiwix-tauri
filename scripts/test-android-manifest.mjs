import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = (path) => readFile(new URL(`../src-tauri/${path}`, import.meta.url), "utf8");

async function handledConfigurationChanges() {
  const manifest = await read("gen/android/app/src/main/AndroidManifest.xml");
  const activity = /<activity\b[^>]*android:name="\.MainActivity"[^>]*>/s.exec(manifest)?.[0] ?? "";
  const handled = /android:configChanges="([^"]*)"/.exec(activity)?.[1];
  assert.ok(handled, "MainActivity declares the configuration changes it handles");
  return handled.split("|");
}

test("attaching a keyboard or controller does not recreate the Activity", async () => {
  // A recreation cancels a running import and stops read-aloud. A hardware
  // keyboard with arrow keys changes `navigation` together with `keyboard`.
  const handled = await handledConfigurationChanges();
  for (const change of ["keyboard", "keyboardHidden", "navigation"])
    assert.ok(handled.includes(change), `${change} must be handled in place`);
});

test("changes the shell reads only once still recreate the Activity", async () => {
  // The shell reads the system font scale once per document, and the WebView
  // fixes its device pixel ratio when it is created.
  const handled = await handledConfigurationChanges();
  for (const change of ["fontScale", "density"])
    assert.ok(!handled.includes(change), `${change} must recreate the Activity`);
});

test("a restored Activity tells the plugin its launch intent was already delivered", async () => {
  const activity = await read("gen/android/app/src/main/java/org/kiwix/workbench/MainActivity.kt");
  assert.match(activity, /KiwixMobilePlugin\.activityCreated\(this, savedInstanceState != null\)/);
  const plugin = await read(
    "plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/KiwixMobilePlugin.kt");
  // Neither the first load nor a resume may read the launch intent directly.
  assert.doesNotMatch(plugin, /receive(?:Documents|Entries|CompletionNotification)\((?:owner|activity)\.intent\)/);
  assert.equal(plugin.match(/= launchIntent\((?:owner|activity)\)/g)?.length, 2);
});

test("native methods keep the names Rust links against in a minified build", async () => {
  const rules = await read("plugins/tauri-plugin-kiwix-mobile/android/consumer-rules.pro");
  assert.match(rules, /-keepclasseswithmembernames[^\n]*class org\.kiwix\.mobile\.\*\* \{\s*native <methods>;\s*\}/);
});
