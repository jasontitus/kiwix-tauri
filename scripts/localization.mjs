#!/usr/bin/env node
/** Offline catalog compiler and explicit, build-time-only translation acquisition.
 * Usage: node scripts/localization.mjs generate
 * Refresh inherited resources only explicitly: generate --qt-root=../kiwix-desktop/resources/i18n --apple-root=../kiwix-apple
 *        node scripts/localization.mjs translate [--locales=fr,de] [--concurrency=4]
 *        node scripts/localization.mjs audit [--dom-snapshot=/path/to/snapshot.json]
 *        node scripts/localization.mjs inventory
 * Community translations retain priority. Machine output is unreviewed, never native-reviewed.
 * Missing inherited translations fall back to English; missing NEW copy fails the audit.
 * `inventory` is the part of the audit that must always pass: every UI string in the
 * sources is collected in manifest.json. It reads only and never reports the fallback gaps.
 * Generation publishes one messages/<BCP47>.json per locale. Runtime statically imports English
 * plus locale-index.json; Vite lazily loads other bundled locales without online translation.
 * whenLocaleReady() and document.documentElement.dataset.localeReady expose chunk readiness.
 */
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.join(root, "src/locales");
const args = {};
for (let index = 3; index < process.argv.length; index += 1) {
  const argument = process.argv[index];
  if (!argument.startsWith("--"))
    throw new Error("Unexpected argument: " + argument);
  const separator = argument.indexOf("=");
  if (separator !== -1)
    args[argument.slice(2, separator)] = argument.slice(separator + 1);
  else
    args[argument.slice(2)] =
      process.argv[index + 1] && !process.argv[index + 1].startsWith("--")
        ? process.argv[++index]
        : true;
}
const aliases = {
  "sr-ec": "sr-Cyrl",
  "ku-latn": "ku-Latn",
  "skr-arab": "skr-Arab",
  "zh-hans": "zh-Hans",
  "zh-hant": "zh-Hant",
  "pt-br": "pt-BR",
  bcl: "bik",
  diq: "zza",
  iw: "he",
  in: "id",
  ji: "yi",
  no: "nb",
};
export function canonical(code) {
  if (typeof code !== "string") return null;
  const raw = code.replaceAll("_", "-");
  try {
    return Intl.getCanonicalLocales(aliases[raw.toLowerCase()] || raw)[0];
  } catch {
    return null;
  }
}
export function direction(code) {
  try {
    const loc = new Intl.Locale(code);
    const script = loc.maximize().script;
    return [
      "Arab",
      "Hebr",
      "Nkoo",
      "Adlm",
      "Rohg",
      "Syrc",
      "Thaa",
      "Samr",
      "Mand",
    ].includes(script)
      ? "rtl"
      : "ltr";
  } catch {
    return "ltr";
  }
}
const json = async (p) => JSON.parse(await fs.readFile(p, "utf8"));
const save = async (p, data) => {
  await fs.mkdir(path.dirname(p), { recursive: true });
  await fs.writeFile(p, JSON.stringify(data, null, 2) + "\n");
};
const placeholders = (text) =>
  [...text.matchAll(/\{([A-Za-z][\w]*)\}/g)]
    .map((x) => x[1])
    .sort()
    .join(",");
