// SPDX-License-Identifier: GPL-3.0-or-later
//! Native platform adaptation; the existing Service remains the sole engine.
use crate::service::mobile_policy::BACKGROUND_CHECKPOINT_WINDOW;
use crate::service::Service;
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}, time::Duration};
#[cfg(target_os = "ios")]
use std::ffi::CString;
use tauri::{Emitter, Manager};
use tauri_plugin_kiwix_mobile::MobileExt;

pub(crate) struct BackgroundRequest {
    pub id: String,
    pub generation: u64,
    pub deadline: tokio::time::Instant,
    pub acknowledged: bool,
    pub error: Option<String>,
}
pub(crate) struct IncomingRequest {
    pub id: String,
    pub uris: Vec<String>,
    pub native_id: Option<String>,
}

pub(crate) fn incoming_summary(request: &IncomingRequest) -> Value {
    let file_names: Vec<_> = request.uris.iter().take(16).filter_map(|uri| {
        let url = url::Url::parse(uri).ok()?;
        if url.scheme() != "file" { return None; }
        let path = url.to_file_path().ok()?;
        let name = path.file_name()?.to_str()?;
        Some(name.chars().filter(|character| !character.is_control()).take(255).collect::<String>())
    }).collect();
    #[cfg(target_os = "ios")]
    let can_read_originals = request.uris.iter().all(|uri| {
        unsafe extern "C" {
            safe fn kiwix_mobile_incoming_can_open_in_place(uri: crate::ffi::CPath<'_>) -> i32;
        }
        // Advisory UI state, not admission. The native action rechecks the
        // original handoff and privacy policy after the user confirms.
        let Ok(uri) = CString::new(uri.as_bytes()) else { return false; };
        kiwix_mobile_incoming_can_open_in_place(crate::ffi::CPath::new(&uri)) == 1
    });
    #[cfg(not(target_os = "ios"))]
    let can_read_originals = false;
    json!({
        "requestId": request.id,
        "count": request.uris.len(),
        "fileNames": file_names,
        "canReadOriginals": can_read_originals,
    })
}
pub(crate) struct EntryRequest {
    pub id: String,
    pub kind: String,
    pub value: String,
}
#[cfg(target_os = "android")]
#[derive(Clone, Copy, Default)]
struct AndroidWindowBinding {
    activity: Option<i64>,
    window: Option<i64>,
    admitted: bool,
}
pub(crate) struct Lifecycle {
    pub desired_active: bool,
    pub active: bool,
    pub inactive: bool,
    pub generation: u64,
    pub retirement_revision: u64,
    pub checkpoint: Option<u64>,
    pub checkpoint_transition: Option<u64>,
    #[cfg(target_os = "ios")]
    pub checkpoint_retiring: bool,
    pub background: Option<BackgroundRequest>,
    pub incoming: VecDeque<IncomingRequest>,
    pub entries: VecDeque<EntryRequest>,
}
#[cfg(target_os = "ios")]
pub(crate) struct TransferRetirement {
    pub task: tokio::task::JoinHandle<()>,
    pub done: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}
pub(crate) struct MobileState {
    pub state: Mutex<Lifecycle>,
    pub transfer_gate: tokio::sync::Mutex<()>,
    pub incoming_gate: tokio::sync::Mutex<()>,
    pub serial: tokio::sync::Mutex<()>,
    #[cfg(target_os = "ios")]
    pub transfer_retirements: tokio::sync::Mutex<Vec<TransferRetirement>>,
    #[cfg(target_os = "ios")]
    pub activation_retry: AtomicBool,
    #[cfg(target_os = "ios")]
    pub sharing_policy_retry: AtomicBool,
    pub wake: tokio::sync::Notify,
    #[cfg(target_os = "android")]
    pub foreground_transfers: AtomicBool,
    /// Reader origins survived a background transition because native
    /// read-aloud (Android) or audible archive media/PiP (iOS) is still holding
    /// the document. The first terminal state must retire them exactly once
    /// before the next foreground snapshot can reuse the authority.
    pub media_retained: AtomicBool,
    /// The active reader's last reported media surfaces. Only the trusted root
    /// shell may set these, and only on iOS, where the background audio mode
    /// keeps the process and its reader server alive.
    #[cfg(target_os = "ios")]
    pub reader_audible: AtomicBool,
    #[cfg(target_os = "ios")]
    pub reader_picture_in_picture: AtomicBool,
    #[cfg(target_os="android")]
    pub policy_attempt_generation: std::sync::atomic::AtomicU64,
    #[cfg(target_os="android")]
    pub transfer_signal_generation: std::sync::atomic::AtomicU64,
    #[cfg(target_os="android")]
    pub transfer_reconciled_generation: std::sync::atomic::AtomicU64,
    #[cfg(target_os = "android")]
    pub transfer_events_gate: tokio::sync::Mutex<()>,
    /// Serializes the one Android 17 local-network permission sheet: a second
    /// ActivityResult launch would replace the first request's callback.
    #[cfg(target_os = "android")]
    pub local_network_gate: tokio::sync::Mutex<()>,
    /// Automatic status polls may show the sheet once per process, never on
    /// every refresh after the user denies it.
    #[cfg(target_os = "android")]
    pub local_network_prompted: AtomicBool,
    #[cfg(target_os = "android")]
    android_window_gate: tokio::sync::Mutex<()>,
    #[cfg(target_os = "android")]
    android_window: Mutex<AndroidWindowBinding>,
    #[cfg(target_os = "android")]
    pub(crate) android_ui_attached: AtomicBool,
}
impl Default for MobileState {
    fn default() -> Self {
        Self {
            #[cfg(not(target_os = "ios"))]
            state: Mutex::new(Lifecycle { desired_active: true, active: true, inactive: false, generation: 1, retirement_revision: 0, background: None, incoming: VecDeque::new(), entries: VecDeque::new(), checkpoint: None, checkpoint_transition: None }),
            #[cfg(target_os = "ios")]
            state: Mutex::new(Lifecycle { desired_active: true, active: true, inactive: false, generation: 1, retirement_revision: 0, background: None, incoming: VecDeque::new(), entries: VecDeque::new(), checkpoint: None, checkpoint_transition: None, checkpoint_retiring: false }),
            serial: tokio::sync::Mutex::new(()),
            #[cfg(target_os = "ios")]
            transfer_retirements: tokio::sync::Mutex::new(Vec::new()),
            #[cfg(target_os = "ios")]
            activation_retry: AtomicBool::new(false),
            #[cfg(target_os = "ios")]
            sharing_policy_retry: AtomicBool::new(false),
            transfer_gate: tokio::sync::Mutex::new(()),
            incoming_gate: tokio::sync::Mutex::new(()),
            wake: tokio::sync::Notify::new(),
            #[cfg(target_os = "android")]
            foreground_transfers: AtomicBool::new(false),
            media_retained: AtomicBool::new(false),
            #[cfg(target_os = "ios")]
            reader_audible: AtomicBool::new(false),
            #[cfg(target_os = "ios")]
            reader_picture_in_picture: AtomicBool::new(false),
            #[cfg(target_os="android")]
            policy_attempt_generation: std::sync::atomic::AtomicU64::new(0),
            #[cfg(target_os="android")]
            transfer_signal_generation: std::sync::atomic::AtomicU64::new(1),
            #[cfg(target_os="android")]
            transfer_reconciled_generation: std::sync::atomic::AtomicU64::new(0),
            #[cfg(target_os="android")]
            transfer_events_gate: tokio::sync::Mutex::new(()),
            #[cfg(target_os = "android")]
            local_network_gate: tokio::sync::Mutex::new(()),
            #[cfg(target_os = "android")]
            local_network_prompted: AtomicBool::new(false),
            #[cfg(target_os = "android")]
            android_window_gate: tokio::sync::Mutex::new(()),
            #[cfg(target_os = "android")]
            android_window: Mutex::new(AndroidWindowBinding::default()),
            #[cfg(target_os = "android")]
            android_ui_attached: AtomicBool::new(true),
        }
    }
}

pub(crate) fn emit(service: &Service, event: &str, payload: Value) {
    if let Ok(app) = service.app.lock() {
        if let Some(app) = app.as_ref() { let _ = app.emit_to("main", event, payload); }
    }
}
pub(crate) fn error(service: &Service, operation: &str, message: String) {
    #[cfg(target_os = "android")]
    eprintln!("Kiwix mobile {operation}: {message}");
    emit(service, "kiwix-mobile-error", json!({"operation": operation, "error": message}));
}

fn lifecycle_generation(service: &Service, active: bool, checkpoint_transition: Option<u64>) -> Option<u64> {
    if service.mobile_terminal() { return None; }
    let Ok(mut state) = service.mobile.state.lock() else { return None; };
    if active { state.inactive = false; }
    if state.desired_active == active && (active || state.checkpoint_transition == checkpoint_transition) {
        return if active && !state.active { Some(state.generation) } else { None };
    }
    state.generation += 1;
    let generation = state.generation;
    state.checkpoint = None;
    if active {
        state.background = None;
    } else {
        #[cfg(target_os = "android")]
        let acknowledged = !service.mobile.android_ui_attached.load(Ordering::SeqCst);
        #[cfg(not(target_os = "android"))]
        let acknowledged = false;
        // Publish bounded write ownership before closing foreground admission;
        // an already-dispatched checkpoint must never wait for foreground.
        state.background = Some(BackgroundRequest {
            id: uuid::Uuid::new_v4().to_string(),
            generation,
            deadline: tokio::time::Instant::now() + BACKGROUND_CHECKPOINT_WINDOW,
            acknowledged,
            error: None,
        });
    }
    state.desired_active = active;
    state.checkpoint_transition = if active { None } else { checkpoint_transition };
    drop(state);
    if !active { emit(service, "kiwix-mobile-background", json!({"generation": generation})); }
    // iOS and Android retain an active native read-aloud session across the
    // background transition; its terminal state retires the source instead.
    #[cfg(target_os = "ios")]
    if !active && !service.mobile_speech_retained() { service.mobile_speech_inactive(); }
    service.mobile.wake.notify_waiters();
    Some(generation)
}

#[cfg(target_os = "ios")]
fn request_lifecycle(service: Arc<Service>, active: bool, checkpoint_transition: Option<u64>) {
    let Some(generation) = lifecycle_generation(&service, active, checkpoint_transition) else { return; };
    tauri::async_runtime::spawn(async move {
        if let Err(message) = service.clone().mobile_transition(generation, active).await {
            #[cfg(target_os = "ios")]
            if active && !service.mobile_active() { service.mobile.activation_retry.store(true, Ordering::SeqCst); }
            error(&service, "lifecycle", message);
        }
    });
}

#[cfg(target_os = "ios")]
pub(crate) fn retry_activation(service: Arc<Service>) {
    if service.mobile_terminal() || !service.mobile.activation_retry.swap(false, Ordering::SeqCst) { return; }
    let generation = {
        let Ok(state) = service.mobile.state.lock() else {
            service.mobile.activation_retry.store(true, Ordering::SeqCst);
            return;
        };
        if !state.desired_active || state.active { return; }
        state.generation
    };
    tauri::async_runtime::spawn(async move {
        let needed = service.mobile.state.lock().is_ok_and(|state|
            state.generation == generation && state.desired_active && !state.active);
        if !needed { return; }
        if let Err(message) = service.clone().mobile_transition(generation, true).await {
            if !service.mobile_active() { service.mobile.activation_retry.store(true, Ordering::SeqCst); }
            error(&service, "foreground-retirement", message);
        }
    });
}

#[cfg(target_os = "android")]
async fn android_transition(service: &Arc<Service>, active: bool) -> Result<(), String> {
    if let Some(generation) = lifecycle_generation(service, active, None) {
        service.clone().mobile_transition(generation, active).await?;
    }
    Ok(())
}

#[cfg(target_os = "android")]
fn android_window_script(token: &str) -> String {
    format!("Object.defineProperty(window, '__KIWIX_ANDROID_WINDOW_INCARNATION__', {{value: {}}});",
        serde_json::to_string(token).expect("Window incarnation is serializable"))
}

/// Create the canonical Android main window before any fallible startup work.
/// The window is what paints the boot-guard panel, so it must exist even when
/// the Service never starts; Rust-side command admission stays closed until
/// `android_open_window` registers this incarnation after startup succeeds.
#[cfg(target_os = "android")]
fn build_android_main_window(app: &tauri::App, token: &str) -> Result<(), String> {
    let config = app.config().app.windows.iter().find(|config| config.label == "main")
        .ok_or("Canonical Android main window configuration missing")?;
    tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|error| error.to_string())?
        .initialization_script(android_window_script(token))
        .build().map_err(|error| error.to_string())?;
    Ok(())
}

