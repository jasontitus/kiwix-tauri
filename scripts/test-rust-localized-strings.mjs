// Every source the Rust core hands its Localizer must be an English catalog
// source, or it reaches native UI and the shared-library web pages untranslated
// in every language with no inventory noticing. Covered call shapes:
//   - `.text("…")` literals;
//   - `t("…")` where a file defines `let t = |…| … locale.text(…)`;
//   - `.counted("…")` literals (plural sources, see plural-sources.json);
//   - `mobile_labels(…)` literal arguments;
//   - `live_activity.rs` `label()` arms;
//   - VARIABLE_SOURCES below: call sites whose source is a variable, listed by
//     hand. Each listed literal must still occur in its file, so the list cannot
//     silently go stale when a call site changes.
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const LITERAL = String.raw`"((?:[^"\\]|\\.)*)"`;
const unescape = (literal) => JSON.parse(`"${literal}"`);

const VARIABLE_SOURCES = [
  // spawn_secondary_paths: `let title = if … { "Look up in Kiwix" } else { "Import archives" }`
  { file: "src-tauri/src/lib.rs", sources: ["Look up in Kiwix", "Import archives"] },
  // mobile_export_authorized: `let title = if share { "Share" } else { "Save export" }`
  { file: "src-tauri/src/mobile_service.rs", sources: ["Share", "Save export"] },
  // update_tabbing_menu: `let sources = [ … ]`
  {
    file: "src-tauri/src/windows.rs",
    sources: ["Window", "Show Previous Tab", "Show Next Tab", "Merge All Windows",
      "Move Tab to New Window", "Show Tab Bar", "Hide Tab Bar", "Minimize", "Zoom",
      "Bring All to Front"],
  },
  // OPDS facet feed titles: `FacetKind::… => (…, …, "Categories" | "Languages")`
  { file: "src-tauri/src/sharing.rs", sources: ["Categories", "Languages"] },
  // Shared-library search note, `let source = if book.fulltext { … } else { … }`
  // passed to `locale.counted(source, …)`.
  {
    file: "src-tauri/src/sharing.rs",
    plural: true,
    sources: ["Showing {shown} of approximately {count} matches.",
      "Showing {shown} of approximately {count} matches by title prefix."],
  },
];

async function rustFiles(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await rustFiles(full)));
    else if (entry.name.endsWith(".rs") && !/(?:^|_)tests?\.rs$/.test(entry.name)) out.push(full);
  }
  return out;
}

async function catalog() {
  const dir = path.join(root, "src/locales");
  const read = async (file) => JSON.parse(await readFile(path.join(dir, file), "utf8"));
  const flat = new Set(Object.keys(await read("messages/en.json")));
  for (const file of await readdir(dir))
    if (/^extra-en-.*\.json$/.test(file)) for (const key of Object.keys(await read(file))) flat.add(key);
  const plural = new Set(Object.keys(await read("plural-sources.json")));
  return { flat, plural };
}

test("every source the Rust core localizes is an English catalog source", async () => {
  const { flat, plural } = await catalog();
  const missing = [];
  const seen = { text: 0, closure: 0, counted: 0, labels: 0, label: 0, variable: 0 };
  const check = (kind, where, key, plurals = false) => {
    seen[kind] += 1;
    if (!(plurals ? plural.has(key) : flat.has(key))) missing.push(`${where}: ${key}`);
  };
  for (const file of await rustFiles(path.join(root, "src-tauri/src"))) {
    const text = await readFile(file, "utf8");
    const relative = path.relative(root, file);
    for (const m of text.matchAll(new RegExp(String.raw`\.text\(\s*${LITERAL}`, "g")))
      check("text", relative, unescape(m[1]));
    if (/let t = \|[^|]*\|[^;]*\.text\(/.test(text))
      for (const m of text.matchAll(new RegExp(String.raw`(?<![\w.])t\(\s*${LITERAL}`, "g")))
        check("closure", `${relative} t()`, unescape(m[1]));
    for (const m of text.matchAll(new RegExp(String.raw`\.counted\(\s*${LITERAL}`, "g")))
      check("counted", `${relative} counted()`, unescape(m[1]), true);
    for (const m of text.matchAll(/mobile_labels\(((?:[^()]|\([^()]*\))*)\)/g))
      for (const s of m[1].matchAll(new RegExp(LITERAL, "g")))
        check("labels", `${relative} mobile_labels()`, unescape(s[1]));
    if (relative.endsWith("live_activity.rs")) {
      const body = text.match(/fn label\(status: &str\) -> &'static str \{([\s\S]*?)\n\}/);
      assert.ok(body, "live_activity.rs label() not found; update this test");
      for (const m of body[1].matchAll(new RegExp(String.raw`=> ${LITERAL}`, "g")))
        check("label", `${relative} label()`, unescape(m[1]));
    }
  }
  for (const { file, sources, plural: plurals } of VARIABLE_SOURCES) {
    const text = await readFile(path.join(root, file), "utf8");
    for (const source of sources) {
      assert.ok(text.includes(JSON.stringify(source)), `${file} no longer contains ${JSON.stringify(source)}; update VARIABLE_SOURCES`);
      check("variable", `${file} (variable source)`, source, plurals);
    }
  }
  // Floors: a pattern that silently stops matching must fail, not pass.
  assert.ok(seen.text > 30, `found only ${seen.text} .text() literals`);
  assert.ok(seen.closure > 10, `found only ${seen.closure} t() closure literals`);
  assert.ok(seen.counted >= 2, `found only ${seen.counted} .counted() literals`);
  assert.ok(seen.labels >= 8, `found only ${seen.labels} mobile_labels() literals`);
  assert.ok(seen.label >= 10, `found only ${seen.label} live_activity label() arms`);
  assert.deepEqual(missing, [],
    "Add flat sources to src/locales/extra-en-rust.json and plural ones to plural-sources.json, then run `node scripts/localization.mjs generate`");
});
