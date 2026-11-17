/** Obsolete system voices that exist in the catalog but produce no audio on current iOS. */
const LEGACY_SPEECH_VOICES = new Set(["com.apple.speech.synthesis.voice.Junior"]);

export interface FilterableSpeechVoice {
  voiceURI: string;
}

export function isLegacySpeechVoice(voice: FilterableSpeechVoice): boolean {
  return LEGACY_SPEECH_VOICES.has(voice.voiceURI);
}

export function filterSpeechVoices<T extends FilterableSpeechVoice>(voices: T[]): T[] {
  return voices.filter((voice) => !isLegacySpeechVoice(voice));
}
