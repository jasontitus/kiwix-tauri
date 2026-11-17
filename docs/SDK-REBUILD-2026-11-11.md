# Release SDK rebuild and verification — 2026-11-11

Source: `583f0c9` (includes merged PR #18). The libzim retained-fd patch changed in PR #18; its current SHA-256 is
`cefe049c5e529bdfa8c71b113f258a92790b40ed661c0326b96a0436fdb17f80`.

All manifests below record that patch hash, were rebuilt from the current source, and passed an independent
manifest-file SHA-256 check (0 mismatches). iOS manifests carry no `files` map; their source-patch records and
installed static archives were checked instead.

| Target | SDK prefix | Manifest SHA-256 | Verification |
|---|---|---|---|
| macOS arm64 | `.native/desktop-retained-v2/aarch64-apple-darwin` | `af3c1f1f69cbf0abf6e7908559fea1ebfcb0cc535229df42b8dd7599d5a4fae6` | 556 files, 0 mismatches; host tests/clippy; packaged app |
| Linux arm64 | `.native/linux-arm64-sdk-current` | `bf19c89715a1d0c2f2209e9bea4e37f1d79ff866bce4c0b7ace291d271659a99` | 558 files, 0 mismatches |
| Linux x86_64 | `.native/linux-x86_64-sdk-current` | `eaefbacd9758bda3c08aa0a01997c5478db6d984451b45e20d464f3f534fb6c6` | 558 files, 0 mismatches |
| iOS device | `.native/mobile-current/aarch64-apple-ios` | `b2e889105889aed21dd488050132b876b6a043da410fc9fb844827604d4babea` | `cargo check`; device archive |
| iOS simulator | `.native/mobile-current/aarch64-apple-ios-sim` | `c65ec3880dd4bd4338e63f603dcc831099a07b717ccb83333137cc196e126ce2` | `cargo check`; simulator app installed/launched |
| Android arm64-v8a | `.native/mobile-current/aarch64-linux-android` | `c0de5dfffaa1e5974b3e2fa097dccfed3d6d2affc43c75da63f95017a4ca31fa` | `cargo check`; APK; physical Samsung |
| Android armeabi-v7a | `.native/mobile-current/armv7-linux-androideabi` | `71c065dabe732df670c03c39a8b8e2a285445566013ec0d0039e6948e8a5d697` | `cargo check`; APK |

## Built applications

- macOS packaged app: `src-tauri/target/release/bundle/macos/Kiwix.app`, executable SHA-256
  `a4f5329cc075f06dc38b197b9b84d833bd299c4f1eaf7f7f047c6a5262ba0f19`; native StreetZim normal/restricted
  mode passed in the merged app.
- iOS device archive: `src-tauri/gen/apple/build/kiwix-workbench_iOS.xcarchive`, executable SHA-256
  `16ef619cba1e14ad42ef6d134da4f496553cb1ad244926ebd6f3c3d326ed83f0`, unsigned debug build, `platform IOS`,
  minimum iOS 16.4, SDK 27.2.
- iOS simulator app: built from `.native/frontend-ios`, installed and launched on the `IosWalkthrough-20261031`
  simulator; screenshot `.native/verify/ios-simulator-launch.png`.
- Android arm64 APK: `.native/verify/kiwix-arm64-android-frontend.apk`, SHA-256
  `d5bf4ee5e391f5f7f8c438fff2879d13899818940efccb7ddb6c87858a596adb`, signed with the Android debug
  certificate `b2a7c3e8…`.
- Android ARMv7 APK: `.native/verify/kiwix-armv7-android-frontend.apk`, SHA-256
  `54cc4a551c3f1cbe798a45483c5e4be198e4d08b4655458ee2e634475763ab15`, signed with the same debug certificate.
- Both APKs use `TAURI_ENV_PLATFORM=android` frontend assets. A generic desktop `dist` build was rejected by
  the Android boot guard on the physical phone; the platform-specific rebuild is the verified artifact.

## Runtime checks

- Samsung SM-A356U (`<samsung-serial>`) accepted the arm64 APK in place with the existing debug certificate; the
  library remained 12 archives and the Hawaii reader rendered the MapLibre map. Screenshots:
  `.native/verify/samsung-nearby.png`, `.native/verify/samsung-hawaii.png`.
- Host gates on this tree: **1078 Rust tests** (19 ignored), **315 JS**, **178 Python**, frontend/TypeScript,
  host clippy, Android arm64/ARMv7 clippy, Windows GNU clippy.
- Windows native evidence remains the merged PR #18 report (`docs/WINDOWS-TEST-REPORT.md`); its SDK was built on
  the native Windows machine, not rebuilt on this Mac.

## Not rebuilt in this pass

- Windows MSVC SDK: native Windows agent report only.
- Linux application package: SDKs were built and manifest-verified; no Linux AppImage was rebuilt here.
- iOS device archive is unsigned (`--no-sign`); it is not a distribution artifact.