/// Finish only the Activity when native startup never produced a Service.
/// Android owns process lifetime, so tao's terminal run path stays prevented
/// and the platform bridge ends the Activity instead.
#[cfg(target_os = "android")]
fn finish_activity_without_service(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(mobile) = app.try_state::<tauri_plugin_kiwix_mobile::Mobile<tauri::Wry>>() else {
            eprintln!("Kiwix mobile finish-activity: native bridge unavailable");
            return;
        };
        let id = match mobile.call("activityState", json!({})).await {
            Ok(state) => state.get("id").and_then(Value::as_i64),
            Err(message) => { eprintln!("Kiwix mobile finish-activity: {message}"); return; }
        };
        if let Err(message) = mobile.call("finishActivity", json!({"id": id})).await {
            eprintln!("Kiwix mobile finish-activity: {message}");
        }
    });
}

#[cfg(target_os = "android")]
fn reconcile_android_window(service: Arc<Service>) {
    tauri::async_runtime::spawn(async move {
        if let Err(message) = reconcile_android_window_inner(service.clone()).await {
            error(&service, "activity-window", message);
        }
    });
}

#[cfg(target_os = "android")]
async fn reconcile_android_window_inner(service: Arc<Service>) -> Result<(), String> {
    let _gate = service.mobile.android_window_gate.lock().await;
    if service.mobile_terminal() { return Ok(()); }
    let app = service.app.lock().map_err(|_| "Application handle unavailable")?.clone()
        .ok_or("Application handle unavailable")?;
    let current = service.mobile_call("activityState", json!({})).await?;
    let activity = current.get("id").and_then(Value::as_i64);
    let window = current.get("windowId").and_then(Value::as_i64);
    let visible = current.get("visible").and_then(Value::as_bool).ok_or("Native Activity visibility missing")?;
    let foreground = current.get("foreground").and_then(Value::as_bool).ok_or("Native Activity foreground state missing")?;
    let previous = *service.mobile.android_window.lock().map_err(|_| "Android window binding unavailable")?;
    let present = app.get_webview_window("main").is_some();

    if present && window.is_some() && previous.window == window {
        let replacing = previous.activity != activity || !previous.admitted;
        let token = service.android_window_token().ok_or("Android window incarnation missing")?;
        if replacing {
            // Configuration keeps the logical Wry window, but its new Java
            // document has a fresh immutable source generation.
            service.mobile.android_ui_attached.store(false, Ordering::SeqCst);
            if let Some(id) = previous.activity {
                service.mobile_call("retireWindow", json!({"id":id})).await?;
            }
            service.clone().android_retire_window(&token).await?;
            android_transition(&service, false).await?;
        }
        android_transition(&service, foreground).await?;
        if visible {
            service.mobile_note_interactive();
            service.mobile_call("openWindow", json!({"id":activity})).await?;
            if replacing {
                service.android_open_window(token, activity.ok_or("Android Activity identity missing")?)?;
            }
            service.mobile.android_ui_attached.store(true, Ordering::SeqCst);
            // A configuration-created shell can already be listening while
            // the former source drains. Wake it only after its own pair opens.
            if replacing { emit(&service, "kiwix-mobile-active", json!({})); }
        } else if foreground {
            service.mobile_note_inactive();
        }
        *service.mobile.android_window.lock().map_err(|_| "Android window binding unavailable")? =
            AndroidWindowBinding { activity, window, admitted: !replacing || visible };
        return Ok(());
    }

    service.mobile.android_ui_attached.store(false, Ordering::SeqCst);
    if let Some(token) = service.android_window_token() {
        if let Some(id) = previous.activity {
            service.mobile_call("retireWindow", json!({"id":id})).await?;
        }
        service.clone().android_retire_window(&token).await?;
    }
    android_transition(&service, false).await?;
    // An overlapping replacement may resume before Tao has removed the old
    // window. Its Destroyed callback reconciles again; never create a duplicate.
    if present { return Ok(()); }
    let current = service.mobile_call("activityState", json!({})).await?;
    let activity = current.get("id").and_then(Value::as_i64);
    let window = current.get("windowId").and_then(Value::as_i64);
    if !current.get("visible").and_then(Value::as_bool).unwrap_or(false)
        || activity.is_none() || window.is_none() { return Ok(()); }
    android_transition(&service, true).await?;
    let token = uuid::Uuid::new_v4().to_string();
    service.mobile_call("openWindow", json!({"id":activity})).await?;
    service.android_open_window(token.clone(), activity.ok_or("Android Activity identity missing")?)?;
    service.mobile.android_ui_attached.store(true, Ordering::SeqCst);
    *service.mobile.android_window.lock().map_err(|_| "Android window binding unavailable")? =
        AndroidWindowBinding { activity, window, admitted: true };
    let build_app = app.clone();
    let script = android_window_script(&token);
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let current = build_app.kiwix_mobile().call_sync("activityState", json!({}))?;
            if current.get("id").and_then(Value::as_i64) != activity
                || current.get("windowId").and_then(Value::as_i64) != window
                || current.get("visible").and_then(Value::as_bool) != Some(true) {
                return Err("Android Activity changed before window construction".to_string());
            }
            let config = build_app.config().app.windows.iter().find(|config| config.label == "main")
                .ok_or_else(|| "Canonical main window configuration missing".to_string())?;
            tauri::WebviewWindowBuilder::from_config(&build_app, config)
                .map_err(|error| error.to_string())?
                .initialization_script(script).build().map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        })();
        let _ = send.send(result);
    }).map_err(|error| error.to_string())?;
    let result = receive.await.map_err(|_| "Android window construction stopped".to_string())?;
    if let Err(message) = result {
        service.mobile.android_ui_attached.store(false, Ordering::SeqCst);
        service.mobile_call("retireWindow", json!({"id":activity})).await?;
        service.clone().android_retire_window(&token).await?;
        return Err(message);
    }
    Ok(())
}