/** Comments apply only to the following assignment, never to a quoted value. */
export function appleStringEntries(text) {
  const result = {};
  const pattern = /\/\/[^\r\n]*|\/\*[\s\S]*?\*\/|"((?:\\.|[^"\\])*)"\s*=\s*"((?:\\.|[^"\\])*)"\s*;/g;
  let fuzzy = false;
  const decode = (value) =>
    JSON.parse('"' + value.replace(/\\U([0-9a-fA-F]{4})/g, "\\u$1") + '"');
  for (const match of text.matchAll(pattern)) {
    if (match[1] === undefined) {
      fuzzy ||= /(?:^|\s)Fuzzy(?:\s|$)/i.test(
        match[0].replace(/^\/\/|^\/\*|\*\/$/g, "").trim(),
      );
      continue;
    }
    result[decode(match[1])] = { value: decode(match[2]), fuzzy };
    fuzzy = false;
  }
  return result;
}
function strings(text) {
  return Object.fromEntries(
    Object.entries(appleStringEntries(text))
      .filter(([, entry]) => !entry.fuzzy)
      .map(([key, entry]) => [key, entry.value]),
  );
}
/** Inspect flat Android strings only; comments/plural items are not candidates. */
export function androidStringMetadata(text) {
  const result = {};
  const pattern = /<!--[\s\S]*?-->|<string(?=\s|>)((?:"[^"]*"|'[^']*'|[^'">])*)>[\s\S]*?<\/string\s*>/g;
  for (const match of text.matchAll(pattern)) {
    if (match[1] === undefined) continue;
    const attributes = Object.fromEntries(
      [...match[1].matchAll(/([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)]
        .map((attribute) => [attribute[1], attribute[2] ?? attribute[3]]),
    );
    if (attributes.name)
      result[attributes.name] = {
        fuzzy: attributes.fuzzy !== undefined &&
          !["false", "0"].includes(attributes.fuzzy),
        translatable: attributes.translatable !== "false",
      };
  }
  return result;
}
export function reusableCommunityEntry(entry) {
  // Missing metadata is not an affirmative non-fuzzy finding.
  return entry.fuzzy === false && entry.sourceFuzzy === false;
}
export function excludeFuzzyInherited(catalogs, provenance, community) {
  const excluded = [];
  const remove = (code, source, value, resource, status) => {
    if (catalogs[code]?.[source] !== value ||
        provenance[code]?.[source] !== resource) return;
    delete catalogs[code][source];
    delete provenance[code][source];
    excluded.push({ code, source, resource, status });
  };
  for (const [code, entries] of Object.entries(community.imports))
    for (const [source, entry] of Object.entries(entries))
      if (!reusableCommunityEntry(entry))
        remove(code, source, entry.value,
          `android:${entry.revision}:${entry.path}#${entry.key}`,
          entry.fuzzy || entry.sourceFuzzy ? "excluded-fuzzy" : "excluded-unverified");
  return excluded;
}
/** Review authority binds exact text/forms, locale, source and application context.
 * Contributor lists or reviewed:true alone are never sufficient evidence.
 */
export function reviewedTranslation(record, code, source, value) {
  const review = record?.review;
  if (record?.reviewed !== true || !review ||
      review.locale !== code || canonical(code) !== code ||
      review.source !== source ||
      !["context", "translator", "reviewer", "competence"].every(
        (key) => typeof review[key] === "string" && review[key].trim(),
      ) || review.translator === review.reviewer ||
      typeof review.evidenceUrl !== "string") return null;
  let url;
  try { url = new URL(review.evidenceUrl); } catch { return null; }
  if (url.protocol !== "https:" || url.username || url.password) return null;
  const reviewedValue = typeof value === "string" ? review.text : review.forms;
  const equal = typeof value === "string"
    ? reviewedValue === value
    : value && typeof value === "object" && reviewedValue &&
      Object.keys(value).length === Object.keys(reviewedValue).length &&
      Object.entries(value).every(([category, text]) => reviewedValue[category] === text);
  return equal ? { status: "reviewed", reviewed: true, review } : null;
}
export function addCatalogTranslation(catalogs, provenance, code, source, translation, resource) {
  if (!source || !translation || typeof source !== "string" ||
      typeof translation !== "string") return;
  catalogs[code] ||= {};
  provenance[code] ||= {};
  const existing = reviewedTranslation(
    provenance[code][source], code, source, catalogs[code][source],
  );
  const incoming = reviewedTranslation(resource, code, source, translation);
  if (!(source in catalogs[code]) || (incoming && !existing)) {
    catalogs[code][source] = translation;
    provenance[code][source] = resource;
  }
}
/** Android BCP 47 resource qualifiers preserve script/region distinctions. */
export function androidLocaleQualifier(code) {
  const normalized = canonical(code);
  if (!normalized) throw new RangeError("Invalid Android resource locale: " + code);
  return "values-b+" + normalized.replaceAll("-", "+");
}
/** Quote Android resource values as well as escaping their XML text node. */
export function androidStringResource(text) {
  return '"' + text
    .replaceAll("\\", "\\\\")
    .replaceAll('"', '\\"')
    .replaceAll("'", "\\'")
    .replaceAll("\n", "\\n")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;") + '"';
}
export function translationPlaceholderToken(index) {
  return "https://kiwix.invalid/" + (93847000 + index);
}
export function restoreTranslationPlaceholders(text, names) {
  return text.replace(
    /https:\/\/kiwix\.invalid\/(93847\d{3})(?!\d)/g,
    (token, identifier) => {
      const name = names[Number(identifier) - 93847000];
      return name === undefined ? token : "{" + name + "}";
    },
  );
}
const widgetSources = {
  widget_name: "Kiwix search",
  widget_search_hint: "Search your offline library",
  widget_no_archives: "Your library is empty",
  widget_library: "Library",
  widget_get_content: "Get content",
  widget_microphone: "Voice search",
  widget_bookmarks: "Bookmarks",
};
async function walk(dir) {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  return (
    await Promise.all(
      entries.map((e) =>
        e.isDirectory()
          ? walk(path.join(dir, e.name))
          : [path.join(dir, e.name)],
      ),
    )
  ).flat();
}
/** Library names shown verbatim; a product name is not translatable copy. */
const productNames = new Set(["Zimru", "zimfile-xapian"]);
export async function inventory(directory = path.join(root, "src")) {
  const sources = new Set();
  for (const file of await fs.readdir(out).catch(() => []))
    if (/^extra-en-.*\.json$/.test(file))
      for (const key of Object.keys(await json(path.join(out, file))))
        sources.add(key);
  // Syntax-aware extraction, not a fragile source-text test. Inventories remain usable before npm install.
  let ts;
  try {
    ts = await import("typescript");
  } catch {
    throw new Error(
      "Install frontend dependencies before collecting UI t() literals.",
    );
  }
  const dynamic = [];
  const untranslated = [];
  const invalidCounts = [];
  for (const file of await walk(directory)) {
    if (!/\.(tsx?|jsx?)$/.test(file)) continue;
    const source = ts.createSourceFile(
      file,
      await fs.readFile(file, "utf8"),
      ts.ScriptTarget.Latest,
      true,
      file.endsWith("tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    );
    function visit(node) {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        ["t", "tPlural"].includes(node.expression.text)
      ) {
        const first = node.arguments[0];
        if (
          first &&
          (ts.isStringLiteral(first) ||
            ts.isNoSubstitutionTemplateLiteral(first))
        )
          sources.add(first.text);
        else
          dynamic.push(
            path.relative(root, file) +
              ":" +
              (source.getLineAndCharacterOfPosition(node.getStart()).line + 1),
          );
        if (
          first &&
          ts.isStringLiteral(first) &&
          first.text.includes("{count}")
        ) {
          const values = node.arguments[1];
          let count;
          if (node.expression.text === "tPlural") count = values;
          else if (values && ts.isObjectLiteralExpression(values)) {
            const property = values.properties.find(
              (property) =>
                (ts.isPropertyAssignment(property) ||
                  ts.isShorthandPropertyAssignment(property)) &&
                property.name.text === "count",
            );
            if (property)
              count = ts.isShorthandPropertyAssignment(property)
                ? property.name
                : property.initializer;
          }
          if (
            !count ||
            ts.isStringLiteral(count) ||
            (ts.isCallExpression(count) &&
              count.expression.getText(source) === "formatNumber")
          )
            invalidCounts.push(
              path.relative(root, file) +
                ":" +
                (source.getLineAndCharacterOfPosition(node.getStart()).line +
                  1),
            );
        }
      }
      const location =
        path.relative(root, file) +
        ":" +
        (source.getLineAndCharacterOfPosition(node.getStart()).line + 1);
      if (
        ts.isJsxText(node) &&
        /\p{L}/u.test(node.text.trim()) &&
        !productNames.has(node.text.trim()) &&
        node.parent?.openingElement?.tagName?.getText(source) !== "kbd"
      )
        untranslated.push({ location, text: node.text.trim() });
      if (
        ts.isJsxAttribute(node) &&
        ["title", "placeholder", "aria-label", "alt", "label"].includes(
          node.name.getText(source),
        ) &&
        node.initializer &&
        ts.isStringLiteral(node.initializer) &&
        node.initializer.text.trim()
      )
        untranslated.push({ location, text: node.initializer.text });
      ts.forEachChild(node, visit);
    }
    const reported = [dynamic.length, untranslated.length, invalidCounts.length];
    visit(source);
    // The catalog implementation forwards its callers' sources to t(); only its
    // own literal copy is inventoried, so generation keeps it in the catalogs.
    if (file.endsWith("i18n.tsx"))
      [dynamic.length, untranslated.length, invalidCounts.length] = reported;
  }
  return { sources: [...sources].sort(), dynamic, untranslated, invalidCounts };
}
/** UI copy the generated manifest does not account for. A source that only lacks a
 * translation is not reported here: it falls back to English by documented policy.
 */
export function inventoryFailures(manifest, { sources, dynamic, untranslated }) {
  const collected = new Set(manifest.sources);
  return [
    ...sources.filter((key) => !collected.has(key)).map((key) => "Uncollected UI key: " + key),
    ...dynamic.map((location) => "Dynamic t() source cannot be inventoried: " + location),
    ...untranslated.map((entry) => "Untranslated JSX at " + entry.location + ": " + entry.text),
  ];
}
async function generate() {
  const previous = await json(path.join(out, "manifest.json"));
  const refresh = Boolean(args["qt-root"] || args["apple-root"]);
  if (
    refresh &&
    (typeof args["qt-root"] !== "string" ||
      typeof args["apple-root"] !== "string")
  )
    throw new Error(
      "An inherited refresh requires explicit --qt-root and --apple-root paths.",
    );
  const qtRoot = refresh ? path.resolve(root, args["qt-root"]) : null;
  const appleRoot = refresh ? path.resolve(root, args["apple-root"]) : null;
  const qtEnglish = refresh ? await json(path.join(qtRoot, "en.json")) : {};
  const appleEnglish = refresh
    ? strings(
        await fs.readFile(
          path.join(appleRoot, "Support/en.lproj/Localizable.strings"),
          "utf8",
        ),
      )
    : {};
  const catalogs = refresh ? {} : await json(path.join(out, "inherited.json"));
  const provenance = {};
  const community = await json(path.join(out, "community-provenance.json"));
  const bundledCatalogs = refresh
    ? await json(path.join(out, "inherited.json"))
    : catalogs;
  const inheritedProvenance = await json(
    path.join(out, "inherited-provenance.json"),
  );
  const names = {};
  const origin = {};
  const resourceNames = {};
  if (!refresh) {
    // Historical origin labels are attribution, not a linguistic review.
    for (const locale of previous.locales) {
      names[locale.code] = locale.name;
      origin[locale.code] = locale.resources;
      resourceNames[locale.code] = locale.resourceName;
      provenance[locale.code] = Object.fromEntries(
        Object.keys(catalogs[locale.code] || {}).map((source) => [
          source,
          inheritedProvenance[locale.code]?.[source] || "inherited",
        ]),
      );
    }
  }
  // Keep only evidence-bound explicit reviews across refresh; community reimports
  // cannot overwrite them. No checked-in reviews are inferred or manufactured.
  if (refresh)
    for (const [code, entries] of Object.entries(bundledCatalogs))
      for (const [source, value] of Object.entries(entries)) {
        const review = reviewedTranslation(
          inheritedProvenance[code]?.[source], code, source, value,
        );
        if (review) {
          catalogs[code] ||= {};
          provenance[code] ||= {};
          catalogs[code][source] = value;
          provenance[code][source] = review;
        }
      }
  const excludedCommunity = refresh
    ? []
    : excludeFuzzyInherited(catalogs, provenance, community);
  const add = (code, source, translation, resource) =>
    addCatalogTranslation(catalogs, provenance, code, source, translation, resource);
  const qtFiles = refresh
    ? (await fs.readdir(qtRoot))
        .filter((f) => f.endsWith(".json") && f !== "qqq.json")
        .sort()
    : [];
  for (const file of qtFiles) {
    const raw = file.slice(0, -5);
    const code = canonical(raw);
    const values = await json(path.join(qtRoot, file));
    if (!code) throw new Error("Invalid Qt locale: " + raw);
    resourceNames[code] = values.name || raw;
    const incorrectEnglishNames = {
      bik: "Bikol",
      dag: "Dagbanli",
      igl: "Igala",
    };
    const needsAutonym =
      resourceNames[code] === raw ||
      [
        "bik",
        "dag",
        "fa",
        "ha",
        "ig",
        "igl",
        "ja",
        "nb",
        "pa",
        "yi",
        "yo",
      ].includes(code);
    let autonym;
    try {
      autonym = new Intl.DisplayNames([code], { type: "language" }).of(code);
    } catch {
      autonym = code;
    }
    names[code] = needsAutonym
      ? incorrectEnglishNames[code] || autonym
      : resourceNames[code];
    (origin[code] ||= []).push("Qt:" + raw);
    for (const [key, source] of Object.entries(qtEnglish))
      if (key !== "name" && key !== "@metadata")
        add(code, source, values[key], "qt");
  }
  const appleDirs = refresh
    ? (await fs.readdir(path.join(appleRoot, "Support")))
        .filter((f) => f.endsWith(".lproj") && f !== "qqq.lproj")
        .sort()
    : [];
  for (const dir of appleDirs) {
    const raw = dir.slice(0, -6);
    const code = canonical(raw);
    const file = path.join(appleRoot, "Support", dir, "Localizable.strings");
    let values;
    try {
      values = appleStringEntries(await fs.readFile(file, "utf8"));
    } catch (error) {
      if (error.code === "ENOENT") continue;
      throw error;
    }
    if (!code) throw new Error("Invalid Apple locale: " + raw);
    if (!names[code]) {
      try {
        names[code] = new Intl.DisplayNames([code], { type: "language" }).of(
          code,
        );
      } catch {
        names[code] = raw;
      }
    }
    (origin[code] ||= []).push("Apple:" + raw);
    for (const [key, source] of Object.entries(appleEnglish)) {
      if (values[key]?.fuzzy) {
        excludedCommunity.push({
          code, source, resource: `apple:${dir}#${key}`, status: "excluded-fuzzy",
        });
        continue;
      }
      add(code, source, values[key]?.value, "apple");
    }
  }
  if (refresh) {
    // Pinned metadata is offline input; no translation-service requests or retries.
    // Refreshed Qt/Apple translations win; pinned Android imports fill exact-source gaps.
    for (const [code, entries] of Object.entries(community.imports)) {
      for (const [source, entry] of Object.entries(entries)) {
        if (!reusableCommunityEntry(entry)) {
          excludedCommunity.push({
            code, source, resource: entry.url,
            status: entry.fuzzy || entry.sourceFuzzy ? "excluded-fuzzy" : "excluded-unverified",
          });
          continue;
        }
        if (source in (catalogs[code] || {})) continue;
        add(
          code,
          source,
          entry.value,
          `android:${entry.revision}:${entry.path}#${entry.key}`,
        );
        const qualifier = entry.path.split("/").at(-2).slice("values-".length);
        const resource = "Android:" + qualifier;
        const resources = (origin[code] ||= []);
        if (!resources.includes(resource)) resources.push(resource);
      }
    }
  }
  // Default generation is self-contained; external checkouts are read only on explicit refresh.
  // Persist values and their origins together so excluded entries cannot return
  // on the next offline generation under a generic "inherited" origin.
  await save(path.join(out, "inherited.json"), catalogs);
  await save(path.join(out, "inherited-provenance.json"), provenance);
  const { sources, dynamic, untranslated } = await inventory();
  catalogs.en ||= {};
  provenance.en ||= {};
  for (const source of sources) {
    catalogs.en[source] = source;
    provenance.en[source] ||= "source";
  }
  const machine = await json(path.join(out, "machine.json")).catch(() => ({}));
  for (const [code, translations] of Object.entries(machine))
    for (const [source, record] of Object.entries(translations)) {
      if (
        !(code in names) ||
        !sources.includes(source) ||
        !record.text ||
        placeholders(source) !== placeholders(record.text)
      )
        continue;
      add(
        code,
        source,
        record.text,
        reviewedTranslation(record, code, source, record.text) ||
          (record.kind === "proper-name" ? "proper-name" : "machine"),
      );
    }
  const englishPlurals = await json(path.join(out, "plural-sources.json"));
  const pluralTranslations = await json(
    path.join(out, "plural-translations.json"),
  );
  const pluralCoverage = {};
  for (const code of Object.keys(names)) {
    const categories = new Intl.PluralRules(code).resolvedOptions()
      .pluralCategories;
    pluralCoverage[code] = {
      categories,
      translated: 0,
      total: Object.keys(englishPlurals).length,
    };
    for (const [source, englishForms] of Object.entries(englishPlurals)) {
      const existingForms = catalogs[code]?.[source];
      const existingReview = code !== "en" && existingForms &&
        typeof existingForms === "object" &&
        reviewedTranslation(provenance[code]?.[source], code, source, existingForms);
      const forms =
        code === "en"
          ? englishForms
          : existingReview ? existingForms : pluralTranslations[code]?.[source]?.forms;
      if (!forms) {
        // A flat inherited plural is not a translation of every cardinal category.
        delete catalogs[code]?.[source];
        delete provenance[code]?.[source];
        continue;
      }
      for (const category of categories) {
        if (
          typeof forms[category] !== "string" ||
          !forms[category].trim() ||
          placeholders(forms[category]) !== placeholders(source)
        )
          throw new Error(`Invalid ${code}/${category} plural form: ${source}`);
      }
      catalogs[code] ||= {};
      provenance[code] ||= {};
      catalogs[code][source] = Object.fromEntries(
        categories.map((category) => [category, forms[category]]),
      );
      provenance[code][source] =
        code === "en" ? "source-plural" :
          reviewedTranslation(existingReview || pluralTranslations[code]?.[source],
            code, source, catalogs[code][source]) ||
          "machine-plural";
      pluralCoverage[code].translated += 1;
    }
  }
  const locales = Object.keys(names)
    .sort()
    .map((code) => ({
      code,
      name: names[code],
      resourceName: resourceNames[code] || names[code],
      direction: direction(code),
      coverage:
        code === "en"
          ? 1
          : sources.length
            ? sources.filter((s) => Object.hasOwn(catalogs[code] || {}, s))
                .length / sources.length
            : 0,
      machineTranslated: Object.values(provenance[code] || {}).some(
        (value) => value === "machine" || value === "machine-plural",
      ),
      resources: origin[code],
    }));
  const missingNew = Object.fromEntries(
    locales
      .filter((l) => l.code !== "en")
      .map((l) => [
        l.code,
        sources.filter((s) => !Object.hasOwn(catalogs[l.code] || {}, s)),
      ])
      .filter(([, keys]) => keys.length),
  );
  const activeSources = new Set(sources);
  const runtimeCatalogs = Object.fromEntries(
    Object.entries(catalogs).map(([code, values]) => [
      code,
      Object.fromEntries(
        Object.entries(values).filter(([source]) => activeSources.has(source)),
      ),
    ]),
  );
  await save(path.join(out, "catalogs.json"), runtimeCatalogs);
  for (const [code, messages] of Object.entries(runtimeCatalogs))
    await save(path.join(out, "messages", code + ".json"), messages);
  // macOS system permission prompts read InfoPlist.strings, not the web catalog.
  const purposeSources = {
    NSCameraUsageDescription:
      "Only this active archive can request the camera. Your system may also ask for permission. You can revoke access below the article at any time.",
    NSLocationUsageDescription:
      "Your system will provide your location only to this active archive. You can revoke access below the article at any time.",
    NSLocationWhenInUseUsageDescription:
      "Your system will provide your location only to this active archive. You can revoke access below the article at any time.",
    NSLocalNetworkUsageDescription:
      "Discover nearby LocalSwarm devices and share offline archives on your local network. Enable discovery or sharing only on a trusted network.",
  };
  for (const [code, messages] of Object.entries(runtimeCatalogs)) {
    const directory = path.join(
      root,
      "src-tauri",
      "native-locales",
      code + ".lproj",
    );
    await fs.mkdir(directory, { recursive: true });
    const text =
      Object.entries(purposeSources)
        .map(
          ([key, source]) =>
            `${JSON.stringify(key)} = ${JSON.stringify(typeof messages[source] === "string" ? messages[source] : source)};`,
        )
        .join("\n") + "\n";
    await fs.writeFile(path.join(directory, "InfoPlist.strings"), text);
  }
  // Android widgets run without the WebView; bundle the same attributed catalog.
  // Missing keys deliberately inherit the base English resource, never a fake translation.
  for (const [code, messages] of Object.entries(runtimeCatalogs)) {
    if (code === "en") continue;
    const entries = Object.entries(widgetSources).filter(
      ([, source]) => typeof messages[source] === "string",
    );
    const directory = path.join(
      root, "src-tauri", "plugins", "tauri-plugin-kiwix-mobile",
      "android", "src", "main", "res", androidLocaleQualifier(code),
    );
    await fs.mkdir(directory, { recursive: true });
    await fs.writeFile(
      path.join(directory, "kiwix_widget_strings.xml"),
      '<?xml version="1.0" encoding="utf-8"?>\n'
        + "<!-- Generated by scripts/localization.mjs; provenance: src/locales/provenance.json. -->\n"
        + "<resources>\n"
        + entries.map(([key, source]) =>
          `  <string name="${key}" formatted="false">${androidStringResource(messages[source])}</string>`,
        ).join("\n") + "\n</resources>\n",
    );
  }
  const androidXml = path.join(
    root, "src-tauri", "plugins", "tauri-plugin-kiwix-mobile",
    "android", "src", "main", "res", "xml",
  );
  await fs.mkdir(androidXml, { recursive: true });
  await fs.writeFile(
    path.join(androidXml, "kiwix_locales_config.xml"),
    '<?xml version="1.0" encoding="utf-8"?>\n'
      + '<!-- Generated by scripts/localization.mjs from the bundled locale index. -->\n'
      + '<locale-config xmlns:android="http://schemas.android.com/apk/res/android">\n'
      + locales.map(({ code }) => `  <locale android:name="${code}" />`).join("\n")
      + "\n</locale-config>\n",
  );
  await save(path.join(out, "locale-index.json"), {
    aliases,
    locales: locales.map(({ code, name, coverage, machineTranslated }) => ({
      code,
      name,
      coverage,
      machineTranslated,
    })),
  });
  await save(path.join(out, "manifest.json"), {
    schema: 2,
    aliases,
    locales,
    sources,
    dynamic,
    untranslated,
    missingNew,
    pluralCoverage,
    policy:
      "Qt and Kiwix Apple community translations are preserved by exact English source match; Qt wins duplicate sources, followed by non-fuzzy Apple entries. Pinned non-fuzzy Android community imports in community-provenance.json fill remaining exact-source gaps and survive explicit refresh with their per-message provenance. Community attribution is not language competence or context review. Explicit reviewed input must bind the canonical locale, exact English source, exact text/forms, application context, translator, separate reviewer, competence and public HTTPS evidence; it takes precedence over unreviewed imports on every generation/refresh. A reviewed flag alone grants no authority. qqq is documentation, not a runtime locale. Default generation uses bundled inherited data and locale metadata only, excluding pinned fuzzy imports. Missing translations explicitly fall back to English. Count messages require every Intl.PluralRules cardinal category for the locale; flat inherited plurals are not reported as category-complete. Build-time machine translations identify their service in machine.json and plural-translations.json and remain unreviewed unless separately attributable review evidence is supplied. Coverage measures UI source availability, not linguistic quality. Intl formatting uses the host ICU data. StreetZim maps run on Windows, macOS and Linux. No runtime translation requests.",
    upstream: {
      ...previous.upstream,
      qt: "../kiwix-desktop/resources/i18n/*.json (explicit refresh only)",
      apple:
        "../kiwix-apple/Support/*.lproj/Localizable.strings (explicit refresh only)",
      bundled: "src/locales/inherited.json",
      qtResourceCountIncludingQqq: refresh
        ? qtFiles.length + 1
        : previous.upstream.qtResourceCountIncludingQqq,
      appleResourceCountIncludingQqq: refresh
        ? appleDirs.length + 1
        : previous.upstream.appleResourceCountIncludingQqq,
    },
  });
  await save(path.join(out, "provenance.json"), provenance);
  const acquisitionErrors = await json(
    path.join(out, "translation-errors.json"),
  ).catch(() => ({}));
  const modelAvailability = await json(
    path.join(out, "model-availability.json"),
  ).catch(() => ({}));
  await save(path.join(out, "translation-status.json"), {
    supportedLocales: locales.length,
    sourceCount: sources.length,
    pluralCoverage,
    excludedCommunity,
    runtime:
      "English messages and locale-index are eager; all other messages/<BCP47>.json files are bundled as offline dynamic-import chunks. English is used while the selected chunk loads. whenLocaleReady() rejects failures; the provider displays their technical details. DOM data-locale-ready is true only after the requested catalog is present.",
    sharedInterface:
      "Rust embeds these same per-locale messages and compact locale metadata. Parsed catalogs and ICU cardinal/decimal formatters are cached; sharing reads current app locale per request, negotiates weighted Accept-Language only for system, emits lang/dir, substitutes plain values once, then escapes the complete HTML text. Sharing has no online translation or script/privileged application bridge.",
    completeLocales: locales
      .filter((locale) => locale.coverage === 1)
      .map((locale) => locale.code),
    incompleteLocales: locales
      .filter((locale) => locale.coverage < 1)
      .map((locale) => ({
        code: locale.code,
        name: locale.name,
        coverage: locale.coverage,
        missingCount: missingNew[locale.code]?.length || 0,
        serviceError: acquisitionErrors[locale.code] || null,
        modelDecline:
          modelAvailability.strongerModelDeclinedLocales?.[locale.code] ||
          modelAvailability.defaultModelDeclinedLocales?.[locale.code] ||
          null,
      })),
    reviewStatus:
      "Machine output and community attribution do not establish competent language review. Explicit reviewed provenance requires exact locale/source/text-or-forms, application context, translator, separate reviewer, competence and public evidence. No review evidence is inferred from authors or reviewed flags. Fuzzy imports are excluded and reported in excludedCommunity; pinned Android flags and license/source evidence are recorded in community-provenance.json. Incomplete locales remain partial and explicitly fall back to English for missing keys; fully internationalized does not mean fully localized.",
  });
  console.log(
    JSON.stringify(
      {
        locales: locales.length,
        uiSources: sources.length,
        missingNew: Object.fromEntries(
          Object.entries(missingNew).map(([code, keys]) => [code, keys.length]),
        ),
        dynamic,
      },
      null,
      2,
    ),
  );
}
async function translate() {
  const manifest = await json(path.join(out, "manifest.json"));
  const catalogs = await json(path.join(out, "catalogs.json"));
  const machine = await json(path.join(out, "machine.json")).catch(() => ({}));
  const pluralSources = new Set(
    Object.keys(await json(path.join(out, "plural-sources.json"))),
  );
  const requested =
    typeof args.locales === "string"
      ? args.locales.split(",").map(canonical)
      : Object.keys(manifest.missingNew);
  const serviceCodes = {
    "zh-Hans": "zh-CN",
    "zh-Hant": "zh-TW",
    "sr-Cyrl": "sr",
    "ku-Latn": "ku",
    "skr-Arab": "skr",
    "pt-BR": "pt",
  };
  const errors = await json(path.join(out, "translation-errors.json")).catch(
    () => ({}),
  );
  let persist = Promise.resolve();
  const jobs = requested.map((code) => async () => {
    const missing = manifest.sources.filter(
      (s) => !pluralSources.has(s) && !Object.hasOwn(catalogs[code] || {}, s),
    );
    if (!missing.length || code === "en") return;
    machine[code] ||= {};
    try {
      const acquired = {};
      const translatedLines = {};
      const lines = [
        ...new Set(
          missing.flatMap((source) => source.split("\n")).filter(Boolean),
        ),
      ];
      const batches = [];
      let batch = [];
      let length = 0;
      for (const source of lines) {
        if (length + source.length > 1400 && batch.length) {
          batches.push(batch);
          batch = [];
          length = 0;
        }
        batch.push(source);
        length += source.length + 10;
      }
      if (batch.length) batches.push(batch);
      for (const batch of batches) {
        const names = batch.map((source) => [
          ...new Set(
            [...source.matchAll(/\{([A-Za-z][\w]*)\}/g)].map((m) => m[1]),
          ),
        ]);
        // Bare numeric sentinels may acquire locale digit grouping or numerals.
        // An inert reserved-domain URL stays opaque to translation; restore only
        // exact tokens and still require the original placeholder multiset.
        if (batch.some((source, i) =>
          names[i].some((_, index) => source.includes(translationPlaceholderToken(index)))))
          throw new Error("Translation placeholder marker collides with source text");
        const marked = batch
          .map((source, i) =>
            source.replace(
              /\{([A-Za-z][\w]*)\}/g,
              (_, name) => translationPlaceholderToken(names[i].indexOf(name)),
            ),
          )
          .join("\n");
        const url = new URL(
          "https://translate.googleapis.com/translate_a/single",
        );
        for (const [key, value] of Object.entries({
          client: "gtx",
          sl: "en",
          tl: serviceCodes[code] || code,
          dt: "t",
          q: marked,
        }))
          url.searchParams.set(key, value);
        const response = await fetch(url, {
          signal: AbortSignal.timeout(45000),
        });
        if (!response.ok)
          throw new Error("Google Translate HTTP " + response.status);
        const result = await response.json();
        if (
          result[0]
            ?.map((s) => s[1])
            .join("")
            .trim() !== marked.trim()
        )
          throw new Error("Translation source boundaries were not preserved");
        const text = result[0]?.map((s) => s[0]).join("");
        if (typeof text !== "string" || !text.trim())
          throw new Error("No translation returned");
        const pieces = text.trim().split("\n");
        if (pieces.length !== batch.length)
          throw new Error("Translation line boundaries were not preserved");
        for (const [index, piece] of pieces.entries()) {
          const translated =
            batch[index].match(/^\s*/)[0] +
            restoreTranslationPlaceholders(piece.trim(), names[index]) +
            batch[index].match(/\s*$/)[0];
          if (
            !translated ||
            placeholders(translated) !== placeholders(batch[index])
          )
            throw new Error("Placeholder mismatch for " + batch[index]);
          translatedLines[batch[index]] = translated;
        }
      }
      for (const source of missing)
        acquired[source] = {
          text: source
            .split("\n")
            .map((line) => (line ? translatedLines[line] : ""))
            .join("\n"),
          service: "Google Translate public endpoint",
          reviewed: false,
        };
      if (
        missing.length > 10 &&
        missing.filter((source) => acquired[source].text === source).length /
          missing.length >
          0.8
      )
        throw new Error(
          "Service echoed English instead of translating this locale",
        );
      Object.assign(machine[code], acquired);
      persist = persist.then(() =>
        save(path.join(out, "machine.json"), machine),
      );
      await persist;
      delete errors[code];
    } catch (error) {
      errors[code] = String(error.message || error);
    }
    console.log(code + ": " + (errors[code] || "acquired " + missing.length));
  });
  const concurrency = Math.max(1, Math.min(8, Number(args.concurrency) || 4));
  let next = 0;
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (next < jobs.length) await jobs[next++]();
    }),
  );
  await save(path.join(out, "translation-errors.json"), errors);
  await generate();
}
async function audit() {
  const manifest = await json(path.join(out, "manifest.json"));
  const catalogs = await json(path.join(out, "catalogs.json"));
  const found = await inventory();
  const { sources, invalidCounts } = found;
  const failures = inventoryFailures(manifest, found);
  for (const location of invalidCounts)
    failures.push(
      "Plural messages require an unformatted numeric count: " + location,
    );
  for (const locale of manifest.locales) {
    if (canonical(locale.code) !== locale.code)
      failures.push("Noncanonical locale: " + locale.code);
    if (locale.direction !== direction(locale.code))
      failures.push("Incorrect direction: " + locale.code);
    for (const source of sources) {
      const text = catalogs[locale.code]?.[source];
      if (text === undefined)
        failures.push(locale.code + ": missing new copy: " + source);
      else if (typeof text === "string") {
        if (placeholders(source) !== placeholders(text))
          failures.push(locale.code + ": placeholder mismatch: " + source);
      } else {
        for (const category of new Intl.PluralRules(
          locale.code,
        ).resolvedOptions().pluralCategories) {
          if (typeof text[category] !== "string")
            failures.push(
              locale.code + ": missing plural " + category + ": " + source,
            );
          else if (placeholders(source) !== placeholders(text[category]))
            failures.push(
              locale.code + ": plural placeholder mismatch: " + source,
            );
        }
      }
    }
  }
  if (args["dom-snapshot"]) {
    const snapshot = await json(args["dom-snapshot"]);
    const code = canonical(snapshot.lang);
    if (!code || !manifest.locales.some((l) => l.code === code))
      failures.push("DOM lang is not a supported BCP 47 locale");
    if (snapshot.dir !== direction(code || "en"))
      failures.push("DOM dir does not match lang");
  }
  console.log(
    JSON.stringify(
      {
        supportedLocales: manifest.locales.length,
        sources: sources.length,
        failures,
      },
      null,
      2,
    ),
  );
  if (failures.length) process.exitCode = 1;
}
async function checkInventory() {
  const failures = inventoryFailures(
    await json(path.join(out, "manifest.json")), await inventory(),
  );
  console.log(JSON.stringify({ failures }, null, 2));
  if (failures.length) {
    console.error(
      "The localization inventory is out of date or UI copy bypasses it. For uncollected keys run `node scripts/localization.mjs generate` and commit the result.",
    );
    process.exitCode = 1;
  }
}
const command = process.argv[2];
if (command === "generate") await generate();
else if (command === "translate") await translate();
else if (command === "audit") await audit();
else if (command === "inventory") await checkInventory();
else if (process.argv[1] === fileURLToPath(import.meta.url)) {
  console.error("Usage: localization.mjs generate|translate|audit|inventory");
  process.exitCode = 2;
}
