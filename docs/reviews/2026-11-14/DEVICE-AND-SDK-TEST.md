# Device and SDK test — 2026-11-14

Source: `b84c62e` (latest pulled main). Host checks passed: `scripts/dev.sh check`, `scripts/dev.sh test`
(**1116 passed, 0 failed, 19 ignored**), **350 JS tests**, frontend build.

## SDK rebuilds for Xapian 1.4.32

The pull moved desktop/Android SDK recipes from Xapian 1.4.23 to 1.4.32. Rebuilt:

| Target | SDK prefix | Manifest SHA-256 |
|---|---|---|
| macOS arm64 | `.native/desktop-current` | `2d24ea7e8a777b6a7ce7e4b452afd9dcf562d93fb9472dc0f4aa44d0e035d3ad` |
| Android arm64 | `.native/android-current/aarch64-linux-android` | `b29650091af7553cd557ebe472f003eaac94b25fc9088ae883a3002adffbce12` |
| Android ARMv7 | `.native/android-current/armv7-linux-androideabi` | `f955cb140fabb659283a4f9d8952db247a855ffa166f6ef5b0ba91fbf3ea4a95` |

The iOS SDK recipe still pins Xapian 1.4.23 and was unchanged; the existing iOS manifest remains
`b2e889105889aed21dd488050132b876b6a043da410fc9fb844827604d4babea`.

## Builds

- Android arm64 debug APK: `.native/verify/kiwix-latest-arm64.apk`, SHA-256
  `a911d4a9a12e3b3ddbb90f0f867f50e262ef4bbce654362fbc4e579adf09ea9b`, installed in place on the
  Samsung SM-A356U with its existing data.
- Android ARMv7 debug APK: `.native/verify/kiwix-latest-armv7.apk`, SHA-256
  `d8c19035fe8e07c4dc5ae65fa1b55560c46a50f8e8ce623c5b49044edcfc3a34`; no ARMv7 device was available.
- Signed iPhone app: `.native/iphone-latest-20261114/.../Kiwix.app`, executable SHA-256
  `831a78904524492c518fdd595dc212747cc5ea91225403a470bfbb84c7ca36a1`, iOS 16.4 minimum, SDK 27.2.
  Installed on the iPhone 17 Pro Max test device and the iPhone 13 Pro.
- Latest iOS simulator app: built from `.native/frontend-ios-20261114`, installed and launched on
  `IosWalkthrough-20261031`.

## Device results

### Samsung SM-A356U — pass

- Latest arm64 APK installed in place; library remained 12 archives.
- Hawaii StreetZim MapLibre map rendered.
- Library pin was created, survived a force-stop/relaunch, and remained at the top.
- Discover defaulted to **English** with one active filter and a populated catalog.
- Archive-editions settings panel exposed the new bulk-update controls.
- New Nearby screen showed Share all / temporary Wi-Fi / Stop finding / share-a-book controls.
- Read Aloud opened the native speech panel; tapping **Read article** started a Google TTS
  `AudioTrack` (`USAGE_MEDIA`, `CONTENT_TYPE_SPEECH`). Sentence/word highlight was not visually
  confirmed on the tested tile page.

### iPhone 17 Pro Max — pass

The latest signed app installed and launched. The Michael Jackson article was open with active
read-along paint: the current sentence had a yellow highlight and the current word a blue highlight.
Evidence: `.native/verify/iphone17-readalong.png`. The prior XCTest passcode sheet was dismissed
before this run. This is a physical-iPhone read-along/reader check; Now Playing controls in Control
Center were not independently exercised.

### iPhone 13 Pro — installed, profile not trusted

`devicectl` installed the app, but launch still failed with the standard untrusted-developer-profile
error after Wi-Fi was restored. The developer certificates are visible, but that alone is not
sufficient on current iOS: enable **Settings → Privacy & Security → Developer Mode**, reboot, and
explicitly trust the Kiwix developer profile under **Settings → General → VPN & Device Management**.
No physical iPhone 13 runtime claim is made.