pub(crate) async fn drain_incoming(_service: Arc<Service>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let result = _service.mobile_call("takeIncomingDocuments", json!({})).await?;
        let batches = result.get("batches").and_then(Value::as_array).ok_or("Native incoming document queue is invalid")?;
        for batch in batches {
            let id = batch.get("requestId").and_then(Value::as_str).ok_or("Native incoming batch has no private ID")?;
            let uris: Vec<String> = serde_json::from_value(batch.get("uris").cloned().ok_or("Native incoming batch has no document identifiers")?).map_err(|e| e.to_string())?;
            _service.mobile_queue_native_incoming(id, uris)?;
        }
    }
    Ok(())
}

pub(crate) async fn drain_entries(_service: Arc<Service>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    _service.mobile_drain_native_entries().await?;
    Ok(())
}

async fn register_events(service: Arc<Service>) -> Result<(), String> {
    let app = service.app.lock().map_err(|_| "Application handle unavailable")?.clone().ok_or("Application handle unavailable")?;
    for event in ["document-progress", "camera-revoked", "checkpoint-expired", "checkpoint-retiring", "incoming-documents", "app-entry", "completion-notification-tap", "mobile-inactive", "mobile-suspended", "mobile-resumed", "transfers-pause", "transfers-timeout", "transfers-error", "hardware-shortcut", "network-policy-changed", "speech-state", "speech-action", "reader-audio", "dynamic-type", "hotspot-stopped", "nearby-bluetooth", "nearby-join"] {
        let service = service.clone();
        app.kiwix_mobile().listen(event, move |payload| {
            match event {
                "nearby-bluetooth" => service.nearby_bluetooth_event(payload),
                "nearby-join" => service.nearby_join_event(payload),
                "speech-state" => service.mobile_speech_event(payload),
                "speech-action" => service.mobile_speech_action(payload),
                "dynamic-type" => emit(&service, "kiwix-mobile-dynamic-type", payload),
                "reader-audio" => {
                    // A native interruption, route loss or failed activation is
                    // terminal for the audible surface even if the web content
                    // is throttled and cannot report its pause.
                    #[cfg(target_os = "ios")]
                    service.mobile_audio_event(&payload);
                    emit(&service, "kiwix-mobile-audio", payload);
                }
                "hotspot-stopped" => {
                    // The platform closed the reservation without a Rust Stop
                    // (Wi-Fi off, tethering policy, Activity retirement). Drop
                    // the stale credentials so the UI cannot advertise them,
                    // and retire a listener that lived on the gone AP link.
                    #[cfg(target_os = "android")]
                    {
                        service.mobile_hotspot_lost();
                        let service = service.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(message) = service.mobile_hotspot_retire_sharing().await {
                                error(&service, "hotspot", message);
                            }
                        });
                    }
                }
                "network-policy-changed" => {
                    if let Err(message) = service.mobile_network_event(payload) {
                        error(&service, event, message);
                    }
                    let service = service.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(message) = service.mobile_network_reconcile().await {
                            error(&service, "network-policy", message);
                            return;
                        }
                        if let Err(message) = service.mobile_resume_after_network().await {
                            error(&service, "network-resume", message);
                        }
                    });
                }
                "document-progress" => emit(&service, "kiwix-mobile-document-progress", payload),
                "hardware-shortcut" => {
                    if !service.mobile_active() { return; }
                    let Some(action)=payload.get("action").and_then(Value::as_str) else { return; };
                    if matches!(action,"import"|"new-tab"|"reopen-tab"|"close-tab"|"search"|"find"|"print"|"save"|"bookmark"|"zoom-in"|"zoom-out"|"zoom-reset"|"next-tab"|"previous-tab"|"back"|"forward"|"fullscreen"|"escape")
                        || action.strip_prefix("select-tab-").is_some_and(|number|matches!(number,"0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"))
                    {
                        emit(&service,"kiwix-mobile-shortcut",json!({"action":action}));
                    }
                }
                "camera-revoked" => {
                    service.mobile_deactivate_sensors();
                    #[cfg(target_os="android")]
                    reconcile_android_window(service.clone());
                }
                "checkpoint-expired" | "checkpoint-retiring" => {
                    service.mobile_expire_checkpoint(&payload, event == "checkpoint-expired");
                }
                "mobile-inactive" => {
                    // Permission/system UI is not an OS background transition.
                    // Existing sensor authorization remains alive; only new
                    // command and native permission admissions are held.
                    if let Ok(mut state) = service.mobile.state.lock() { state.inactive = true; }
                    #[cfg(target_os = "ios")]
                    if !service.mobile_speech_retained() { service.mobile_speech_inactive(); }
                }
                "mobile-suspended" => {
                    #[cfg(target_os="ios")]
                    let transition = match payload.get("transition").and_then(Value::as_u64).filter(|value| *value > 0) {
                        Some(transition) => Some(transition),
                        None => { error(&service,event,"Native suspension has no checkpoint transition identity".into()); return; }
                    };
                    #[cfg(target_os="ios")]
                    request_lifecycle(service.clone(), false, transition);
                    #[cfg(target_os="android")]
                    reconcile_android_window(service.clone());
                }
                "mobile-resumed" => {
                    // A resumed Activity is interactive: clear any stale
                    // inactive flag before reconcile decides whether a full
                    // lifecycle transition is still required.
                    #[cfg(target_os="android")]
                    service.mobile_note_interactive();
                    #[cfg(target_os="ios")]
                    request_lifecycle(service.clone(), true, None);
                    #[cfg(target_os="android")]
                    reconcile_android_window(service.clone());
                }
                "incoming-documents" => {
                    let service = service.clone();
                    tauri::async_runtime::spawn(async move { if let Err(message) = drain_incoming(service.clone()).await { error(&service, "incoming", message); } });
                }
                "completion-notification-tap" => service.notify_library_changed(),
                "app-entry" => {
                    let service = service.clone();
                    tauri::async_runtime::spawn(async move { if let Err(message) = drain_entries(service.clone()).await { error(&service, "entry", message); } });
                }
                "transfers-pause" | "transfers-timeout" | "transfers-error" => {
                    #[cfg(target_os="android")]
                    {
                        service.mobile.transfer_signal_generation.fetch_add(1, Ordering::SeqCst);
                        service.mobile.foreground_transfers.store(false, Ordering::SeqCst);
                        let service = service.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(message)=service.mobile_reconcile_transfer_events().await {error(&service,event,message);}
                        });
                    }
                    #[cfg(target_os="ios")]
                    error(&service,event,format!("Unexpected foreground-transfer event: {payload}"));
                }
                _ => {}
            }
        }).await?;
    }
    service.nearby_listeners_ready();
    let network = service.mobile_call("networkState", json!({})).await?;
    service.mobile_network_event(network)?;
    service.mobile_network_reconcile().await?;
    service.mobile_bootstrap_archive_backup().await?;
    #[cfg(target_os="android")]
    service.mobile_reconcile_transfer_events().await?;
    service.mobile_resume_after_network().await?;
    drain_incoming(service.clone()).await?;
    drain_entries(service).await
}

