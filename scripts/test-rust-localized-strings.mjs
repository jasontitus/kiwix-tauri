// Every literal the Rust core passes to its Localizer (`.text("…")`), and every
// native download status label, must be an English catalog source. Otherwise
// it reaches native UI (dialogs, notifications, Live Activities) untranslated
// in every language and no inventory notices.
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function rustFiles(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await rustFiles(full)));
    else if (entry.name.endsWith(".rs")) out.push(full);
  }
  return out;
}

async function englishSources() {
  const dir = path.join(root, "src/locales");
  const sources = new Set(
    Object.keys(JSON.parse(await readFile(path.join(dir, "messages/en.json"), "utf8"))),
  );
  for (const file of await readdir(dir))
    if (/^extra-en-.*\.json$/.test(file))
      for (const key of Object.keys(JSON.parse(await readFile(path.join(dir, file), "utf8"))))
        sources.add(key);
  return sources;
}

const unescape = (literal) => JSON.parse(`"${literal}"`);

test("every Rust localizer literal and native status label is a catalog source", async () => {
  const sources = await englishSources();
  const missing = [];
  let seen = 0;
  for (const file of await rustFiles(path.join(root, "src-tauri/src"))) {
    const text = await readFile(file, "utf8");
    const relative = path.relative(root, file);
    for (const match of text.matchAll(/\.text\(\s*"((?:[^"\\]|\\.)*)"/g)) {
      seen += 1;
      const key = unescape(match[1]);
      if (!sources.has(key)) missing.push(`${relative}: ${key}`);
    }
    if (relative.endsWith("live_activity.rs")) {
      const body = text.match(/fn label\(status: &str\) -> &'static str \{([\s\S]*?)\n\}/);
      assert.ok(body, "live_activity.rs label() not found; update this test");
      for (const match of body[1].matchAll(/=> "((?:[^"\\]|\\.)*)"/g)) {
        seen += 1;
        const key = unescape(match[1]);
        if (!sources.has(key)) missing.push(`${relative} label(): ${key}`);
      }
    }
  }
  assert.ok(seen > 10, `found only ${seen} localized Rust literals; the pattern may be stale`);
  assert.deepEqual(missing, [], "Add these to src/locales/extra-en-rust.json, then run `node scripts/localization.mjs generate`");
});
