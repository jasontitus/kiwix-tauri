//! Android runtime probe for kiwix-tauri's HTTPS tracker path.
//! Compiles the app's exact `android_tls.rs` and drives real librqbit 9.1.0.
#[path = "../../../../src-tauri/src/android_tls.rs"]
mod android_tls;
#[path = "../../../../src-tauri/src/ffi.rs"]
mod ffi;

use jni_0_22::{objects::{JObject, JString}, sys, EnvUnowned};
use std::{sync::{Mutex, Once}, time::Duration};

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn note(line: impl Into<String>) {
    let line: String = line.into();
    logcat(&line);
    EVENTS.lock().unwrap().push(line);
}
fn logcat(line: &str) {
    #[link(name = "log")]
    extern "C" { fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32; }
    let text = std::ffi::CString::new(line.replace('\0', " ")).unwrap();
    unsafe { __android_log_write(4, c"KiwixProbeLive".as_ptr(), text.as_ptr()); }
}

async fn direct(url: &str) -> String {
    // Built exactly as librqbit's Session builds its client (no custom roots).
    let client = reqwest::Client::builder().user_agent("probe-direct").build().unwrap();
    let task = tokio::spawn({
        let url = url.to_string();
        async move { client.get(url).send().await.map(|r| r.status().as_u16()) }
    });
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(Duration::from_secs(45), task).await;
    let elapsed = started.elapsed().as_millis();
    let Ok(outcome) = outcome else { return format!("NO RESULT after {elapsed} ms (request never completed)"); };
    let text = match outcome {
        Ok(Ok(status)) => format!("HTTP {status}"),
        Ok(Err(error)) => format!("request error: {error:?}"),
        Err(join) if join.is_panic() => "PANICKED (task aborted)".into(),
        Err(join) => format!("join error: {join}"),
    };
    format!("{text} [{elapsed} ms]")
}

async fn librqbit_announce(dir: String, port: u16) -> String {
    use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    let opts = SessionOptions {
        persistence: None, fastresume: false, dht: None, listen: None,
        disable_local_service_discovery: true,
        client_name_and_version: Some("Kiwix Workbench".into()),
        ..Default::default()
    };
    let session = match Session::new_with_opts(dir.into(), opts).await {
        Ok(session) => session,
        Err(error) => return format!("session error: {error:#}"),
    };
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&tr=https%3A%2F%2F10.0.2.2%3A{port}%2Fannounce",
        "c9e15763f722f23e98a29decdfae341b98d53056");
    let add = session.add_torrent(AddTorrent::from_url(magnet),
        Some(AddTorrentOptions { list_only: true, ..Default::default() }));
    // No peer exists; the announce is observed on the tracker side.
    match tokio::time::timeout(Duration::from_secs(120), add).await {
        Err(_) => "magnet resolution pending after 120s (expected: tracker has no peers)".into(),
        Ok(Ok(_)) => "magnet resolved".into(),
        Ok(Err(error)) => format!("add error: {error:#}"),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_kiwix_probe_MainActivity_run<'local>(
    mut unowned: EnvUnowned<'local>, _this: JObject<'local>, context: JObject<'local>,
    mode: JString<'local>, dir: JString<'local>,
) -> sys::jstring {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        struct Logcat;
        impl std::io::Write for Logcat {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let text = String::from_utf8_lossy(buf);
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    let chars: Vec<char> = line.chars().collect();
                    for chunk in chars.chunks(300) { logcat(&format!("trace {}", chunk.iter().collect::<String>())); }
                }
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("librqbit_tracker_comms=trace,librqbit::session=debug"))
            .with_ansi(false).without_time().with_writer(|| Logcat).try_init();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            note(format!("PANIC: {info}"));
            logcat(&format!("  on thread {:?}", std::thread::current().name()));
            previous(info);
        }));
    });
    unowned.with_env(|env| -> Result<sys::jstring, jni_0_22::errors::Error> {
        let mode = mode.try_to_string(env)?;
        let dir = dir.try_to_string(env)?;
        // Simulate Tao: publish the JavaVM and a process-lifetime global ref to
        // the application Context before any app code runs.
        let vm = env.get_java_vm()?;
        let global = env.new_global_ref(&context)?;
        unsafe { ndk_context::initialize_android_context(vm.get_raw().cast(), global.as_raw().cast()) };
        std::mem::forget(global);

        let mut out = vec![format!("mode={mode}")];
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        if mode == "init" {
            // As in the app's TorrentManager::engine(): a Tokio worker that is
            // not attached to the JVM.
            let calls = runtime.block_on(async {
                tokio::spawn(async {
                    let first = android_tls::ensure_platform_verifier();
                    let second = android_tls::ensure_platform_verifier();
                    (first, second, std::thread::current().name().map(str::to_owned))
                }).await
            }).unwrap();
            out.push(format!("ensure_platform_verifier on {:?}: {:?}", calls.2, calls.0));
            out.push(format!("ensure again (idempotent): {:?}", calls.1));
            if calls.0.is_err() {
                return Ok(env.new_string(out.join("\n"))?.into_raw());
            }
        }
        runtime.block_on(async {
            for (label, url) in [
                ("trusted+crl  ", "https://10.0.2.2:8443/announce?info_hash=direct"),
                ("trusted+norev", "https://10.0.2.2:8445/announce?info_hash=direct"),
                ("untrusted    ", "https://10.0.2.2:8444/announce?info_hash=direct"),
            ] {
                let line = format!("direct {label}: {}", direct(url).await);
                logcat(&line);
                out.push(line);
            }
            let task = tokio::spawn(librqbit_announce(format!("{dir}/a"), 8443));
            let line = match task.await {
                Ok(text) => format!("librqbit 8443: {text}"),
                Err(join) if join.is_panic() => "librqbit 8443: add_torrent PANICKED (task aborted)".into(),
                Err(join) => format!("librqbit 8443: join error {join}"),
            };
            logcat(&line);
            out.push(line);
        });
        runtime.shutdown_timeout(Duration::from_secs(2));
        out.extend(EVENTS.lock().unwrap().drain(..).map(|e| e.chars().take(300).collect()));
        let text = out.join("\n");
        Ok(env.new_string(text)?.into_raw())
    }).resolve::<jni_0_22::errors::ThrowRuntimeExAndDefault>()
}