/// Ticks between full reconciliations while nothing is in flight. Lifecycle and
/// transfer visibility stay on the fast tick; the library, download and map
/// reconciliation that clones state and re-serializes payloads only runs while
/// the previous pass observed work, after a lifecycle or transfer signal, or
/// once per idle window.
const IDLE_RECONCILE_TICKS: u64 = 5;

/// Cadence of the full reconciliation pass in the mobile maintenance loop.
struct Reconciliation {
    tick: u64,
    /// Lifecycle generation the last full pass observed.
    generation: u64,
    /// The last full pass saw work in flight.
    busy: bool,
}
impl Reconciliation {
    fn new() -> Self { Self { tick: 0, generation: 0, busy: false } }
    /// A signal (lifecycle transition, pending native transfer event, retry
    /// flag) must be observed on this tick, not on the idle cadence.
    fn due(&mut self, signaled: bool) -> bool {
        let due = self.busy || signaled || self.tick.is_multiple_of(IDLE_RECONCILE_TICKS);
        self.tick = self.tick.wrapping_add(1);
        due
    }
    fn observed(&mut self, generation: u64, busy: bool) {
        self.generation = generation;
        self.busy = busy;
    }
}

/// A transfer phase that still owns a worker.
fn pending_phase(status: &str) -> bool {
    matches!(status, "queued" | "connecting" | "resolving" | "initializing" | "metadata"
        | "checking" | "downloading" | "verifying" | "publishing" | "composing" | "validating")
}