### Kindle Fire HD 10 (KFTRPWI, Fire OS 9 / API 28) — pass with automatic fallback

The tablet appeared on USB and accepted the arm64 debug APK in place. It launched to the new
responsive tablet UI with three retained archives; the Coffee archive opened in the reader and the
new Nearby/Maps/Discover surfaces rendered. Evidence: `.native/verify/fire-latest.png`,
`.native/verify/fire-library.png`.

The latest build also loads the app-level Kokoro model from the app data directory, offers
`Kokoro · Adam · American English` in the reader's Read aloud panel, and opens an AAudio player from
`org.kiwix.workbench` while synthesizing the Coffee article with sherpa-onnx. The Fire has no
network path for the 181.9 MB model download, so the verified pack was provisioned into the app data
directory; download-from-Settings was exercised on the Samsung. Evidence:
`.native/verify/kokoro-fire-launch.png`, `.native/verify/kokoro-fire-voices.png`,
`.native/verify/kokoro-fire-playing.png`.

The standard build now measures performance before offering Kokoro. On this
Fire the uncached probe measured `3.06` (synthesis time divided by audio time;
the pass limit is `1.10`), so Kokoro was omitted from the reader and the
existing system voice path selected Ivona `salli`. Read article showed
“Reading aloud with salli” and live highlighting. This is a CPU throughput
limit, not a network or AAudio problem. The fallback also exposed and fixed two
system-TTS issues: the speech foreground service could miss its five-second
window while TTS binder work occupied the main thread, and Ivona rejected a
chunk exactly equal to the advertised 4000-character maximum. Evidence:
`.native/verify/fire-adaptive-readaloud.png`,
`.native/verify/fire-adaptive-voice-panel.png`,
`.native/verify/fire-system-fixed6.png`.

### iOS simulator — pass

The latest app launched to the Library welcome screen with the new icon/navigation and no crash.

## Limits

Physical iPhone 13 launch remains blocked on developer-profile trust. No claim is made for audible
Kokoro output on a physical iPhone, physical iPhone Now Playing, Android 17 local-network
permission (Samsung is API 36), or bulk update execution without an eligible update. Android
read-along highlighting and the app-owned AAudio route were verified on Samsung and Fire.

## Kokoro TTS check

The current app has its own Kokoro integration; it does not depend on a system Kokoro TTS engine.
The pinned English pack is downloaded from the CastCircle release, SHA-256 verified, extracted
under `files/kiwix/models/kokoro-en-fp16-v1_0`, and synthesized by sherpa-onnx in the Rust process.

- **Samsung (SM-S928U1, Android 16 / API 36):** the Settings download installed the 181.9 MB pack.
  The reader listed `Kokoro · Adam · American English`; reading Animal/Coffee showed
  “Reading aloud with Kokoro · Adam” and an `org.kiwix.workbench` AAudio player in
  `state:started` routed to the speaker at 44.1 kHz. Evidence:
  `.native/verify/kokoro-samsung-downloaded4.png`,
  `.native/verify/kokoro-samsung-fresh-playing.png`.
- **Kindle Fire HD 10:** the pack was provisioned into app data because the tablet had no working
  network. The reader listed and used `Kokoro · Adam`; the app-owned AAudio player started while
  sherpa-onnx synthesized the Coffee article. Evidence:
  `.native/verify/kokoro-fire-voices.png`, `.native/verify/kokoro-fire-playing.png`.
- **iPhone 17 / iPhone 13:** iOS now ships the same app-level Kokoro path and an embedded sherpa
  xcframework. The iOS simulator lifecycle check passed earlier, but audible Kokoro output on a
  physical iPhone is still unverified; the iPhone 13 could not launch without developer-profile
  trust.
- **System TTS:** remains the default and fallback path. No `dev.kokoro.tts` engine or system-level
  Kokoro package is required.

Audio was observed through Android's active AAudio route and the app UI; no speaker recording or
speech transcription was performed.
