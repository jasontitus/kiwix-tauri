// SPDX-License-Identifier: GPL-3.0-or-later
//! Android trust for librqbit's HTTPS client.
//!
//! librqbit's `rust-tls` feature builds a reqwest 0.13 client without custom
//! roots, so reqwest uses `rustls-platform-verifier`. On Android that verifier
//! calls the platform trust manager through JNI and must be given the
//! application Context once. Without it, the first HTTPS tracker announce
//! panics inside the TLS handshake ("Expect rustls-platform-verifier to be
//! initialized"): the announce fails and the panic hook records a crash.
//!
//! The JVM half is `org.rustls.platformverifier.CertificateVerifier`, built
//! from `vendor/rustls-platform-verifier-android-0.2.0`. Initialization loads
//! it through the application class loader and resolves the method and result
//! fields the verifier calls, so a missing or R8-stripped component is reported
//! here instead of failing every handshake.
//!
//! Verification is a blocking JNI call on the Tokio worker that runs the TLS
//! handshake, and Android may fetch a CRL over the network inside it. A slow CA
//! revocation server therefore holds that worker for the duration (not
//! measured on devices). librqbit's handshakes run on the dedicated torrent
//! runtime (`torrent_runtime` in torrent.rs), not Tauri's; see
//! docs/RELIABILITY.md.
//!
//! The app's own reqwest 0.12 client (catalog, `.torrent` metadata, HTTP
//! downloads) uses bundled WebPKI roots and does not depend on this.
use jni_0_22::{
    objects::{JClass, JObject},
    signature::{FieldSignature, MethodSignature, RuntimeFieldSignature, RuntimeMethodSignature},
    strings::JNIString,
    sys, Env, JavaVM,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

const VERIFIER_CLASS: &str = "org.rustls.platformverifier.CertificateVerifier";
const RESULT_CLASS: &str = "org.rustls.platformverifier.VerificationResult";
// Must match rustls-platform-verifier 0.7.1 `verification/android.rs`.
const VERIFY_SIGNATURE: &str = "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;\
    [Ljava/lang/String;[BJ[[B)Lorg/rustls/platformverifier/VerificationResult;";

static READY: AtomicBool = AtomicBool::new(false);
static INITIALIZING: Mutex<()> = Mutex::new(());

/// Initializes the platform certificate verifier once per process.
///
/// Idempotent and cheap after success. A failure is returned (not cached) so a
/// later attempt can retry; callers must not start HTTPS work after an error.
pub(crate) fn ensure_platform_verifier() -> Result<(), String> {
    if READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let _guard = INITIALIZING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if READY.load(Ordering::Acquire) {
        return Ok(());
    }
    initialize().map_err(|error| {
        // Technical detail (class/JNI names only) goes to logcat; Android drops
        // stderr. The user gets the recovery.
        logcat_error(&format!("rustls-platform-verifier initialization failed: {error}"));
        "BitTorrent is unavailable because this installation is incomplete. Reinstall Kiwix.".to_string()
    })?;
    READY.store(true, Ordering::Release);
    Ok(())
}

fn initialize() -> Result<(), String> {
    // Tao publishes the process-lifetime application Context and JavaVM in
    // `onCreate`, before Tauri's main entry point runs; the only caller is the
    // torrent engine, which exists only after setup.
    let android = ndk_context::android_context();
    if android.vm().is_null() || android.context().is_null() {
        return Err("the Android application context is unavailable".into());
    }
    // SAFETY: the pointer is the live process JavaVM published by Tao.
    let vm = unsafe { JavaVM::from_raw(android.vm().cast::<sys::JavaVM>()) };
    // Scoped attach: a Tokio worker is not left permanently attached.
    vm.attach_current_thread_for_scope(|env| -> Result<(), Failure> {
        let result = (|| -> Result<(), Failure> {
            // SAFETY: Tao owns a global reference to the application Context for
            // the life of the process. The temporary wrapper is used only to
            // create a frame-local reference; JObject has no Drop, so the global
            // reference is never deleted here.
            let global = unsafe { JObject::from_raw(env, android.context().cast()) };
            let context = env.new_local_ref(&global)?;
            require_verifier_class(env, &context)?;
            rustls_platform_verifier::android::init_with_env(env, context)?;
            Ok(())
        })();
        // A pending Java exception must never leak onto a reused worker thread.
        if env.exception_check() {
            env.exception_clear();
        }
        result
    })
    .map_err(|Failure(message)| message)
}

/// Loads the JVM component through the same loader the verifier will use and
/// resolves every member the verifier reaches through JNI.
fn require_verifier_class(env: &mut Env<'_>, context: &JObject<'_>) -> Result<(), Failure> {
    let get_loader = RuntimeMethodSignature::from_str("()Ljava/lang/ClassLoader;")?;
    let load_class = RuntimeMethodSignature::from_str("(Ljava/lang/String;)Ljava/lang/Class;")?;
    let loader = env
        .call_method(context, JNIString::new("getClassLoader"), MethodSignature::from(&get_loader), &[])
        .and_then(|value| value.l())
        .map_err(|error| Failure(format!("the application class loader is unavailable ({error})")))?;
    let load = |env: &mut Env<'_>, name: &str| -> Result<(), Failure> {
        let text = env.new_string(name)?;
        let class = env
            .call_method(&loader, JNIString::new("loadClass"), MethodSignature::from(&load_class), &[(&text).into()])
            .and_then(|value| value.l())
            .and_then(|class| env.cast_local::<JClass>(class))
            .map_err(|error| Failure(format!("{name} is not packaged in this build ({error})")))?;
        let missing = |member: &'static str| move |error| Failure(format!("{name}.{member} is missing ({error})"));
        if name == VERIFIER_CLASS {
            let signature = RuntimeMethodSignature::from_str(VERIFY_SIGNATURE)?;
            env.get_static_method_id(&class, JNIString::new("verifyCertificateChain"), MethodSignature::from(&signature))
                .map_err(missing("verifyCertificateChain"))?;
        } else {
            for (field, signature) in [("code", "I"), ("message", "Ljava/lang/String;")] {
                let signature = RuntimeFieldSignature::from_str(signature)?;
                env.get_field_id(&class, JNIString::new(field), FieldSignature::from(&signature))
                    .map_err(missing(field))?;
            }
        }
        Ok(())
    };
    load(env, VERIFIER_CLASS)?;
    load(env, RESULT_CLASS)
}

struct Failure(String);

impl From<jni_0_22::errors::Error> for Failure {
    fn from(error: jni_0_22::errors::Error) -> Self {
        Self(error.to_string())
    }
}

fn logcat_error(message: &str) {
    #[link(name = "log")]
    unsafe extern "C" {
        safe fn __android_log_write(priority: i32, tag: crate::ffi::CPath<'_>, text: crate::ffi::CPath<'_>) -> i32;
    }
    const ANDROID_LOG_ERROR: i32 = 6;
    let Ok(text) = std::ffi::CString::new(message.replace('\0', " ")) else { return };
    // Both paths are live NUL-terminated strings for the synchronous call.
    __android_log_write(ANDROID_LOG_ERROR, crate::ffi::CPath::new(c"Kiwix"), crate::ffi::CPath::new(&text));
}