/// Download rows that still owe work. Completed, failed and paused rows stay in
/// the list — a paused row waits for an explicit user or network event — so only
/// a live transfer phase or an unfinished verification keeps the fast cadence.
fn download_work_pending(rows: &Value) -> bool {
    rows.as_array().is_some_and(|rows| rows.iter().any(|row| {
        row.get("status").and_then(Value::as_str).is_some_and(pending_phase)
            || row.get("importState").and_then(Value::as_str).is_some_and(|state| matches!(state, "queued" | "verifying"))
    }))
}

/// A map job still owns a worker.
fn map_work_pending(status: &Value) -> bool {
    status.get("status").and_then(Value::as_str).is_some_and(pending_phase)
        || status.get("importState").and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "queued" | "verifying"))
}

async fn maintain(service: Arc<Service>) {
    let mut reconciliation = Reconciliation::new();
    loop {
        tokio::time::sleep(Duration::from_secs(if cfg!(target_os = "ios") { 1 } else { 3 })).await;
        if service.mobile_terminal() { break; }
        let generation = service.mobile.state.lock().map(|state| state.generation).unwrap_or(0);
        #[cfg(target_os = "ios")]
        let signaled = generation != reconciliation.generation
            || service.mobile.activation_retry.load(Ordering::SeqCst)
            || service.mobile.sharing_policy_retry.load(Ordering::SeqCst);
        #[cfg(target_os = "android")]
        let signaled = generation != reconciliation.generation
            || service.mobile.transfer_signal_generation.load(Ordering::SeqCst)
                != service.mobile.transfer_reconciled_generation.load(Ordering::SeqCst);
        #[cfg(target_os = "ios")]
        retry_activation(service.clone());
        let active = service.mobile_active();
        // Native only holds a sleep lease for a transfer while the scene is
        // foreground and the process is not suspended, and its resume path
        // re-evaluates the retained value, so a suspended tick would spend a
        // bridge round trip to publish a state native ignores.
        #[cfg(target_os = "ios")]
        let foreground = service.mobile.state.lock().is_ok_and(|state| state.desired_active);
        #[cfg(target_os = "ios")]
        if foreground {
            if let Err(message) = service.mobile_local_transfer_update().await {
                error(&service, "local-transfers", message);
            }
        }
        // Runs while backgrounded too: the retirement grace period is exactly
        // when the shown transfer turns into "paused" on the lock screen. While
        // backgrounded with nothing shown there is no job to project, and a new
        // transfer cannot start there, so the acquisition snapshot is skipped.
        #[cfg(target_os = "ios")]
        if active || !service.mobile_live_activity_idle() {
            if let Err(message) = service.mobile_live_activity_update().await {
                crate::diagnostics::record(&service.store, "live-activity", Some(&message));
            }
        }
        let due = active && reconciliation.due(signaled);
        if active {
            #[cfg(target_os="android")]
            match service.mobile_reconcile_transfer_events().await {
                Ok(()) => if let Err(message)=service.mobile_resume_policy().await {error(&service,"resume-interrupted",message);},
                Err(message) => error(&service,"native-transfer-events",message),
            }
            if due {
                let mut busy = false;
                for command in ["library_maintenance", "downloads", "map_status"] {
                    match service.clone().dispatch(command, json!({"__windowLabel":"main"})).await {
                        Ok(result) => busy |= match command {
                            "downloads" => download_work_pending(&result),
                            "map_status" => map_work_pending(&result),
                            _ => false,
                        },
                        Err(message) => if service.mobile_active() { error(&service, command, message); },
                    }
                }
                reconciliation.observed(generation, busy);
            }
        }
        // The update is a no-op without a foreground transfer; the idle tick
        // still runs it so a snapshot failure pauses transfers as before.
        #[cfg(target_os = "android")]
        if service.mobile.foreground_transfers.load(Ordering::SeqCst) || due {
            if let Err(message) = service.mobile_transfer_update().await {
                error(&service, "transfers", message);
                if let Err(message) = service.mobile_pause_transfers().await { error(&service, "transfers", message); }
            }
        }
    }
}

