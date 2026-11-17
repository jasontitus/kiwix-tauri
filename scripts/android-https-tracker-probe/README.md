# Android HTTPS tracker probe

Runtime check that BitTorrent HTTPS tracker announces work on Android. It is a
test harness, not part of the app build or CI.

librqbit 9 (`rust-tls`) builds a reqwest 0.13 client without custom roots, so
reqwest verifies through `rustls-platform-verifier`. On Android that verifier
needs a one-time JNI initialisation, supplied by `src-tauri/src/android_tls.rs`,
and the Kotlin class from `src-tauri/vendor/rustls-platform-verifier-android-0.2.0`.

The probe app packages that vendored module and the app's own
`network_security_config.xml`. Its Rust library compiles the app's exact
`android_tls.rs` and runs the same librqbit and reqwest versions as the app.
It publishes the application Context through `ndk_context` as Tao does, then:

- `mode=noinit`: skips the initialisation (the app before the fix);
- `mode=init`: calls `ensure_platform_verifier()` twice from a Tokio worker
  that is not attached to the JVM, as the app's torrent engine does;

and then makes three direct HTTPS requests with a client built the way librqbit
builds one, plus a real librqbit magnet `add_torrent` whose only tracker is
`https://10.0.2.2:8443/announce`.

## Steps

1. Android SDK with an emulator and the `system-images;android-24;default;x86_64`
   image, NDK r28, and `rustup target add x86_64-linux-android`.
   An emulator without KVM runs with `-accel off`.
2. Boot an AVD with `-writable-system`, then `adb root` and `adb remount`.
3. `server/make-pki.sh <pki-dir>`, then push `<pki-dir>/<hash>.0` to
   `/system/etc/security/cacerts/` (mode 644). Append
   `10.0.2.2 r10.c.lencr.org` to `/system/etc/hosts`. The CRL-only leaf names
   `http://r10.c.lencr.org/probe.crl`, a host on the app's list.
4. As root on the host: `python3 -I server/tracker.py <pki-dir> <log-file>`.
   It listens on 80, 8080 (CRL), 8443 (CRL-only leaf), 8445 (leaf without
   revocation info) and 8444 (untrusted root).
5. Copy `src-tauri/Cargo.lock` into `rust/`, then build `rust/` for
   `x86_64-linux-android` with the NDK clang as linker. Copy `libprobe.so` to
   `android/app/src/main/jniLibs/x86_64/`.
6. From `android/`, write `local.properties` with `sdk.dir`, then run
   `../../../src-tauri/gen/android/gradlew -p . :app:assembleRelease`. The
   release build is R8-shrunk like the app's. Add `-PprobeWithoutVerifier=true`
   for a deliberately broken package without the Kotlin module.
7. Install the APK, then `ADB=adb TRACKER_LOG=<log-file> ./run.sh noinit` and
   `./run.sh init`. Each run starts a fresh process.
8. Revoked case: `server/make-pki.sh <pki-dir> revoke`, then `./run.sh init`.
   To see the effect of an HTTPS-only policy, remove the CA host block from a
   copy of the app's `network_security_config.xml`; this step is manual.

On a kernel without `getrandom` (the API 24 emulator's 3.10 kernel), aws-lc
blocks until the kernel reports 256 bits of entropy, and the idle software
emulator stayed below that. Build `tools/credit.c` with the NDK, push it, and
run it as root in a loop during probe runs. aws-lc is also the app's TLS
provider; real devices on kernels without `getrandom` were not checked.

## Result, 2026-11-05 (API 24 x86_64 emulator, software CPU, release APK)

| Case | `noinit` | `init` |
|---|---|---|
| `ensure_platform_verifier()` | not called | `Ok(())`, twice, on `tokio-rt-worker` |
| Same, package without the Kotlin module | — | `Err("BitTorrent is unavailable because this installation is incomplete. Reinstall Kiwix.")`; logcat `Kiwix` names the missing class; no panic |
| CRL-only leaf, trusted root | panic: "Expect rustls-platform-verifier to be initialized" | HTTP 200 after Android fetched the CRL |
| Same leaf, CRL lists it as revoked (debug APK) | — | `InvalidCertificate(Revoked)` |
| Leaf without OCSP or CRL, trusted root | panic | `InvalidCertificate(Revoked)` (upstream behaviour, see below) |
| Untrusted root | panic | `InvalidCertificate(UnknownIssuer)` |
| librqbit magnet, HTTPS tracker | `add_torrent` panicked; no request reached the tracker | tracker logged `info_hash=c9e15763…`, UA `Kiwix Workbench`; librqbit parsed the response |

The debug APK gave the same `init` results. With the app's earlier HTTPS-only
network config, the CRL fetch was blocked, and the CRL-only leaf failed as
`Revoked`. Android's revocation check treats a certificate without an OCSP
responder as a hard failure unless the CRL can be fetched. The app config
therefore permits cleartext to upstream's CA revocation hosts.

On this software CPU, a handshake that fetched a CRL from the LAN server took
about 5–8 s. The revocation check without a fetch (the no-revocation leaf) took
1.3–2.5 s, and the untrusted root failed in about 25 ms. Verification blocks a
Tokio worker for that time; latency against real CA servers on a device was
not measured. In the app, librqbit's handshakes run on a dedicated torrent
runtime, so that worker is not one of Tauri's (this probe does not model it).

Not covered: the full app APK, physical devices, Android versions other than 7.0
(API 24), and public CAs and trackers on the Internet (the session's network
proxy intercepts TLS). A leaf with neither OCSP nor a CRL distribution point
that chains to a system root fails as `Revoked`. That comes from upstream's
Kotlin verifier, not from this app.
