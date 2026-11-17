import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import {
  androidLocaleQualifier, androidStringResource, translationPlaceholderToken,
  restoreTranslationPlaceholders, appleStringEntries, androidStringMetadata,
  reusableCommunityEntry, excludeFuzzyInherited, reviewedTranslation,
  addCatalogTranslation,
} from "./localization.mjs";

test("Android resource locales retain script and region rather than collapsing Chinese", () => {
  assert.equal(androidLocaleQualifier("zh-Hans"), "values-b+zh+Hans");
  assert.equal(androidLocaleQualifier("zh-Hant"), "values-b+zh+Hant");
  assert.equal(androidLocaleQualifier("pt_BR"), "values-b+pt+BR");
  assert.equal(androidLocaleQualifier("sr-ec"), "values-b+sr+Cyrl");
  assert.throws(() => androidLocaleQualifier("../bad"), RangeError);
});

test("Android string resources escape both XML and Android's quoted string syntax", () => {
  assert.equal(androidStringResource('L\'archive & <livre> "A"\\B\n{title}'), '"L\\\'archive &amp; &lt;livre&gt; \\"A\\"\\\\B\\n{title}"');
  assert.equal(androidStringResource("@library  100% ?"), '"@library  100% ?"');
});

test("translation placeholders are opaque URLs, not locale-formatted bare numbers", () => {
  const count = translationPlaceholderToken(0);
  const title = translationPlaceholderToken(1);
  assert.equal(new URL(count).hostname, "kiwix.invalid");
  assert.equal(
    restoreTranslationPlaceholders(`Envoyé : ${count}, « ${title} ». ${count}`, ["count", "title"]),
    "Envoyé : {count}, « {title} ». {count}",
  );
  assert.equal(restoreTranslationPlaceholders("93 847 000", ["count"]), "93 847 000", "do not guess grouped numbers into authority");
  const unknown = translationPlaceholderToken(2);
  assert.equal(restoreTranslationPlaceholders(unknown, ["count"]), unknown, "unknown slot is rejected by the caller's placeholder check");
});

test("Apple fuzzy comments bind to one assignment across context comments", () => {
  const entries = appleStringEntries(`
    // Author: A contributor is not a reviewer.
    // Fuzzy
    // Context: permission dialog, not an article title.
    "permission" = "First {title} %@";
    "neighbor" = "Second {title} %1$s";
    /* Fuzzy */
    /* Context: library toolbar */
    "toolbar" = "Third";
    "literal" = "// Fuzzy is quoted text, not a status";
    // "commented" = "must not become a candidate";
    /* "also-commented" = "must not become a candidate"; */
    "escaped" = "Quote: \\"A\\"; Unicode: \\U00e9";
  `);
  assert.deepEqual(entries.permission, { value: "First {title} %@", fuzzy: true });
  assert.deepEqual(entries.neighbor, { value: "Second {title} %1$s", fuzzy: false });
  assert.deepEqual(entries.toolbar, { value: "Third", fuzzy: true });
  assert.deepEqual(entries.literal, { value: "// Fuzzy is quoted text, not a status", fuzzy: false });
  assert.deepEqual(entries.escaped, { value: 'Quote: "A"; Unicode: é', fuzzy: false });
  assert.equal(entries.commented, undefined);
  assert.equal(entries["also-commented"], undefined);
});

test("Android fuzzy attributes do not bleed into neighbors or plural items", () => {
  const entries = androidStringMetadata(`
    <resources>
      <!-- <string name="commented" fuzzy="true">not a candidate</string> -->
      <string-array name="array"><item>not a flat string</item></string-array>
      <string name="first" fuzzy="true">First {title} %1$s</string>
      <!-- Context and fuzzy prose are not XML flags. -->
      <string formatted="false" name='neighbor'>Second {title} %1$s</string>
      <string fuzzy = 'false' name="explicit">Third</string>
      <string name="numeric" fuzzy="1">Fourth</string>
      <plurals name="count">
        <item quantity="one">{count} item</item>
        <item quantity="other">{count} items</item>
      </plurals>
      <string translatable="false" name="constant">Kiwix</string>
    </resources>
  `);
  assert.deepEqual(entries, {
    first: { fuzzy: true, translatable: true },
    neighbor: { fuzzy: false, translatable: true },
    explicit: { fuzzy: false, translatable: true },
    numeric: { fuzzy: true, translatable: true },
    constant: { fuzzy: false, translatable: false },
  });
  assert.equal(reusableCommunityEntry({ fuzzy: true, sourceFuzzy: false }), false);
  assert.equal(reusableCommunityEntry({ fuzzy: false, sourceFuzzy: true }), false);
  assert.equal(reusableCommunityEntry({ authors: ["Contributor"] }), false);
  assert.equal(reusableCommunityEntry({ fuzzy: false, sourceFuzzy: false }), true);
});