/// Why native startup failed, kept so the window can act on it.
struct StartupFailure {
    message: String,
    data_dir: Option<std::path::PathBuf>,
}

/// Actions the boot-guard panel offers while native startup is incomplete.
fn startup_failure_actions() -> Value {
    json!([
        {"label": "Reset saved window state", "action": "reset_window_state"},
        {"label": "Quit", "action": "quit"}
    ])
}

/// Paint the boot-guard diagnostic panel in the existing window. The boot guard
/// is a classic script, so it is installed even when the application bundle
/// never runs; the eval is repeated because an early one can land before the
/// page commits, and the panel render is idempotent.
fn show_startup_failure(app: &tauri::AppHandle, message: &str) {
    let payload = json!({
        "source": "native",
        "reason": message,
        "window": "main",
        // The setup closure runs once per process; a webview reload cannot
        // recover a native startup failure. Only reset/quit are offered.
        "allowReload": false,
        "actions": startup_failure_actions()
    });
    let script = format!("window.__kiwixShowStartupFailure && window.__kiwixShowStartupFailure({payload});");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for _ in 0..12 {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval(&script);
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
    });
}

/// Only reachable while the Service could not be created; it gives the failure
/// panel a bounded, non-destructive way forward instead of a crash loop.
#[tauri::command]
fn startup_failure(app: tauri::AppHandle, action: String) -> Result<Value, String> {
    let failure = app.try_state::<StartupFailure>();
    if action == "status" {
        // The page's own fallback panel probes this: the native eval path is
        // unavailable on iOS, so the reason and actions must come from here.
        return Ok(match failure.as_deref() {
            Some(failure) => json!({
                "failed": true,
                "reason": failure.message,
                "actions": startup_failure_actions(),
                "allowReload": false
            }),
            None => json!({"failed": false}),
        });
    }
    let failure = failure.ok_or("Kiwix has already started; the startup actions are no longer available")?;
    match action.as_str() {
        "quit" => {
            app.exit(0);
            Ok(json!({"ok": true}))
        }
        "reset_window_state" => {
            let data_dir = failure.data_dir.clone().ok_or("The application storage location is unavailable")?;
            crate::store::reset_saved_windows(&data_dir)?;
            Ok(json!({
                "ok": true,
                "message": "Saved window state was reset. Quit Kiwix and open it again."
            }))
        }
        _ => Err("Unknown startup action".into()),
    }
}

