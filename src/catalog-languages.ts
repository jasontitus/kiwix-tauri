export interface CatalogLanguageFacet {
  value: string;
}

function displayLanguage(code: string, locale: string): string {
  try {
    const normalized = Intl.getCanonicalLocales(code)[0];
    if (!normalized) return code;
    return new Intl.DisplayNames([locale], {
      type: "language",
      fallback: "code",
    }).of(normalized) || code;
  } catch {
    return code;
  }
}

/** Match device BCP-47 languages to catalog language facet codes. */
export function deviceCatalogLanguages(
  deviceLanguages: readonly string[],
  facets: readonly CatalogLanguageFacet[],
  locale: string,
): string[] {
  const labels = new Set<string>();
  for (const value of deviceLanguages) {
    try {
      const language = new Intl.Locale(value).language;
      if (language) labels.add(displayLanguage(language, locale).toLocaleLowerCase(locale));
    } catch {
      // Ignore malformed device locales rather than selecting an arbitrary one.
    }
  }
  if (!labels.size) return [];
  const selected = facets
    .filter(({ value }) =>
      value
        .split(/[,;]/)
        .map((code) => code.trim())
        .filter(Boolean)
        .some((code) => labels.has(displayLanguage(code, locale).toLocaleLowerCase(locale))),
    )
    .map(({ value }) => value);
  return [...new Set(selected)];
}
