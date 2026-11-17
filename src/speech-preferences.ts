import { normalizeLocale } from "./i18n";
import type { SpeechPreference } from "./types";
import { rankMatchingVoices } from "./speech-voice-ranking";
import { selectSpeechVoice } from "./speech-voice-selection";
import { filterSpeechVoices, isLegacySpeechVoice } from "./speech-voice-filter";

export interface InstalledVoice {
  name: string;
  lang: string;
  voiceURI: string;
  default: boolean;
  /** Native voice-quality tier; Android reports 0, iOS maps AVSpeechSynthesisVoiceQuality. */
  quality?: number;
}

/** Intl's CLDR aliases canonicalize ZIM ISO 639-3 codes (eng → en, fra → fr). */
export function archiveSpeechLanguages(metadata: string): string[] {
  return [...new Set(metadata.split(/[;,\s]+/).map(normalizeLocale)
    .filter((language): language is string => Boolean(language) && language !== "und" && language !== "mul"))];
}

export function speechPreference(
  preferences: Record<string, SpeechPreference>, language: string,
): SpeechPreference {
  return preferences[language] ?? preferences["*"] ?? { voice: "", rate: 1 };
}

interface VoiceLocale {
  locale: string;
  candidate: Intl.Locale;
}

/** Voice locales repeat across renders; canonicalize and maximize each tag once. */
const voiceLocales = new Map<string, VoiceLocale | null>();
const maximizedTargets = new Map<string, Intl.Locale>();
const LOCALE_CACHE_LIMIT = 128;

function voiceLocale(lang: string): VoiceLocale | null {
  const cached = voiceLocales.get(lang);
  if (cached !== undefined) return cached;
  const locale = normalizeLocale(lang);
  const entry = locale
    ? { locale, candidate: new Intl.Locale(locale).maximize() }
    : null;
  if (voiceLocales.size >= LOCALE_CACHE_LIMIT) voiceLocales.clear();
  voiceLocales.set(lang, entry);
  return entry;
}

function maximizedTarget(locale: string): Intl.Locale {
  const cached = maximizedTargets.get(locale);
  if (cached) return cached;
  const target = new Intl.Locale(locale).maximize();
  if (maximizedTargets.size >= LOCALE_CACHE_LIMIT) maximizedTargets.clear();
  maximizedTargets.set(locale, target);
  return target;
}

export function matchingSpeechVoices(voices: InstalledVoice[], language: string): InstalledVoice[] {
  const desired = normalizeLocale(language);
  if (!desired) return [];
  const target = maximizedTarget(desired);
  const matching: { voice: InstalledVoice; desired: boolean }[] = [];
  for (const voice of voices) {
    if (isLegacySpeechVoice(voice)) continue;
    const entry = voiceLocale(voice.lang);
    if (!entry || entry.candidate.language !== target.language ||
        entry.candidate.script !== target.script) continue;
    matching.push({ voice, desired: entry.locale === desired });
  }
  return rankMatchingVoices(matching);
}

export function selectedSpeechVoice(voices: InstalledVoice[], language: string, saved: string): InstalledVoice | undefined {
  const usable = filterSpeechVoices(voices);
  return selectSpeechVoice(usable, matchingSpeechVoices(usable, language), saved);
}

export function adjustedSpeechRate(rate: number, direction: number): number {
  return Math.max(0.25, Math.min(2, Math.round((rate + direction * 0.05) * 100) / 100));
}
