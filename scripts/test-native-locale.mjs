import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Execute the production locale store. Replace only Vite's build-time glob macro;
// React rendering is outside this preference/OS consumer regression boundary.
function localeStore(languages = ["en"]) {
  const filename = new URL("../src/i18n.tsx", import.meta.url);
  const require = createRequire(filename);
  const metadata = require("./locales/locale-index.json");
  const loaders = Object.fromEntries(metadata.locales.map(({ code }) => [
    `./locales/messages/${code}.json`, () => Promise.resolve({}),
  ]));
  const result = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
    transformers: { before: [(context) => (source) => {
      const visit = (node) => {
        if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === "glob" && ts.isMetaProperty(node.expression.expression)) {
          return context.factory.createIdentifier("__localeLoaders");
        }
        return ts.visitEachChild(node, visit, context);
      };
      return ts.visitNode(source, visit);
    }] },
  });
  const exports = {};
  vm.runInNewContext(result.outputText, {
    exports,
    require: (name) => name === "react" ? { createContext: () => ({}) }
      : name === "react/jsx-runtime" ? {}
      : name === "./byte-units" ? require("./byte-units.ts")
      : require(name),
    navigator: { languages, language: languages[0] },
    __localeLoaders: loaders,
    Intl,
  }, { filename: filename.pathname });
  return exports;
}

test("OS per-app locale overrides stale WebView system language without replacing preference", async () => {
  const locale = localeStore(["en"]);
  locale.setSystemLocale("de-DE");
  assert.equal(locale.getLocale(), "de");
  locale.setLocale("fr");
  assert.equal(locale.getLocale(), "fr");
  locale.setSystemLocale("ar");
  assert.equal(locale.getLocale(), "fr", "explicit user language wins");
  locale.setLocale("system");
  assert.equal(locale.getLocale(), "ar", "system remains a preference, not a saved resolved locale");
  assert.equal(locale.localeDirection(), "rtl");
  await locale.whenLocaleReady();
});

test("clearing native per-app locale restores navigator matching and respects Chinese script", async () => {
  const locale = localeStore(["fr-CA", "en"]);
  locale.setSystemLocale("zh-TW");
  assert.equal(locale.getLocale(), "zh-Hant");
  locale.setSystemLocale(null);
  assert.equal(locale.getLocale(), "fr");
  locale.setSystemLocale("not a locale");
  assert.equal(locale.getLocale(), "fr");
  await locale.whenLocaleReady();
});

test("byte sizes never print 1000 of the smaller unit", () => {
  const locale = localeStore(["en"]);
  assert.equal(locale.formatBytes(0), "0 byte");
  assert.equal(locale.formatBytes(999), "999 byte");
  assert.equal(locale.formatBytes(999_999), "1 MB");
  assert.equal(locale.formatBytes(999_950_000), "1 GB");
  assert.equal(locale.formatBytes(1_000_000), "1 MB");
});
