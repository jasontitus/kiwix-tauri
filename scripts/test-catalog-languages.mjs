import assert from "node:assert/strict";
import test from "node:test";
import { deviceCatalogLanguages } from "../src/catalog-languages.ts";

const facets = [
  { value: "eng" },
  { value: "fra" },
  { value: "zho" },
  { value: "por" },
  { value: "spa" },
  { value: "deu" },
];

test("device languages select matching catalog facet codes", () => {
  assert.deepEqual(
    deviceCatalogLanguages(["en-US", "fr-FR"], facets, "en"),
    ["eng", "fra"],
  );
});

test("base languages match script and region variants", () => {
  assert.deepEqual(deviceCatalogLanguages(["zh-Hant"], facets, "en"), ["zho"]);
  assert.deepEqual(deviceCatalogLanguages(["pt-BR"], facets, "en"), ["por"]);
});

test("two-letter facet codes and multi-language facets are supported", () => {
  assert.deepEqual(
    deviceCatalogLanguages(["en-GB", "es-ES"], [{ value: "en" }, { value: "por,spa" }], "en"),
    ["en", "por,spa"],
  );
});

test("no match leaves the catalog unfiltered", () => {
  assert.deepEqual(deviceCatalogLanguages(["xx-XX"], facets, "en"), []);
  assert.deepEqual(deviceCatalogLanguages([], facets, "en"), []);
  assert.deepEqual(deviceCatalogLanguages(["not a locale"], facets, "en"), []);
});

test("duplicate device languages produce one facet selection", () => {
  assert.deepEqual(
    deviceCatalogLanguages(["en-US", "en-GB"], [{ value: "eng" }], "en"),
    ["eng"],
  );
});