pub fn run() {
    #[allow(unused_mut)]
    let mut context = tauri::generate_context!();
    #[cfg(target_os = "android")]
    let initial_token = uuid::Uuid::new_v4().to_string();
    #[cfg(target_os = "android")]
    {
        let main = context.config_mut().app.windows.iter_mut().find(|config| config.label == "main")
            .expect("Canonical Android main window configuration missing");
        main.create = false;
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_kiwix_mobile::init())
        .on_page_load(|webview, payload| {
            if webview.label() == "main" && payload.event() == tauri::webview::PageLoadEvent::Started {
                if let Some(service) = webview.try_state::<Arc<Service>>() {
                    // Initial startup must preserve cold-open invitation intake.
                    // Any later main-frame replacement retires the old source's
                    // radio/join authority before its asynchronous native drain.
                    if service.is_frontend_ready("main") { service.inner().clone().nearby_root_reload(); }
                }
            }
        })
        .on_permission_request(|webview, kind| {
            if webview.label() == "main" {
                if let Some(service) = webview.try_state::<Arc<Service>>() {
                    #[cfg(target_os = "android")]
                    if !service.mobile.android_ui_attached.load(Ordering::SeqCst) {
                        return tauri::webview::PermissionResponse::Deny;
                    }
                    if service.mobile_active() { return service.sensor_manager("main").permission_response("main", kind); }
                }
            }
            tauri::webview::PermissionResponse::Deny
        })
        .invoke_handler(tauri::generate_handler![crate::dispatch, startup_failure])
        .setup(move |app| {
            // Android's canonical window is created before any fallible startup
            // work: it is the only surface that can paint the boot-guard panel,
            // and native command admission stays closed until `start` succeeds.
            #[cfg(target_os = "android")]
            if let Err(error) = build_android_main_window(app, &initial_token) {
                // Without a window there is no surface for the boot guard, so
                // this is a builder failure, not a recoverable startup failure.
                return Err(format!("Kiwix could not create its main window. {error}").into());
            }
            #[cfg(target_os="android")]
            let profile_root = app.handle().kiwix_mobile().storage_root().map(|root| root.join("kiwix")).map_err(|error| error.to_string());
            #[cfg(target_os="ios")]
            let profile_root = app.path().app_data_dir().map_err(|error| error.to_string());
            let profile_root = match profile_root {
                Ok(root) => root,
                Err(error) => {
                    // No profile path means no Service and no reset target, but
                    // the window still exists: show a real screen, never abort.
                    let message = format!("Kiwix could not find its application storage. {error}");
                    eprintln!("Kiwix: {message}");
                    app.manage(StartupFailure { message: message.clone(), data_dir: None });
                    show_startup_failure(app.handle(), &message);
                    return Ok(());
                }
            };
            let start = |app: &tauri::App| -> Result<Arc<Service>, String> {
                // Capture panics that occur while the service starts or reads an
                // archive. Mobile profiles are app-private; no shared location.
                crate::crash::install(&profile_root, true);
                let service = Service::new(profile_root.clone(), false, false)?;
                *service.app.lock().map_err(|_| "Application handle unavailable")? = Some(app.handle().clone());
                crate::provider_reference::install(app.handle().clone())?;
                crate::transfer_policy::install(app.handle().clone())?;
                // The Wi-Fi performance lock rides the mobile plugin bridge, so the
                // platform hook needs the application handle before any transfer
                // can start.
                #[cfg(target_os = "android")]
                crate::power_activity::install(app.handle().clone());
                // Archive geolocation (permissions.rs) reaches LocationManager
                // through the same plugin bridge.
                #[cfg(all(target_os = "android", native_location))]
                crate::location_android::install(app.handle().clone());
                service.mobile_initialize()?;
                #[cfg(target_os = "ios")]
                service.initialize_notifications();
                #[cfg(target_os = "android")]
                {
                    let activity = app.handle().kiwix_mobile().call_sync("activityState", json!({})).map_err(|error| error.to_string())?;
                    service.android_open_window(initial_token.clone(),
                        activity.get("id").and_then(Value::as_i64).ok_or("Initial Android Activity identity missing")?)?;
                    *service.mobile.android_window.lock().map_err(|_| "Android window binding unavailable")? =
                        AndroidWindowBinding {
                            activity: activity.get("id").and_then(Value::as_i64),
                            window: activity.get("windowId").and_then(Value::as_i64),
                            admitted: true,
                        };
                }
                Ok(service)
            };
            match start(app) {
                Ok(service) => {
                    #[cfg(target_os = "android")]
                    {
                        let installing = service.clone();
                        tauri::async_runtime::spawn(async move { installing.prepare_bundled_voice().await });
                    }
                    app.manage(service.clone());
                    let listeners = service.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(message) = register_events(listeners.clone()).await { error(&listeners, "native-listeners", message); }
                    });
                    tauri::async_runtime::spawn(async move {
                        #[cfg(target_os = "ios")]
                        if let Err(message) = service.bootstrap_ios_acquisition().await {
                            error(&service, "background-download-recovery", message);
                        }
                        maintain(service).await;
                    });
                }
                Err(message) => {
                    // A profile that cannot be opened is a recoverable startup
                    // failure, never a process abort: the boot-guard panel keeps
                    // the app usable and offers reset/quit actions.
                    eprintln!("Kiwix: native startup failed: {message}");
                    app.manage(StartupFailure { message: message.clone(), data_dir: Some(profile_root.clone()) });
                    show_startup_failure(app.handle(), &message);
                }
            }
            Ok(())
        })
        .build(context);
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            // Builder failure is not profile state; there is no window to draw
            // in. Report and exit without a panic loop.
            eprintln!("Kiwix could not create its native application: {error}");
            std::process::exit(1);
        }
    };
    // Register the optional selector on Tao's class before UIApplicationMain
    // creates/caches its non-retained delegate. Never replace the live delegate.
    #[cfg(target_os = "ios")]
    if let Err(message) = crate::ios_background::install_delegate_handoff() {
        eprintln!("Kiwix could not initialize background delivery: {message}");
        app.manage(StartupFailure { message: message.clone(), data_dir: None });
        show_startup_failure(app.handle(), &message);
    }
    #[cfg(target_os = "ios")]
    let authorized = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "ios")]
    let exiting = Arc::new(AtomicBool::new(false));
    app.run(move |app, event| {
        #[cfg(target_os = "android")]
        if let tauri::RunEvent::ExitRequested { api, .. } = &event { api.prevent_exit(); }
        let Some(service) = app.try_state::<Arc<Service>>().map(|state| state.inner().clone()) else {
            // Native startup never produced a Service (for example the
            // boot-guard Quit action): there is no Rust admission to retire,
            // but Android still owns process lifetime. Only the Activity ends.
            #[cfg(target_os = "android")]
            if let tauri::RunEvent::ExitRequested { code, .. } = &event {
                if code.is_some() { finish_activity_without_service(app); }
            }
            return;
        };
        match event {
            #[cfg(target_os = "android")]
            tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::Suspended | tauri::WindowEvent::Resumed | tauri::WindowEvent::Destroyed, .. }
                if label == "main" => reconcile_android_window(service),
            #[cfg(target_os = "android")]
            tauri::RunEvent::Resumed => reconcile_android_window(service),
            #[cfg(target_os = "ios")]
            tauri::RunEvent::Opened { urls } => {
                let mut files = Vec::new();
                let mut links = Vec::new();
                let mut speech = Vec::new();
                for url in urls {
                    if url.scheme() == "kiwix" {
                        eprintln!("Kiwix iOS opened URL host={:?} query={}", url.host_str(),
                            if url.query().is_some() { "present" } else { "none" });
                    }
                    match url.scheme() {
                        "file" => files.push(url.to_string()),
                        "zim" => links.push(url.to_string()),
                        "kiwix" => speech.push(url),
                        _ => {}
                    }
                }
                // UIKit's document callback owns one grant per URL. Do not
                // acknowledge an accidental duplicate twice within one event.
                files.sort_unstable();
                files.dedup();
                if !files.is_empty() {
                    if let Err(message) = service.mobile_queue_incoming(files.clone()) {
                        error(&service, "incoming", message);
                        let service = service.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(message) = service.mobile_call("releaseIncomingOriginals",
                                json!({"uris":files})).await {
                                error(&service, "incoming",
                                    format!("Releasing rejected incoming document access failed: {message}"));
                            }
                        });
                    }
                }
                for url in speech {
                    if url.host_str() != Some("speech") { continue; }
                    let Some(action) = url.query_pairs()
                        .find(|(key, _)| key == "action").map(|(_, value)| value.into_owned())
                        else { continue; };
                    service.mobile_speech_action(json!({"action": action}));
                }
                if !links.is_empty() {
                    // Queue exactly like a file open; a cold launch consumes the
                    // same launch location from its first snapshot.
                    let generation = service.launch_generation();
                    tauri::async_runtime::spawn(async move {
                        match service.queue_zim_links("main", links, generation).await {
                            Ok(_) => emit(&service, "kiwix-open", json!({})),
                            Err(message) => error(&service, "deep-link", message),
                        }
                    });
                }
            }
            #[cfg(target_os = "android")]
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                // Tao's terminal run path calls libc exit, destroying process-
                // global Android render resources while their threads are live.
                // Android owns process lifetime; only the Activity may finish.
                api.prevent_exit();
                if code.is_some() {
                    let id = match app.kiwix_mobile().call_sync("activityState", json!({})) {
                        Ok(state) => state.get("id").and_then(Value::as_i64),
                        Err(message) => { error(&service, "finish-activity", message); return; }
                    };
                    tauri::async_runtime::spawn(async move {
                        if let Err(message) = service.mobile_call("finishActivity", json!({"id":id})).await {
                            error(&service, "finish-activity", message);
                        }
                        reconcile_android_window(service);
                    });
                } else {
                    reconcile_android_window(service);
                }
            }
            #[cfg(target_os = "ios")]
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if authorized.load(Ordering::SeqCst) { return; }
                api.prevent_exit();
                if exiting.swap(true, Ordering::SeqCst) { return; }
                service.mobile_begin_shutdown();
                let app = app.clone();
                let authorized = authorized.clone();
                let exiting = exiting.clone();
                tauri::async_runtime::spawn(async move {
                    let stopped = service.shutdown().await;
                    if let Err(error) = &stopped {
                        eprintln!("Mobile service retirement did not complete: {error}");
                    }
                    // The iOS native bridge never owned a separate terminal
                    // phase; authorize exit only after Service shutdown joins.
                    if stopped.is_err() {
                        exiting.store(false, Ordering::SeqCst);
                        return;
                    }
                    authorized.store(true, Ordering::SeqCst);
                    app.exit(code.unwrap_or(0));
                });
            }
            _ => {}
        }
    });
}
