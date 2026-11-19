# Offline neural read-aloud (Kokoro)

Read aloud keeps the platform voice path as the default. On every platform
(iPhone, Android, Windows, macOS and Linux) a user may install a single English
Kokoro voice pack from **Settings → Read aloud → Download neural voices**. The app
runs a one-time real-time performance check before offering the model; on devices
that cannot keep up, the reader continues with system voices.

## Model and provenance

- Model: Kokoro-82M English, fp16 ONNX, 24 kHz output, 28 US/GB speakers.
- Pack: `kokoro-en-fp16-v1_0.tar.bz2` (181,886,349 bytes), pinned release tag
  `kokoro-en-fp16-v1` on the CastCircle repository.
- SHA-256: `4cafe1c49bf4b0a7f9c2fab9f2b010b05544dde2b28b66e3211e832308a1a1f9`.
- The app verifies the archive before extraction, rejects absolute/traversal
  paths and links, extracts into a temporary directory, checks the required
  files, and only then publishes `models/kokoro-en-fp16-v1_0`.
- The model is not bundled. It is a one-time network download and can be
  removed from the same Settings panel. The APK contains the sherpa-onnx and
  ONNX Runtime libraries needed to run it, not the 181.9 MB model pack.
- The app (mobile and desktop) offers the download when online and the model is absent:
  a dismissible Library prompt after launch, and a prompt in the Read aloud
  panel before the first neural playback. **Not now** is remembered per
  surface; playback continues with system voices. There is no automatic
  download.

The archive is a user-owned release asset, not an unpinned upstream branch.
Changing the bytes requires a new tag and a corresponding code update.

## Runtime

- Inference: `sherpa-onnx` `OfflineTts` with the Kokoro model.
- Grapheme-to-phoneme: `misaki-rs` with the embedded English lexicon and
  morphology rules. This supplies plural/past/continuous forms and POS-aware
  heteronyms; the pack's raw word lexicon alone drops inflected words.
- Playback: `rodio` on the platform audio device. Android keeps the existing
  `KiwixSpeechService` `mediaPlayback` foreground service and MediaSession;
  iOS claims the existing playback `AVAudioSession` for the duration.
- Synthesis and playback run on one named worker thread. Each sentence-sized
  chunk is cached by text, speaker and rate, so repeated playback is immediate.
  The cache is bounded to roughly 300 MB and removed with the model.
- The existing native speech ownership/capability path remains authoritative:
  private/read-only transitions, reader retirement and explicit stop retire
  Kokoro exactly like a platform voice.

## Bundled Android offline build

The standard Android APK omits the voice data. Supply the expanded model directory when building an offline edition.

```sh
ORG_GRADLE_PROJECT_kiwixBundledKokoroDir=/path/to/model-root \
  npm run tauri -- android build --target aarch64 --apk --ci
```

`model-root/kokoro-en-fp16-v1_0` must contain the pinned distribution. Gradle reads the 360 named files in the same order as the Rust installer's manifest, checks each length and SHA-256, and stores their 221,488,168 bytes as one uncompressed APK asset. Other files in the supplied directory are excluded. A missing or modified required file stops the build. [Pinned model layout](../src-tauri/src/kokoro/pack-v1.tsv)

On Android, AssetManager locates the asset inside the installed base or split APK. The app rechecks that APK's identity and bounds reads to the reported region. The bundled edition and the download share the Rust installer and its file checks. The app can prepare the bundled model offline during writable, non-private startup. Settings can retry an interrupted installation or explicitly reinstall a removed model; a normal launch respects its removal marker. The optional native inference engine still applies its performance gate.

## Hardware gate

Kokoro is offered only when synthesis can keep up with playback. After the
model is installed, a background probe synthesizes one fixed, uncached
140-character English sentence with `am_adam` and measures:

`ratio = synthesis time / generated audio duration`

A ratio at or below `1.10` passes. The result is persisted as
`models/performance.json`; later launches do not re-probe. Unknown, failed, or
slow measurements omit Kokoro from `mobile_speech_voices`, so the existing
selection logic falls back to an installed system voice. Deleting the model
removes the measurement. This gate prevents a slow device from starting a
playback that would pause for tens of seconds between chunks.

## Article text

The reader bridge extracts spoken text from the live document, not
`innerText` of the whole page:

- skips headers, navigation, sidebars, footers, forms, scripts, hidden nodes
  and known Wikipedia chrome (`mw-editsection`, TOC, infobox, navbox,
  references, hatnotes, categories, print footer);
- skips the page's first H1 and a leading bold title matching the document
  title, so the article title is not read twice;
- stops at terminal sections such as References, External links, See also,
  Further reading and Notes;
- preserves block boundaries and the live-node run map used for sentence and
  word highlighting.
- treats a period, colon, or semicolon as a chunk boundary only when it is
  followed by whitespace or the end of the text. Decimal points, times,
  versions, and URL punctuation therefore stay in one utterance (`33.6
  meters`, `10:30`, `1.2.3`).

