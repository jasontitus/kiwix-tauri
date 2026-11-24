/// <reference types="vite/client" />
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import englishMessages from "./locales/messages/en.json";
import manifestData from "./locales/locale-index.json";
import { BYTE_UNITS, byteUnitIndex } from "./byte-units";

export interface SupportedLocale {
  code: string;
  name: string;
  /** Fraction of current UI sources available; not a linguistic quality rating. */
  coverage: number;
  /** Contains build-time machine translations; these have not been native-reviewed. */
  machineTranslated: boolean;
}
interface LocaleContextValue {
  /** Selected preference, including "system"; formatting uses the resolved locale. */
  locale: string;
  setLocale: (locale: string) => void;
  locales: SupportedLocale[];
}
type PluralForms = Partial<Record<Intl.LDMLPluralRule, string>> & {
  other: string;
};
type Messages = Record<string, string | PluralForms>;
const catalogs: Record<string, Messages> = { en: englishMessages };
// Vite emits these as local build chunks; changing language never contacts a translation service.
const localeLoaders = import.meta.glob<Messages>(
  ["./locales/messages/*.json", "!./locales/messages/en.json"],
  { import: "default" },
);
const pendingCatalogs = new Map<string, Promise<void>>();
let activeLoad: { locale: string; promise: Promise<void> } | null = null;
let loadError: Error | null = null;
const aliases: Record<string, string> = manifestData.aliases;
export const locales: SupportedLocale[] = manifestData.locales;
const supported = new Set(locales.map((locale) => locale.code));
const subscribers = new Set<() => void>();
let preference = "system";
let revision = 0;
let resolvedLocale: string | null = null;
let nativeSystemLocale: string | null = null;
const byteUnits = BYTE_UNITS;
const LocaleContext = createContext<LocaleContextValue | null>(null);

