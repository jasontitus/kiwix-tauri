import assert from "node:assert/strict";
import test from "node:test";
import { rankMatchingVoices } from "../src/speech-voice-ranking.ts";
import { selectSpeechVoice } from "../src/speech-voice-selection.ts";
import { filterSpeechVoices } from "../src/speech-voice-filter.ts";

const voice = (voiceURI, quality = 0, isDefault = false) => ({
  name: voiceURI,
  lang: "en-US",
  voiceURI,
  default: isDefault,
  quality,
});

const ranked = (entries) => rankMatchingVoices(entries).map((entry) => entry.voiceURI);

test("exact language outranks a higher-quality regional voice", () => {
  assert.deepEqual(
    ranked([
      { voice: voice("premium-regional", 3), desired: false },
      { voice: voice("enhanced-exact", 2), desired: true },
    ]),
    ["enhanced-exact", "premium-regional"],
  );
});

test("quality ranks above the fabricated default flag", () => {
  assert.deepEqual(
    ranked([
      { voice: voice("compact", 1, true), desired: true },
      { voice: voice("enhanced", 2), desired: true },
      { voice: voice("premium", 3), desired: true },
    ]),
    ["premium", "enhanced", "compact"],
  );
});

test("default and name break remaining ties deterministically", () => {
  assert.deepEqual(
    ranked([
      { voice: voice("zulu", 2), desired: true },
      { voice: voice("alpha", 2), desired: true },
      { voice: voice("system", 2, true), desired: true },
    ]),
    ["system", "alpha", "zulu"],
  );
});

test("an English-only neural voice never becomes the fallback for another language", () => {
  const kokoro = { name: "Kokoro · Heart", lang: "en-US", voiceURI: "kokoro:af_heart", default: false, quality: 500 };
  const english = { name: "System English", lang: "en-US", voiceURI: "system:en", default: true, quality: 1 };
  const french = { name: "System French", lang: "fr-FR", voiceURI: "system:fr", default: false, quality: 2 };
  assert.equal(selectSpeechVoice([kokoro, english, french], [french], "").voiceURI, "system:fr");
  assert.equal(selectSpeechVoice([kokoro, english], [], "").voiceURI, "system:en");
  assert.equal(selectSpeechVoice([kokoro, english], [kokoro, english], "kokoro:af_heart").voiceURI, "kokoro:af_heart");
  assert.equal(selectSpeechVoice([kokoro, english, french], [english], "system:fr").voiceURI, "system:fr");
});

test("legacy Junior is removed and an installed Kokoro voice becomes the English default", () => {
  const junior = { name: "Junior", lang: "en-US", voiceURI: "com.apple.speech.synthesis.voice.Junior", default: true, quality: 0 };
  const system = { name: "System English", lang: "en-US", voiceURI: "system:en", default: false, quality: 2 };
  const kokoro = { name: "Kokoro · Heart", lang: "en-US", voiceURI: "kokoro:af_heart", default: false, quality: 500 };
  const usable = filterSpeechVoices([junior, system, kokoro]);
  assert.deepEqual(usable.map((voice) => voice.voiceURI), ["system:en", "kokoro:af_heart"]);
  assert.equal(selectSpeechVoice(usable, [kokoro, system], junior.voiceURI).voiceURI, "kokoro:af_heart");
});