The rules are generic semantic/class/id checks; Wikipedia-specific classes are
additions, not a requirement for other ZIM files.

## Limits

- English only. A language with no matching Kokoro voice selects an installed
  system voice; the neural voices are never used as a cross-language fallback.
- CPU synthesis is near real time on mid-range devices, but not on every
  low-end device. Each punctuation-delimited chunk is synthesized separately;
  while it plays, one next chunk is synthesized on a prefetch thread. Devices
  that fail the hardware gate use system TTS instead of starting a playback
  that cannot keep up.
- Desktop routes only the Kokoro voices through Rust (`mobile_speech.rs`'s
  desktop transport, the same ownership machine and `mobile_speech_*` contract
  as mobile, with no native synthesizer). Operating-system voices stay in the
  archive frame's `speechSynthesis`. The reader merges both lists; changing to
  a voice of the other engine mid-read stops reading rather than switching live.
- Windows links sherpa-onnx's shared build, so its runtime DLLs ship beside the
  executable ([packaging](PACKAGING.md)).
- Android emulator, Samsung (API 36) and Kindle Fire HD 10 (API 28) paths are
  verified with the pinned pack. iOS compilation and the audio-session handoff
  are verified; audible physical-iPhone acceptance remains required.
- On the Fire HD 10 (MT8183), the debug build measured 5.1-6.2 seconds of
  synthesis for 1.0-1.45 seconds of audio on short lines and 17.4 seconds for
  5.19 seconds of real Wikipedia prose. The probe measured a ratio of `3.06`,
  so the standard build now selects system TTS on that device. Samsung
  measured `0.87` and keeps Kokoro. This is CPU throughput, not network or
  audio-output latency.

## Verification

- `cargo test` on the host: full suite passed, including Kokoro chunk/voice/WAV
  tests and the speech background-retention invariant.
- `node --test scripts/test-reader-read-aloud.mjs scripts/test-download-state.mjs
  scripts/test-speech-preferences.mjs`: 40 passed, including a real-DOM
  `childNodes` extraction fixture and the English-only voice fallback.
- Android emulator (API 36, clean 5.8 GB data image): downloaded and installed
  the 181.9 MB pack, loaded the sherpa-onnx engine, synthesized cached WAVs,
  opened AAudio, kept a `KiwixReadAloud` media session playing after Home, and
  showed no lexicon OOV warnings after the Misaki G2P cutover. Audio content
  was not listened to or transcribed.
- Samsung SM-S928U1 (Android 16 / API 36): the Settings download installed the
  pinned pack, the reader selected `Kokoro · Adam · American English`, and
  `dumpsys audio` showed an `org.kiwix.workbench` AAudio player in
  `state:started` on the speaker route. Evidence:
  `.native/verify/kokoro-samsung-downloaded4.png`,
  `.native/verify/kokoro-samsung-fresh-playing.png`.
- Kindle Fire HD 10 (Fire OS 9 / API 28): the pack was provisioned into app data
  (no usable tablet network); the reader selected `Kokoro · Adam`, sherpa-onnx
  synthesized the Coffee article, and the app-owned AAudio player started.
  Evidence: `.native/verify/kokoro-fire-voices.png`,
  `.native/verify/kokoro-fire-playing.png`. The performance probe measured
  `3.06`; the standard build then omitted Kokoro and selected Ivona `salli`.
  Read article showed “Reading aloud with salli”, highlighted the live text,
  and the TTS service entered the foreground without a crash. Evidence:
  `.native/verify/fire-adaptive-readaloud.png`,
  `.native/verify/fire-adaptive-voice-panel.png`,
  `.native/verify/fire-system-fixed6.png`.
- iOS simulator: debug app built with the sherpa xcframework embedded and the
  Swift `speechEngineUpdate` command compiled. A temporary XCUITest launched
  the restored Animal article, opened Reader navigation → Read aloud → Read
  article, backgrounded the app for five seconds, reactivated it, and still
  found the live “Stop reading” control. This proves the Rust lifecycle no
  longer retires the started session on backgrounding; audible output on a
  physical iPhone remains unverified.
- Independent review found and this change fixed: lifecycle-retired Kokoro
  sessions not stopping the rodio worker, pause/resume skipping a chunk,
  Android notification state not following pause, desktop offering an unusable
  download, an uncancellable stalled transfer, model deletion leaving the
  media session alive, iOS pause leaving ducking active, real-DOM `nodeText`
  reading only child elements, and saved non-matching system voices being
  dropped.
- The adaptive gate test covers the `1.10` real-time threshold, and the Fire
  and Samsung probes produced `3.06` and `0.87` respectively. The standard
  Fire build used Ivona `salli` after the gate; the Samsung build kept
  `Kokoro · Adam`. A chunker regression test keeps `33.6 meters` and
  `1,234.5 kilograms` in one utterance.