test("default generation removes only the exact fuzzy pinned value and origin", () => {
  const entry = {
    value: "Imported", revision: "revision", path: "values-nqo/strings.xml",
    key: "key", fuzzy: true, sourceFuzzy: false,
  };
  const resource = "android:revision:values-nqo/strings.xml#key";
  const catalogs = { nqo: {
    "Search history": "Imported", "Open library": "Other translation",
    "Same text": "Imported", "Non-fuzzy": "Neighbor",
  } };
  const provenance = { nqo: {
    "Search history": resource, "Open library": resource,
    "Same text": "qt", "Non-fuzzy": resource,
  } };
  const community = { imports: { nqo: {
    "Search history": entry, "Open library": entry, "Same text": entry,
    "Non-fuzzy": { ...entry, value: "Neighbor", fuzzy: false },
  } } };
  const excluded = excludeFuzzyInherited(catalogs, provenance, community);
  assert.deepEqual(excluded.map(({ code, source }) => ({ code, source })),
    [{ code: "nqo", source: "Search history" }]);
  assert.equal(catalogs.nqo["Search history"], undefined);
  assert.equal(provenance.nqo["Search history"], undefined);
  assert.equal(catalogs.nqo["Open library"], "Other translation");
  assert.equal(catalogs.nqo["Same text"], "Imported");
  assert.equal(catalogs.nqo["Non-fuzzy"], "Neighbor");
});

const reviewRecord = (value) => ({
  reviewed: true,
  review: {
    locale: "fr", source: "{count} items", context: "Library item count",
    translator: "Translator identity", reviewer: "Independent reviewer identity",
    competence: "Attributable French language qualification",
    evidenceUrl: "https://example.org/reviews/exact-source-and-context",
    ...(typeof value === "string" ? { text: value } : { forms: value }),
  },
});

test("review authority requires exact locale/source/text/context and public attributable evidence", () => {
  const source = "{count} items", text = "{count} éléments";
  const record = reviewRecord(text);
  assert.equal(reviewedTranslation({ reviewed: true }, "fr", source, text), null);
  assert.equal(reviewedTranslation({ authors: ["Contributor"] }, "fr", source, text), null);
  assert.equal(reviewedTranslation(record, "fr-CA", source, text), null);
  assert.equal(reviewedTranslation(record, "fr", "Other context source", text), null);
  assert.equal(reviewedTranslation(record, "fr", source, "Changed translation"), null);
  for (const patch of [
    { context: "" }, { reviewer: "" }, { competence: "" },
    { reviewer: record.review.translator },
    { evidenceUrl: "file:///private/review" },
    { evidenceUrl: "https://credentials:secret@example.org/review" },
  ]) {
    assert.equal(reviewedTranslation({ ...record, review: { ...record.review, ...patch } },
      "fr", source, text), null);
  }
  assert.equal(reviewedTranslation(record, "fr", source, text)?.status, "reviewed");
  const scriptRecord = { ...record, review: { ...record.review, locale: "zh-Hans" } };
  assert.equal(reviewedTranslation(scriptRecord, "zh-Hant", source, text), null);
});

test("explicit review survives lower-confidence refresh inputs without granting authority to a flag", () => {
  const catalogs = {}, provenance = {};
  const source = "{count} items", text = "{count} éléments";
  addCatalogTranslation(catalogs, provenance, "fr", source, "Community", "qt");
  const record = reviewRecord(text);
  const review = reviewedTranslation(record, "fr", source, text);
  addCatalogTranslation(catalogs, provenance, "fr", source, text, review);
  for (const resource of ["qt", "apple", "android:revision:path#key", "machine",
    { reviewed: true }])
    addCatalogTranslation(catalogs, provenance, "fr", source, "Lower confidence", resource);
  assert.equal(catalogs.fr[source], text);
  assert.deepEqual(provenance.fr[source], review);
  const refreshedCatalogs = {}, refreshedProvenance = {};
  addCatalogTranslation(refreshedCatalogs, refreshedProvenance, "fr", source,
    catalogs.fr[source], provenance.fr[source]);
  addCatalogTranslation(refreshedCatalogs, refreshedProvenance, "fr", source,
    "New upstream value", "qt");
  assert.equal(refreshedCatalogs.fr[source], text);
});

test("review evidence binds exact plural forms without rewriting placeholders or categories", () => {
  const forms = { one: "{count} élément", many: "{count} éléments", other: "{count} éléments" };
  const record = reviewRecord(forms);
  const before = structuredClone(forms);
  assert.equal(reviewedTranslation(record, "fr", "{count} items", forms)?.status, "reviewed");
  assert.equal(reviewedTranslation(record, "fr", "{count} items",
    { ...forms, one: "Un élément" }), null);
  assert.equal(reviewedTranslation(record, "fr", "{count} items",
    { one: forms.one, other: forms.other }), null);
  assert.deepEqual(forms, before);
});

test("pinned Android candidates retain explicit fuzzy metadata without gaining review authority", async () => {
  const input = JSON.parse(await readFile(
    new URL("../src/locales/community-provenance.json", import.meta.url), "utf8",
  ));
  const candidates = Object.values(input.imports).flatMap(Object.values);
  for (const entry of candidates) {
    assert.equal(typeof entry.fuzzy, "boolean");
    assert.equal(typeof entry.sourceFuzzy, "boolean");
    assert.equal(reviewedTranslation(entry, "nqo", "Search history", entry.value), null);
  }
  assert.equal(candidates.filter(reusableCommunityEntry).length, input.eligibleMessageCount);
  assert.equal(candidates.filter((entry) => !reusableCommunityEntry(entry)).length,
    input.excludedFuzzyMessageCount);
  for (const source of ["Search history", "Open library"])
    assert.equal(reusableCommunityEntry(input.imports.nqo[source]), false);
});
