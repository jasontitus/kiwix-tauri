import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { inventory, inventoryFailures } from "./localization.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// The inventory half of `node scripts/localization.mjs audit`. Sources that only lack a
// translation fall back to English by documented policy and are not failures here.
test("every UI string in the sources is collected in the localization manifest", async () => {
  const manifest = JSON.parse(await readFile(path.join(root, "src/locales/manifest.json"), "utf8"));
  const failures = inventoryFailures(manifest, await inventory());
  assert.deepEqual(
    failures, [],
    "Run `node scripts/localization.mjs generate` for uncollected keys; pass other UI copy through t().",
  );
});

test("the inventory reports uncollected, dynamic and untranslated copy, and nothing else", async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), "kiwix-localization-inventory-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(path.join(directory, "Example.tsx"), [
    "export function Example({ label, count }) {",
    "  return <dl>",
    '    <dt>{t("Collected source")}</dt><dd>{t("Not yet collected")}</dd>',
    "    <dt>Zimru</dt><dt>zimfile-xapian</dt><dd><kbd>Ctrl</kbd></dd>",
    "    <dd>Plain words</dd>",
    '    <input placeholder="Type here" title={t(label)} />',
    '    <p>{t("Only English: {count} items", { count: formatNumber(count) })}</p>',
    "  </dl>;",
    "}",
    "",
  ].join("\n"));
  // The catalog implementation forwards runtime sources; its literal copy is still collected.
  await writeFile(path.join(directory, "i18n.tsx"), [
    "export function tPlural(source, count) { return t(source, { count }); }",
    'export function Failure() { return <strong title="Raw">{t("Provider failure copy")} raw</strong>; }',
    "",
  ].join("\n"));
  const found = await inventory(directory);
  assert.ok(found.sources.includes("Provider failure copy"));
  assert.deepEqual(found.untranslated.map((entry) => entry.text), ["Plain words", "Type here"]);
  assert.equal(found.dynamic.length, 1);
  assert.match(found.dynamic[0], /Example\.tsx:6$/);
  for (const source of ["Collected source", "Not yet collected", "Only English: {count} items"])
    assert.ok(found.sources.includes(source), source);

  // A collected source with no translation in any locale is the documented English fallback,
  // and a formatted plural count belongs to the full audit; neither is an inventory failure.
  const manifest = {
    sources: found.sources.filter((source) => source !== "Not yet collected"),
    missingNew: { fr: ["Only English: {count} items", "Collected source"] },
  };
  assert.equal(found.invalidCounts.length, 1);
  const failures = inventoryFailures(manifest, found);
  assert.equal(failures.length, 4, failures.join("\n"));
  assert.equal(failures[0], "Uncollected UI key: Not yet collected");
  assert.match(failures[1], /^Dynamic t\(\) source cannot be inventoried: .*Example\.tsx:6$/);
  assert.match(failures[2], /^Untranslated JSX at .*Example\.tsx:5: Plain words$/);
  assert.match(failures[3], /^Untranslated JSX at .*Example\.tsx:6: Type here$/);
  assert.deepEqual(inventoryFailures({ sources: found.sources }, { ...found, dynamic: [], untranslated: [] }), []);
});
