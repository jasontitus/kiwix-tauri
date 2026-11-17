import type { InstalledVoice } from "./speech-preferences";

export interface RankedSpeechVoice {
  voice: InstalledVoice;
  desired: boolean;
}

/** Exact language first, then native quality, system default, and name. */
export function rankMatchingVoices(matching: RankedSpeechVoice[]): InstalledVoice[] {
  return matching
    .sort((a, b) =>
      Number(b.desired) - Number(a.desired) ||
      (b.voice.quality ?? 0) - (a.voice.quality ?? 0) ||
      Number(b.voice.default) - Number(a.voice.default) ||
      a.voice.name.localeCompare(b.voice.name))
    .map((entry) => entry.voice);
}
