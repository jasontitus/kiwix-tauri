/** A language's matching voices, or the OS default when no neural voice fits. */
export interface SelectableVoice {
  voiceURI: string;
  default: boolean;
}

/**
 * Never fall back to an English-only neural voice for another language. The
 * caller supplies the already language-matched list; `all` is also used to
 * honor an explicitly saved non-neural voice and for the OS-default fallback.
 */
export function selectSpeechVoice<T extends SelectableVoice>(
  all: T[],
  matching: T[],
  saved: string,
  neuralPrefix = "kokoro:",
): T | undefined {
  const savedVoice = all.find((voice) => voice.voiceURI === saved);
  if (savedVoice && (!savedVoice.voiceURI.startsWith(neuralPrefix) ||
      matching.some((voice) => voice.voiceURI === saved))) return savedVoice;
  if (matching.length) return matching[0];
  const system = all.filter((voice) => !voice.voiceURI.startsWith(neuralPrefix));
  return system.find((voice) => voice.default) ?? system[0] ??
    all.find((voice) => voice.default) ?? all[0];
}