/** Accept legacy Qt/Apple identifiers as well as canonical BCP 47 tags. */
export function normalizeLocale(value: string): string | null {
  const raw = value.trim().replaceAll("_", "-");
  if (!raw) return null;
  try {
    return (
      Intl.getCanonicalLocales(aliases[raw.toLowerCase()] || raw)[0] || null
    );
  } catch {
    return null;
  }
}
interface LocaleKeys {
  code: string;
  language: string;
  script: string | undefined;
  /** Maximized region, i.e. the CLDR likely region when the tag omits one. */
  region: string | undefined;
  /** Region explicitly present in the tag; undefined when the tag has none. */
  explicitRegion: string | undefined;
}
/** Canonical/maximized keys are derived once; `locales` is a static manifest. */
let localeKeys: LocaleKeys[] | null | undefined;
function localeKeyIndex(): LocaleKeys[] | null {
  if (localeKeys !== undefined) return localeKeys;
  try {
    const keys = locales.map(({ code }) => {
      const tag = new Intl.Locale(code);
      const maximized = tag.maximize();
      return {
        code,
        language: maximized.language,
        script: maximized.script,
        region: maximized.region,
        explicitRegion: tag.region,
      };
    });
    localeKeys = keys;
    return keys;
  } catch {
    localeKeys = null;
    return null;
  }
}
const localeMatches = new Map<string, string | null>();
function matchLocale(value: string): string | null {
  const canonical = normalizeLocale(value);
  if (!canonical) return null;
  if (supported.has(canonical)) return canonical;
  const cached = localeMatches.get(canonical);
  if (cached !== undefined) return cached;
  const keys = localeKeyIndex();
  let match: string | null = null;
  if (keys) {
    try {
      const desired = new Intl.Locale(canonical).maximize();
      // Respect script before region; never serve Simplified Chinese for a Traditional preference.
      let exactRegion: string | null = null;
      let withoutRegion: string | null = null;
      let firstSameScript: string | null = null;
      for (const entry of keys) {
        if (entry.language !== desired.language ||
            entry.script !== desired.script) continue;
        if (!firstSameScript) firstSameScript = entry.code;
        if (!withoutRegion && !entry.explicitRegion) withoutRegion = entry.code;
        if (!exactRegion && entry.region === desired.region)
          exactRegion = entry.code;
      }
      match = exactRegion ?? withoutRegion ?? firstSameScript;
    } catch {
      match = null;
    }
  }
  if (localeMatches.size >= 64) localeMatches.clear();
  localeMatches.set(canonical, match);
  return match;
}
export function getLocale(): string {
  if (resolvedLocale) return resolvedLocale;
  if (preference !== "system") return (resolvedLocale = preference);
  if (nativeSystemLocale) {
    const match = matchLocale(nativeSystemLocale);
    if (match) return (resolvedLocale = match);
  }
  if (typeof navigator !== "undefined") {
    for (const language of navigator.languages) {
      const match = matchLocale(language);
      if (match) return (resolvedLocale = match);
    }
    return (resolvedLocale = matchLocale(navigator.language) || "en");
  }
  return (resolvedLocale = "en");
}
export function localeDirection(code = getLocale()): "ltr" | "rtl" {
  try {
    const script =
      new Intl.Locale(normalizeLocale(code) || "en").maximize().script || "";
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
function updateDocument() {
  if (typeof document === "undefined") return;
  document.documentElement.lang = getLocale();
  document.documentElement.dir = localeDirection();
  document.documentElement.dataset.localeReady = String(
    Boolean(catalogs[getLocale()]),
  );
}
function notify() {
  resolvedLocale = null;
  revision += 1;
  updateDocument();
  for (const subscriber of subscribers) subscriber();
}
/** Start or join the current preference's offline chunk load. Stale completions only populate cache. */
function preloadLocale(): Promise<void> {
  const locale = getLocale();
  if (activeLoad?.locale === locale) return activeLoad.promise;
  loadError = null;
  if (catalogs[locale]) {
    activeLoad = null;
    return Promise.resolve();
  }
  let pending = pendingCatalogs.get(locale);
  if (!pending) {
    const loader = localeLoaders[`./locales/messages/${locale}.json`];
    pending = (
      loader
        ? loader()
        : Promise.reject(new Error(`Missing offline locale chunk: ${locale}`))
    )
      .then((messages) => {
        catalogs[locale] = messages;
      })
      .finally(() => {
        pendingCatalogs.delete(locale);
      });
    pendingCatalogs.set(locale, pending);
  }
  const state = { locale, promise: Promise.resolve() };
  activeLoad = state;
  state.promise = pending.then(
    () => {
      if (activeLoad === state) notify();
    },
    (error) => {
      if (activeLoad === state) {
        activeLoad = null;
        loadError = error instanceof Error ? error : new Error(String(error));
        notify();
      }
      throw error;
    },
  );
  // The provider visibly exposes failures; explicit readiness callers also receive rejection.
  void state.promise.catch(() => undefined);
  return state.promise;
}
/** Resolves only when the selected locale's bundled messages are ready; rejects on chunk failure. */
export function whenLocaleReady(): Promise<void> {
  return preloadLocale();
}
export function setLocale(locale: string): void {
  const next =
    locale.toLowerCase() === "system" ? "system" : matchLocale(locale);
  if (!next) throw new RangeError("Unsupported interface locale: " + locale);
  if (preference === next) {
    void preloadLocale();
    updateDocument();
    return;
  }
  preference = next;
  resolvedLocale = null;
  void preloadLocale();
  notify();
}
/** Native per-app language can differ from WebView navigator.languages until reload.
 * Keep it separate from the user's "system" preference, like native text size. */
export function setSystemLocale(locale: string | null): void {
  const next = locale ? normalizeLocale(locale) : null;
  if (nativeSystemLocale === next) return;
  nativeSystemLocale = next;
  if (preference === "system") {
    resolvedLocale = null;
    void preloadLocale();
    notify();
  }
}
function subscribe(listener: () => void) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}
const snapshot = () => revision;
const serverSnapshot = () => 0;
export function I18nProvider({ children }: { children: ReactNode }) {
  const version = useSyncExternalStore(subscribe, snapshot, serverSnapshot);
  const [dismissedError, setDismissedError] = useState<Error | null>(null);
  useEffect(() => {
    void preloadLocale();
    updateDocument();
    const onLanguageChange = () => {
      if (preference === "system") {
        resolvedLocale = null;
        void preloadLocale();
        notify();
      }
    };
    window.addEventListener("languagechange", onLanguageChange);
    return () => window.removeEventListener("languagechange", onLanguageChange);
  }, []);
  const value = useMemo(
    () => ({ locale: preference, setLocale, locales }),
    [version],
  );
  return (
    <LocaleContext.Provider value={value}>
      {loadError && loadError !== dismissedError && (
        <div className="startup-notice" role="alert">
          <div>
            <strong>{t("The interface language could not be loaded.")}</strong>
            <p>{loadError.message}</p>
          </div>
          <button type="button" onClick={() => {
            setDismissedError(null);
            void preloadLocale().catch(() => undefined);
          }}>
            {t("Try again")}
          </button>
          <button type="button" onClick={() => setDismissedError(loadError)}>
            {t("Dismiss")}
          </button>
        </div>
      )}
      {children}
    </LocaleContext.Provider>
  );
}
export function useLocale(): LocaleContextValue {
  const value = useContext(LocaleContext);
  if (!value) throw new Error("useLocale must be used inside I18nProvider");
  return value;
}

/** Community sources win; explicitly incomplete inherited/new catalogs fall back to English.
 * See locales/manifest.json missingNew and policy, never an implicit completeness claim.
 * Missing interpolation values are preserved visibly rather than silently removed.
 */
const cardinalRules = new Map<string, Intl.PluralRules>();
function translate(
  source: string,
  values?: Record<string, string | number>,
  explicitCount?: number,
): string {
  const locale = getLocale();
  const count =
    explicitCount ??
    (typeof values?.count === "number" ? values.count : undefined);
  const localized = catalogs[locale]?.[source];
  const message = localized ?? catalogs.en?.[source] ?? source;
  const grammarLocale = localized === undefined ? "en" : locale;
  let translated: string;
  if (typeof message === "string") translated = message;
  else {
    let rules = cardinalRules.get(grammarLocale);
    if (!rules) {
      rules = new Intl.PluralRules(grammarLocale);
      cardinalRules.set(grammarLocale, rules);
    }
    translated =
      message[count === undefined ? "other" : rules.select(count)] ??
      message.other;
  }
  if (!values && count === undefined) return translated;
  return translated.replace(
    /\{([A-Za-z][\w]*)\}/g,
    (placeholder, name: string) => {
      const value =
        name === "count" && count !== undefined ? count : values?.[name];
      return value === undefined
        ? placeholder
        : typeof value === "number"
          ? formatNumber(value)
          : value;
    },
  );
}
/** Numeric count values select locale-specific cardinal forms before number formatting. */
export function t(
  source: string,
  values?: Record<string, string | number>,
): string {
  return translate(source, values);
}
export function tPlural(
  source: string,
  count: number,
  values?: Record<string, string | number>,
): string {
  return translate(source, values, count);
}
const numbers = new Map<string, Intl.NumberFormat>();
const dates = new Map<string, Intl.DateTimeFormat>();
const bytesFormatters = new Map<string, Intl.NumberFormat>();
const displayNames = new Map<string, Intl.DisplayNames>();
export function formatNumber(value: number): string {
  const locale = getLocale();
  let formatter = numbers.get(locale);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale);
    numbers.set(locale, formatter);
  }
  return formatter.format(value);
}
export function formatDate(value: string | number | Date): string {
  // Archive calendar dates are not UTC instants; avoid moving them to the previous day.
  const date =
    value instanceof Date
      ? value
      : new Date(
          typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value)
            ? value + "T00:00:00"
            : value,
        );
  if (Number.isNaN(date.getTime())) return t("Unknown date");
  const locale = getLocale();
  let formatter = dates.get(locale);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, { dateStyle: "medium" });
    dates.set(locale, formatter);
  }
  return formatter.format(date);
}
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return t("Unknown size");
  const index = byteUnitIndex(value);
  const locale = getLocale();
  const key = locale + ":" + index;
  let formatter = bytesFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, {
      style: "unit",
      unit: byteUnits[index],
      unitDisplay: "short",
      maximumFractionDigits: index === 0 ? 0 : 1,
    });
    bytesFormatters.set(key, formatter);
  }
  return formatter.format(value / 1000 ** index);
}
export function languageName(code: string): string {
  const normalized = normalizeLocale(code);
  if (!normalized) return code;
  const locale = getLocale();
  let formatter = displayNames.get(locale);
  if (!formatter) {
    formatter = new Intl.DisplayNames([locale], {
      type: "language",
      fallback: "code",
    });
    displayNames.set(locale, formatter);
  }
  try {
    return formatter.of(normalized) || code;
  } catch {
    return code;
  }
}
