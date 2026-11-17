// SPDX-License-Identifier: GPL-3.0-or-later
//! Root-only native read-aloud. No archive frame receives a native capability.
//! The synchronous ownership gate orders source retirement before any async
//! privacy/reader gate; native calls never retain this gate or a privacy lease.
use std::{collections::HashMap, sync::{Arc, Mutex}};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    window: String,
    session: String,
    archive: String,
    installation: String,
    origin: String,
    generation: u64,
}
#[derive(Clone, Debug)]
pub(super) enum Scope {
    All,
    Window(String),
    Session(String, String),
    Installation(String),
}
impl Scope {
    fn matches(&self, owner: &Owner) -> bool {
        match self {
            Self::All => true,
            Self::Window(window) => owner.window == *window,
            Self::Session(window, session) => owner.window == *window && owner.session == *session,
            Self::Installation(installation) => owner.installation == *installation,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Preparing, Prepared, Starting, Started, Retiring }
struct Registration {
    id: String,
    owner: Owner,
    token: Option<String>,
    ordinal: u64,
    phase: Phase,
    text_length: u64,
    /// True when the native registration is only an ownership capability for
    /// the cross-platform Kokoro engine; no native synthesizer was started.
    kokoro: bool,
}
impl Registration {
    fn retire_for_event(&mut self, valid: bool, speaking: bool, paused: bool) -> bool {
        if !valid || (!speaking && !paused) {
            self.phase = Phase::Retiring;
            true
        } else { false }
    }
}
#[derive(Clone)]
struct Capability { native_id: String, token: String }
#[derive(Default)]
struct State {
    registrations: HashMap<String, Registration>,
    blocks: HashMap<u64, Scope>,
    next: u64,
    /// Newest started ordinal per window. Mobile has one window; on desktop a
    /// slow start in one window must not be superseded by another window's.
    latest_started: HashMap<String, u64>,
}
impl State {
    fn blocked(&self, owner: &Owner) -> bool {
        self.blocks.values().any(|scope| scope.matches(owner))
    }
    /// Restore only the dormant native reader owner, preserving any exact
    /// started speech capability without admitting a new/replacement source.
    #[cfg(any(mobile, test))]
    fn restore_reader(&self, owner: &Owner, sensors: &crate::permissions::SensorManager) -> Result<(), String> {
        if owner.window != "main" || self.blocked(owner) { return Err("SPEECH_SOURCE_RETIRING".into()); }
        let registered = self.registrations.values().any(|entry| entry.owner.window == owner.window);
        let started = self.registrations.values().any(|entry| entry.owner == *owner &&
            matches!(entry.phase, Phase::Starting | Phase::Started) && entry.token.is_some());
        if (registered && !started) || self.registrations.values().any(|entry|
            entry.owner.window == owner.window && entry.owner != *owner && entry.phase != Phase::Retiring) {
            return Err("SPEECH_SESSION_RETIRED".into());
        }
        sensors.restore(&crate::permissions::RetainedSensorSession {
            session_id: owner.session.clone(), archive_id: owner.archive.clone(),
            installation_id: owner.installation.clone(), origin: owner.origin.clone(), generation: owner.generation,
        })
    }
    #[cfg(any(mobile, test))]
    fn restore_before_foreground(&self, owner: &Owner, sensors: &crate::permissions::SensorManager,
        publish: impl FnOnce()) -> Result<(), String> {
        self.restore_reader(owner, sensors)?;
        publish();
        Ok(())
    }
    fn reserve(&mut self, id: &str, owner: Owner) -> Result<String, String> {
        if self.blocked(&owner) { return Err("SPEECH_SOURCE_RETIRING".into()); }
        // Retiring/in-flight preparations still occupy a native slot until their
        // exact Retire acknowledgement. Cancellation cannot evade the bound.
        if self.registrations.len() >= 8 { return Err("SPEECH_BUSY".into()); }
        if self.registrations.values().any(|entry| entry.id == id && entry.owner.window == owner.window) {
            return Err("SPEECH_SESSION_REGISTERED".into());
        }
        self.next = self.next.checked_add(1).ok_or("SPEECH_SEQUENCE_EXHAUSTED")?;
        let native_id = uuid::Uuid::new_v4().to_string();
        self.registrations.insert(native_id.clone(), Registration {
            id: id.into(), owner, token: None, ordinal: self.next,
            phase: Phase::Preparing, text_length: 0, kokoro: false,
        });
        Ok(native_id)
    }
    fn retire(&mut self, scope: &Scope) -> Vec<Capability> {
        self.registrations.iter_mut().filter_map(|(native_id, entry)| {
            if !scope.matches(&entry.owner) { return None; }
            entry.phase = Phase::Retiring;
            entry.token.as_ref().map(|token| Capability { native_id: native_id.clone(), token: token.clone() })
        }).collect()
    }
    fn owned(&self, window: &str, id: &str, token: &str) -> Option<String> {
        self.registrations.iter().find_map(|(native_id, entry)| {
            (entry.owner.window == window && entry.id == id && entry.token.as_deref() == Some(token))
                .then(|| native_id.clone())
        })
    }
    fn finish_retire(&mut self, capability: &Capability) {
        if self.registrations.get(&capability.native_id).is_some_and(|entry| {
            entry.phase == Phase::Retiring && entry.token.as_deref() == Some(&capability.token)
        }) { self.registrations.remove(&capability.native_id); }
    }
    fn begin_start(&mut self, native_id: &str, owner: &Owner, length: u64) -> Result<Vec<Capability>, String> {
        if self.blocked(owner) { return Err("SPEECH_SOURCE_RETIRING".into()); }
        let entry = self.registrations.get(native_id).ok_or("SPEECH_SESSION_RETIRED")?;
        if entry.owner != *owner || entry.phase != Phase::Prepared { return Err("SPEECH_SESSION_CONSUMED".into()); }
        let latest = self.latest_started.entry(owner.window.clone()).or_default();
        if entry.ordinal < *latest { return Err("SPEECH_SESSION_SUPERSEDED".into()); }
        let ordinal = entry.ordinal;
        *latest = ordinal;
        let mut retired = Vec::new();
        for (other_id, other) in &mut self.registrations {
            if other_id != native_id && other.owner.window == owner.window && other.ordinal < ordinal {
                other.phase = Phase::Retiring;
                if let Some(token) = &other.token {
                    retired.push(Capability { native_id: other_id.clone(), token: token.clone() });
                }
            }
        }
        let entry = self.registrations.get_mut(native_id).unwrap();
        // Publish before native callbacks, including terminal-before-ack.
        entry.phase = Phase::Starting;
        entry.text_length = length;
        Ok(retired)
    }
    fn finish_start(&mut self, native_id: &str) -> bool {
        if let Some(entry) = self.registrations.get_mut(native_id) {
            if entry.phase == Phase::Starting { entry.phase = Phase::Started; return true; }
        }
        false
    }
}
/// A started read-aloud session keeps its source across background sensor
/// suspension; only a session without a live capability is retired.
#[cfg_attr(any(not(mobile), target_os = "android"), allow(dead_code))]
pub(crate) fn background_retires_speech(retained: bool) -> bool { !retained }

#[derive(Clone, Default)]
pub(super) struct SpeechManager(Arc<Mutex<State>>);
impl SpeechManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
    fn block(&self, scope: Scope) -> (Retirement, Vec<Capability>) {
        let mut state = self.lock();
        state.next = state.next.checked_add(1).expect("Speech retirement sequence exhausted");
        let key = state.next;
        let retired = state.retire(&scope);
        state.blocks.insert(key, scope);
        (Retirement { manager: self.clone(), key }, retired)
    }
}
pub(super) struct Retirement { manager: SpeechManager, key: u64 }
impl Drop for Retirement {
    fn drop(&mut self) { self.manager.lock().blocks.remove(&self.key); }
}

#[cfg(mobile)]
mod transport {
    use super::*;
    use crate::{mobile, service::{text, Service}};
    use serde_json::{json, Value};
    use tauri::Emitter;
    use tauri_plugin_kiwix_mobile::MobileExt;

    async fn cleanup(app: &tauri::AppHandle, manager: &SpeechManager, capability: &Capability) -> Result<Value, String> {
        let payload = json!({"id":capability.native_id,"token":capability.token});
        let stopped = app.kiwix_mobile().call("speechStop", payload.clone()).await;
        // Retire even after Stop failure: only Retire releases the native slot.
        let retired = app.kiwix_mobile().call("speechRetire", payload).await;
        match retired {
            Ok(_) => manager.lock().finish_retire(capability),
            Err(error) => return Err(error),
        }
        stopped
    }
    impl Service {
        fn speech_app(&self) -> Result<tauri::AppHandle, String> {
            self.app.lock().map_err(|_| "Application handle unavailable")?.clone()
                .ok_or_else(|| "Mobile native speech adapter is unavailable".into())
        }
        fn speech_cleanup(&self, capabilities: Vec<Capability>) {
            if capabilities.is_empty() { return; }
            let app = match self.speech_app() {
                Ok(app) => app,
                Err(error) => { mobile::error(self, "speech-retire", error); return; }
            };
            let neural = {
                let state = self.speech.lock();
                capabilities.iter().filter(|capability| state.registrations.get(&capability.native_id)
                    .is_some_and(|entry| entry.kokoro)).cloned().collect::<Vec<_>>()
            };
            for capability in &neural {
                self.read_aloud_activity(json!({"state": "terminal", "sessionId": capability.native_id}));
                self.kokoro.stop(&capability.native_id, &capability.token);
            }
            let manager = self.speech.clone();
            tauri::async_runtime::spawn(async move {
                for capability in capabilities {
                    if let Err(error) = cleanup(&app, &manager, &capability).await {
                        let _ = app.emit_to("main", "kiwix-mobile-error", json!({"operation":"speech-retire","error":error}));
                    }
                }
                if !neural.is_empty() {
                    let _ = app.kiwix_mobile().call("speechEngineStop", json!({})).await;
                    let _ = app.kiwix_mobile().call("speechEngineNowPlaying", json!({"active": false})).await;
                }
            });
        }
        /// Mirror the Rust Kokoro transport into the iOS Now Playing card.
        /// Android already has a MediaSession owned by SpeechPlayback.
        async fn engine_now_playing(&self, active: bool, speaking: bool, title: Option<&str>, subtitle: Option<&str>) {
            #[cfg(target_os = "ios")]
            {
                let _ = self.mobile_call("speechEngineNowPlaying", json!({
                    "active": active, "speaking": speaking,
                    "title": title, "subtitle": subtitle,
                })).await;
            }
            #[cfg(not(target_os = "ios"))]
            { let _ = (active, speaking, title, subtitle); }
        }

        /// Mirror the Kokoro player onto the read-aloud Live Activity. This is
        /// housekeeping like Now Playing: a refused or disabled system surface
        /// must never become a reader error.
        fn read_aloud_activity(&self, payload: Value) {
            #[cfg(target_os = "ios")]
            let payload = if self.store.private_mode().unwrap_or(true) {
                json!({"state": "clear"})
            } else {
                payload
            };
            #[cfg(target_os = "ios")]
            {
                let app = match self.speech_app() {
                    Ok(app) => app,
                    Err(error) => { eprintln!("Kiwix read-aloud activity: {error}"); return; }
                };
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = app.kiwix_mobile().call("readAloudActivity", payload).await {
                        eprintln!("Kiwix read-aloud activity: {error}");
                    }
                });
            }
            #[cfg(not(target_os = "ios"))]
            { let _ = payload; }
        }
        pub(in crate::service) fn mobile_speech_block(&self, scope: Scope) -> Retirement {
            let (guard, retired) = self.speech.block(scope);
            self.speech_cleanup(retired);
            guard
        }
        #[cfg(target_os = "ios")]
        pub(crate) fn mobile_speech_inactive(&self) {
            // Lifecycle flags are already inactive, so dropping this temporary
            // block cannot admit a new source while native cleanup is pending.
            let _retired = self.mobile_speech_block(Scope::All);
        }
        /// Called synchronously before operation waits on transfer/privacy gates.
        pub(in crate::service) fn mobile_speech_lifecycle(&self, command: &str, args: &Value, label: &str) -> Result<Option<Retirement>, String> {
            let scope = match command {
                "sensor_activate" | "close_window" => Some(Scope::Window(label.into())),
                "sensor_deactivate" => Some(Scope::Session(label.into(), text(args, "sessionId")?.into())),
                "remove_book" | "relink_book" | "migrate_library" | "move_archives" | "recover_migrations" => Some(Scope::All),
                "mobile_open_external_archive" if args.get("replaceId").is_some_and(|value| !value.is_null()) => Some(Scope::All),
                "save_settings" => args.get("settings").filter(|settings| {
                    settings.get("privateMode").is_some() || settings.get("readOnly").is_some()
                }).map(|_| Scope::All),
                _ => None,
            };
            Ok(scope.map(|scope| self.mobile_speech_block(scope)))
        }
        fn speech_validate(&self, owner: &Owner) -> Result<(), String> {
            self.ensure_running()?;
            if owner.window != "main" { return Err("Native speech requires the trusted root window".into()); }
            self.ensure_window_active(&owner.window)?;
            if !self.mobile_active() { return Err("SPEECH_FOREGROUND_REQUIRED".into()); }
            if self.launch_generation() != owner.generation { return Err("Browsing session expired after a privacy transition".into()); }
            let book = self.store.book(&owner.archive)?.filter(|book| book.available && book.installation_id == owner.installation)
                .ok_or("SPEECH_ARCHIVE_INSTALLATION_RETIRED")?;
            if !self.sensor_manager(&owner.window).owns_session(&owner.session, &owner.archive, &book.installation_id, &owner.origin) {
                return Err("SPEECH_READER_SESSION_RETIRED".into());
            }
            Ok(())
        }
        /// Identity-only check usable from the synchronous native event path.
        fn speech_validate_retained_identity(&self, owner: &Owner) -> Result<(), String> {
            self.ensure_running()?;
            if owner.window != "main" { return Err("Native speech requires the trusted root window".into()); }
            if self.launch_generation() != owner.generation { return Err("Browsing session expired after a privacy transition".into()); }
            self.store.book(&owner.archive)?.filter(|book| book.available && book.installation_id == owner.installation)
                .ok_or("SPEECH_ARCHIVE_INSTALLATION_RETIRED")?;
            Ok(())
        }
        /// Identity-only check for a transport action delivered while the
        /// Activity is backgrounded. The reader document is retained by the
        /// media exemption, so no foreground/window/sensor admission applies.
        async fn speech_validate_retained(&self, owner: &Owner) -> Result<(), String> {
            self.speech_validate_retained_identity(owner)?;
            let book = self.store.book(&owner.archive)?.filter(|book| book.available && book.installation_id == owner.installation)
                .ok_or("SPEECH_ARCHIVE_INSTALLATION_RETIRED")?;
            let readers = self.readers.lock().await;
            let reader = readers.get(&format!("main:{}", owner.archive))
                .filter(|reader| reader.origin == owner.origin && reader.installation_id == book.installation_id)
                .ok_or("SPEECH_READER_LISTENER_RETIRED")?;
            let _ = reader;
            Ok(())
        }
        /// A started session still owns its reader document across Android
        /// suspension. Paused sessions count: they may resume from transport.
        #[cfg(any(target_os = "ios", target_os = "android"))]
        pub(crate) fn mobile_speech_retained(&self) -> bool {
            self.speech.lock().registrations.values().any(|entry| {
                matches!(entry.phase, Phase::Starting | Phase::Started) && entry.token.is_some()
            })
        }
        fn speech_active_capability(&self) -> Option<(String, String, Owner)> {
            let state = self.speech.lock();
            state.registrations.iter()
                .filter(|(_, entry)| matches!(entry.phase, Phase::Starting | Phase::Started)
                    && entry.token.is_some() && entry.owner.window == "main")
                .max_by_key(|(_, entry)| entry.ordinal)
                .map(|(native_id, entry)| (native_id.clone(), entry.token.clone().unwrap(), entry.owner.clone()))
        }
        async fn speech_restore_reader_owner(&self, owner: &Owner, foreground_transition: Option<u64>) -> Result<Option<Value>, String> {
            self.speech_validate_retained_identity(owner)?;
            self.ensure_window_active(&owner.window)?;
            let settings = self.store.settings()?;
            if settings.private_mode || settings.read_only { return Err("SPEECH_SOURCE_RETIRING".into()); }
            let readers = self.readers.lock().await;
            readers.get(&format!("{}:{}", owner.window, owner.archive))
                .filter(|reader| reader.origin == owner.origin && reader.installation_id == owner.installation)
                .ok_or("SPEECH_READER_LISTENER_RETIRED")?;
            self.speech_validate_retained_identity(owner)?;
            let state = self.speech.lock();
            // Event validation locks speech before lifecycle. Preserve that
            // order and publish foreground in the same critical section as
            // sensor restoration, never exposing active + missing owner.
            let mut native = self.mobile.state.lock().map_err(|_| "Mobile lifecycle unavailable")?;
            if let Some(generation) = foreground_transition {
                if native.generation != generation || !native.desired_active { return Ok(None); }
            } else if !native.active || !native.desired_active || native.inactive {
                return Err("SPEECH_FOREGROUND_REQUIRED".into());
            }
            let sensors = self.sensor_manager(&owner.window);
            if foreground_transition.is_some() {
                state.restore_before_foreground(owner, &sensors, || {
                    native.active = true;
                    native.background = None;
                })?;
            } else { state.restore_reader(owner, &sensors)?; }
            Ok(Some(self.sensor_status(&sensors, &owner.session)))
        }
        /// Caller holds native lifecycle serialization. Restore before active
        /// publication so a native TTS event cannot observe an ownership gap.
        #[cfg(target_os = "android")]
        pub(in crate::service) async fn mobile_restore_retained_reader(&self, generation: u64) -> Result<(), String> {
            let sensors = self.sensor_manager("main");
            let Some(retained) = sensors.retained_session() else { return Ok(()); };
            if retained.generation != self.launch_generation() {
                sensors.deactivate_session(&retained.session_id);
                return Ok(());
            }
            let owner = Owner {
                window: "main".into(), session: retained.session_id, archive: retained.archive_id,
                installation: retained.installation_id, origin: retained.origin, generation: retained.generation,
            };
            let result = self.speech_restore_reader_owner(&owner, Some(generation)).await;
            if result.is_err() {
                sensors.deactivate_session(&owner.session);
                let _retired = self.mobile_speech_block(Scope::Session(owner.window.clone(), owner.session.clone()));
            }
            result.map(|_| ())
        }
        async fn speech_source(&self, args: &Value, label: &str) -> Result<Owner, String> {
            let readers = self.readers.lock().await;
            let archive = text(args, "archiveId")?;
            let origin = text(args, "origin")?;
            let reader = readers.get(&format!("{label}:{archive}")).filter(|reader| reader.origin == origin)
                .ok_or("SPEECH_READER_LISTENER_RETIRED")?;
            let owner = Owner {
                window: label.into(), session: text(args, "sessionId")?.into(), archive: archive.into(),
                installation: reader.installation_id.clone(), origin: origin.into(), generation: self.ensure_generation(args)?,
            };
            // Source mutations take the synchronous speech block before awaiting
            // readers, so this snapshot cannot admit a retiring source.
            let state = self.speech.lock();
            if state.blocked(&owner) { return Err("SPEECH_SOURCE_RETIRING".into()); }
            self.speech_validate(&owner)?;
            Ok(owner)
        }
        pub(in crate::service) async fn mobile_speech_command(self: Arc<Self>, command: &str, args: Value, label: &str) -> Result<Value, String> {
            let attempted = if command == "mobile_speech_start" {
                let state = self.speech.lock();
                args.get("id").and_then(Value::as_str).zip(args.get("token").and_then(Value::as_str))
                    .and_then(|(id, token)| state.owned(label, id, token).map(|native_id| Capability { native_id, token: token.into() }))
            } else { None };
            let result = self.clone().speech_execute(command, args, label).await;
            if result.is_err() {
                if let Some(capability) = attempted {
                    let retire = {
                        let mut state = self.speech.lock();
                        state.registrations.get_mut(&capability.native_id).is_some_and(|entry| {
                            if entry.phase == Phase::Retiring { return false; }
                            entry.phase = Phase::Retiring;
                            true
                        })
                    };
                    if retire { self.speech_cleanup(vec![capability]); }
                }
            }
            result
        }
        async fn speech_execute(self: Arc<Self>, command: &str, args: Value, label: &str) -> Result<Value, String> {
            if label != "main" { return Err("Native speech requires the trusted root window".into()); }
            if matches!(command, "mobile_speech_stop" | "mobile_speech_retire") {
                // Ownership only: no active-window/mobile/privacy/session guard.
                let id = text(&args, "id")?;
                let token = text(&args, "token")?;
                let capability = {
                    let mut state = self.speech.lock();
                    state.owned(label, id, token).map(|native_id| {
                        let kokoro = state.registrations.get(&native_id).is_some_and(|entry| entry.kokoro);
                        state.registrations.get_mut(&native_id).unwrap().phase = Phase::Retiring;
                        (Capability { native_id, token: token.into() }, kokoro)
                    })
                };
                let Some((capability, kokoro)) = capability else {
                    return Ok(if command == "mobile_speech_stop" {
                        json!({"id":id,"speaking":false,"offset":0,"stopped":false})
                    } else { json!({"id":id,"retired":false}) });
                };
                if kokoro {
                    self.kokoro.stop(&capability.native_id, &capability.token);
                    let _ = self.mobile_call("speechEngineStop", json!({})).await;
                    self.engine_now_playing(false, false, None, None).await;
                    self.read_aloud_activity(json!({"state": "terminal", "sessionId": capability.native_id}));
                }
                let result = cleanup(&self.speech_app()?, &self.speech, &capability).await?;
                let service = self.clone();
                tauri::async_runtime::spawn(async move { service.mobile_speech_released().await; });
                return Ok(if command == "mobile_speech_stop" {
                    json!({"id":id,"speaking":false,"offset":result.get("offset").and_then(Value::as_u64).unwrap_or(0),"stopped":result.get("stopped").and_then(Value::as_bool).unwrap_or(false)})
                } else { json!({"id":id,"retired":true}) });
            }
            if matches!(command, "mobile_speech_pause" | "mobile_speech_resume" | "mobile_speech_previous" |
                "mobile_speech_next" | "mobile_speech_rewind" | "mobile_speech_forward" | "mobile_speech_seek") {
                let id = text(&args, "id")?;
                let token = text(&args, "token")?;
                let owned = {
                    let state = self.speech.lock();
                    state.owned(label, id, token).and_then(|native_id| {
                        state.registrations.get(&native_id)
                            .map(|entry| (native_id, entry.owner.clone(), entry.text_length, entry.kokoro))
                    })
                };
                let Some((native_id, _owner, text_length, kokoro)) = owned else { return Err("SPEECH_SESSION_RETIRED".into()); };
                // Transport commands are authorized by the unguessable session
                // token and the active engine identity. Re-validating the reader
                // source here made pause fail when the frame/sensor state was in
                // flux even though the audio thread was still owned and playing.
                if kokoro {
                    let position = if command == "mobile_speech_seek" {
                        Some(args.get("offset").and_then(Value::as_u64).filter(|offset| *offset <= text_length)
                            .ok_or("SPEECH_INVALID_REQUEST")? as usize)
                    } else { None };
                    let seconds = if matches!(command, "mobile_speech_rewind" | "mobile_speech_forward") {
                        Some(args.get("seconds").and_then(Value::as_f64)
                            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 300.0)
                            .ok_or("SPEECH_INVALID_REQUEST")?)
                    } else { None };
                    let transport = match command {
                        "mobile_speech_pause" => crate::service::kokoro::Command::Pause,
                        "mobile_speech_resume" => crate::service::kokoro::Command::Resume,
                        "mobile_speech_previous" => crate::service::kokoro::Command::Previous,
                        "mobile_speech_next" => crate::service::kokoro::Command::Next,
                        "mobile_speech_rewind" => crate::service::kokoro::Command::Rewind(seconds.unwrap_or(15.0)),
                        "mobile_speech_forward" => crate::service::kokoro::Command::Forward(seconds.unwrap_or(15.0)),
                        _ => crate::service::kokoro::Command::Seek(position.unwrap_or(0)),
                    };
                    if !self.kokoro.command(&native_id, token, transport) {
                        return Err("SPEECH_SESSION_RETIRED".into());
                    }
                    let paused = command == "mobile_speech_pause";
                    if matches!(command, "mobile_speech_pause" | "mobile_speech_resume") {
                        let _ = self.mobile_call("speechEngineUpdate", json!({
                            "speaking": !paused, "paused": paused, "canPrevious": true,
                        })).await;
                        self.engine_now_playing(true, !paused, None, None).await;
                        let locale = crate::localization::Localizer::for_app(&self.store.settings()?.locale);
                        let status_label = if paused {
                            locale.text("Paused", &[])
                        } else {
                            locale.text("Reading aloud", &[])
                        };
                        self.read_aloud_activity(json!({
                            "state": "active", "sessionId": native_id,
                            "status": if paused { "paused" } else { "reading" },
                            "statusLabel": status_label,
                            "offset": self.kokoro.active_offset(&native_id, token).unwrap_or(0),
                            "total": text_length,
                        }));
                    }
                    let offset = self.kokoro.active_offset(&native_id, token).unwrap_or(0);
                    return Ok(json!({"id":id,"paused":command == "mobile_speech_pause","offset":offset}));
                }
                let native: &'static str = match command {
                    "mobile_speech_pause" => "speechPause",
                    "mobile_speech_resume" => "speechResume",
                    "mobile_speech_previous" => "speechPrevious",
                    "mobile_speech_next" => "speechNext",
                    "mobile_speech_rewind" => "speechRewind",
                    "mobile_speech_forward" => "speechForward",
                    _ => "speechSeek",
                };
                let mut payload = json!({"id":native_id,"token":token});
                if command == "mobile_speech_seek" {
                    let offset = args.get("offset").and_then(Value::as_u64).filter(|offset| *offset <= text_length)
                        .ok_or("SPEECH_INVALID_REQUEST")?;
                    payload["offset"] = json!(offset);
                } else if matches!(command, "mobile_speech_rewind" | "mobile_speech_forward") {
                    let seconds = args.get("seconds").and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 300.0)
                        .ok_or("SPEECH_INVALID_REQUEST")?;
                    payload["seconds"] = json!(seconds);
                }
                return self.mobile_call(native, payload).await;
            }
            self.ensure_window_active(label)?;
            self.ensure_generation(&args)?;
            if !self.mobile_active() { return Err("SPEECH_FOREGROUND_REQUIRED".into()); }
            if command == "mobile_speech_restore_reader" {
                // Service dispatch routes this exact-owner command before the
                // ordinary sensor_activate window-wide speech retirement.
                // There is no native Start/Resume/Stop/Retire on restoration.
                let _lifecycle = self.mobile.serial.lock().await;
                if !self.mobile_active() { return Err("SPEECH_FOREGROUND_REQUIRED".into()); }
                let owner = Owner {
                    window: label.into(), session: text(&args, "sessionId")?.into(),
                    archive: text(&args, "archiveId")?.into(), installation: text(&args, "installationId")?.into(),
                    origin: text(&args, "origin")?.into(), generation: self.ensure_generation(&args)?,
                };
                return self.speech_restore_reader_owner(&owner, None).await?
                    .ok_or_else(|| "SPEECH_FOREGROUND_REQUIRED".into());
            }
            if command == "mobile_speech_voices" {
                if !self.speech.lock().blocks.is_empty() { return Err("SPEECH_SOURCE_RETIRING".into()); }
                let mut result = self.mobile_call("speechVoices", json!({})).await?;
                self.ensure_generation(&args)?;
                self.ensure_window_active(label)?;
                if !self.mobile_active() { return Err("SPEECH_FOREGROUND_REQUIRED".into()); }
                if !self.store.private_mode()? && !self.store.read_only()? {
                    self.kokoro.ensure_performance_probe();
                }
                if self.kokoro.usable() {
                    if let Some(voices) = result.get_mut("voices").and_then(Value::as_array_mut) {
                        voices.extend(crate::service::kokoro::voice_entries());
                    }
                }
                return Ok(result);
            }
            if !matches!(command, "mobile_speech_prepare" | "mobile_speech_start") { return Err("Unknown native speech command".into()); }
            let id = text(&args, "id")?.to_string();
            if uuid::Uuid::parse_str(&id).is_err() { return Err("SPEECH_INVALID_REQUEST".into()); }
            let owner = self.speech_source(&args, label).await?;
            if command == "mobile_speech_prepare" {
                // Native engines assign issuance ordinals on receipt. Serialize
                // reserve through Prepare acknowledgement so those ordinals have
                // exactly Rust's order, even across concurrent root requests.
                // Start/Stop/Retire and lifecycle retirement never take this gate.
                let _preparation = self.speech_prepare.lock().await;
                let native_id = {
                    let mut state = self.speech.lock();
                    self.speech_validate(&owner)?;
                    state.reserve(&id, owner.clone())?
                };
                let response = self.mobile_call("speechPrepare", json!({"id":native_id})).await;
                let response = match response {
                    Ok(response) => response,
                    Err(error) => { self.speech.lock().registrations.remove(&native_id); return Err(error); }
                };
                let token = response.get("token").and_then(Value::as_str).filter(|token| uuid::Uuid::parse_str(token).is_ok())
                    .ok_or("Native speech preparation returned no capability")?.to_string();
                let capability = Capability { native_id: native_id.clone(), token: token.clone() };
                let admission = {
                    let mut state = self.speech.lock();
                    let valid = self.speech_validate(&owner).and_then(|_| {
                        if state.blocked(&owner) { Err("SPEECH_SOURCE_RETIRING".into()) } else { Ok(()) }
                    });
                    let entry = state.registrations.get_mut(&native_id).ok_or("SPEECH_SESSION_RETIRED")?;
                    entry.token = Some(token.clone());
                    if entry.phase == Phase::Preparing && valid.is_ok() {
                        entry.phase = Phase::Prepared;
                        Ok(())
                    } else {
                        entry.phase = Phase::Retiring;
                        Err(valid.err().unwrap_or_else(|| "SPEECH_SESSION_RETIRED".into()))
                    }
                };
                if let Err(error) = admission {
                    self.speech_cleanup(vec![capability]);
                    return Err(error);
                }
                return Ok(json!({"id":id,"token":token}));
            }
            let token = text(&args, "token")?.to_string();
            let length = text(&args, "text")?.encode_utf16().count() as u64;
            if length > 1_000_000 { return Err("SPEECH_INVALID_REQUEST".into()); }
            // Resolve ownership read-only first so a retired session keeps its
            // error precedence, then build the complete fallible native payload
            // before the registration becomes Starting: a missing field or
            // unreadable settings must not retire unrelated sessions or leave a
            // token-bearing registration waiting for Start.
            let native_id = {
                let state = self.speech.lock();
                self.speech_validate(&owner)?;
                state.owned(label, &id, &token).ok_or("SPEECH_SESSION_RETIRED")?
            };
            // Move the bounded text into the native payload; no second text copy.
            let Value::Object(mut fields) = args else { return Err("Invalid speech arguments".into()); };
            let mut payload = serde_json::Map::new();
            for key in ["text", "lang", "voice", "rate"] {
                payload.insert(key.into(), fields.remove(key).ok_or_else(|| format!("Missing speech argument: {key}"))?);
            }
            if let Some(title) = fields.remove("title") {
                payload.insert("title".into(), title);
            }
            payload.insert("id".into(), json!(native_id));
            payload.insert("token".into(), json!(token));
            let locale = crate::localization::Localizer::for_app(&self.store.settings()?.locale);
            payload.insert("labels".into(), json!({
                "title": locale.text("Reading aloud", &[]),
                "pause": locale.text("Pause", &[]),
                "paused": locale.text("Paused", &[]),
                "resume": locale.text("Resume", &[]),
                "stop": locale.text("Stop", &[]),
                "previous": locale.text("Previous paragraph", &[]),
                "rewind": locale.text("Rewind 15 seconds", &[]),
                "forward": locale.text("Forward 15 seconds", &[]),
            }));
            let capability = Capability { native_id: native_id.clone(), token };
            let retired = {
                let mut state = self.speech.lock();
                self.speech_validate(&owner)?;
                // Ownership cannot change for this id+token; re-read it so a
                // retirement between the two critical sections is still caught.
                if state.owned(label, &id, &capability.token).as_deref() != Some(native_id.as_str()) {
                    return Err("SPEECH_SESSION_RETIRED".into());
                }
                match state.begin_start(&capability.native_id, &owner, length) {
                    Ok(retired) => retired,
                    Err(error) => {
                        state.registrations.get_mut(&capability.native_id).unwrap().phase = Phase::Retiring;
                        drop(state);
                        self.speech_cleanup(vec![capability]);
                        return Err(error);
                    }
                }
            };
            self.speech_cleanup(retired);
            let neural = payload.get("voice").and_then(Value::as_str)
                .is_some_and(|voice| voice.starts_with("kokoro:"));
            if neural {
                let language = payload.get("lang").and_then(Value::as_str).unwrap_or_default();
                if !language.to_ascii_lowercase().starts_with("en") {
                    if let Some(entry) = self.speech.lock().registrations.get_mut(&capability.native_id) {
                        entry.phase = Phase::Retiring;
                    }
                    self.speech_cleanup(vec![capability.clone()]);
                    return Err("Natural voices are only available for English; choose an installed system voice".into());
                }
            }
            let response = if neural {
                let voice = payload.get("voice").and_then(Value::as_str).unwrap_or_default().to_string();
                let rate = payload.get("rate").and_then(Value::as_f64).unwrap_or(1.0);
                let text = payload.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
                let title = payload.get("title").and_then(Value::as_str).unwrap_or_default()
                    .chars().take(160).collect::<String>();
                let labels = payload.get("labels").cloned().unwrap_or(Value::Null);
                let reading_label = labels.get("title").and_then(Value::as_str)
                    .unwrap_or("Reading aloud").to_string();
                let paused_label = labels.get("paused").and_then(Value::as_str)
                    .unwrap_or("Paused").to_string();
                let now_playing_title = labels.get("title").and_then(Value::as_str).map(str::to_owned);
                let now_playing_voice = voice.clone();
                let activity_title = if title.is_empty() { reading_label.clone() } else { title };
                let activity_voice = voice.clone();
                let audio = self.mobile_call("speechEngineStart", json!({"labels": labels})).await;
                if let Err(error) = audio {
                    self.kokoro.stop(&capability.native_id, &capability.token);
                    let _ = self.mobile_call("speechEngineStop", json!({})).await;
                    Err(error)
                } else {
                let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
                let result = self.kokoro.start(crate::service::kokoro::StartRequest {
                    id: capability.native_id.clone(), token: capability.token.clone(), text, voice, rate,
                    title: activity_title.clone(), reading_label: reading_label.clone(),
                    paused_label: paused_label.clone(),
                }, events);
                if result.is_ok() {
                    // Request while the app is still foreground; ActivityKit
                    // refuses a new activity from a background process.
                    self.read_aloud_activity(json!({
                        "state": "active", "sessionId": capability.native_id,
                        "title": activity_title, "voice": activity_voice,
                        "status": "reading", "statusLabel": reading_label,
                        "offset": 0, "total": length,
                    }));
                    self.engine_now_playing(true, true, now_playing_title.as_deref(), Some(&now_playing_voice)).await;
                    let service = self.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut receiver = receiver;
                        while let Some(event) = receiver.recv().await {
                            service.mobile_speech_event(event);
                        }
                    });
                }
                if result.is_err() { let _ = self.mobile_call("speechEngineStop", json!({})).await; }
                result.map(|_| json!({"id":native_id,"accepted":true}))
                }
            } else {
                self.mobile_call("speechStart", Value::Object(payload)).await
            };
            let admission = {
                let mut state = self.speech.lock();
                let valid = self.speech_validate(&owner);
                if response.is_ok() && valid.is_ok() && state.finish_start(&capability.native_id) {
                    if let Some(entry) = state.registrations.get_mut(&capability.native_id) { entry.kokoro = neural; }
                    Ok(())
                } else {
                    if let Some(entry) = state.registrations.get_mut(&capability.native_id) { entry.phase = Phase::Retiring; }
                    Err(response.err().or_else(|| valid.err()).unwrap_or_else(|| "SPEECH_SESSION_RETIRED".into()))
                }
            };
            if let Err(error) = admission {
                if neural {
                    self.read_aloud_activity(json!({
                        "state": "terminal", "sessionId": capability.native_id,
                    }));
                    self.kokoro.stop(&capability.native_id, &capability.token);
                }
                self.speech_cleanup(vec![capability]);
                return Err(error);
            }
            Ok(json!({"id":id,"accepted":true}))
        }
        /// Retire the reader-visible session before its model files disappear.
        pub(crate) fn mobile_speech_model_deleted(self: &Arc<Self>) {
            if let Some((native_id, _token)) = self.kokoro.active_identity() {
                self.read_aloud_activity(json!({"state": "terminal", "sessionId": native_id}));
                self.mobile_speech_event(json!({
                    "id": native_id, "speaking": false, "paused": false, "offset": 0,
                }));
            }
        }

        pub(crate) fn mobile_speech_event(self: &Arc<Self>, payload: Value) {
            let Some(native_id) = payload.get("id").and_then(Value::as_str) else { return; };
            let Some(speaking) = payload.get("speaking").and_then(Value::as_bool) else { return; };
            let Some(offset) = payload.get("offset").and_then(Value::as_u64) else { return; };
            let paused = !speaking && payload.get("paused").and_then(Value::as_bool).unwrap_or(false);
            let mut cleanup = None;
            let mut released = false;
            {
                let mut state = self.speech.lock();
                let Some(entry) = state.registrations.get_mut(native_id) else { return; };
                if !matches!(entry.phase, Phase::Starting | Phase::Started) { return; }
                let valid = offset <= entry.text_length && if self.mobile_active() {
                    self.speech_validate(&entry.owner).is_ok()
                } else {
                    self.speech_validate_retained_identity(&entry.owner).is_ok()
                };
                if entry.retire_for_event(valid, speaking, paused) {
                    cleanup = entry.token.as_ref().map(|token| Capability { native_id: native_id.into(), token: token.clone() });
                    released = true;
                }
                if valid {
                    let mut forwarded = json!({"id":entry.id,"speaking":speaking,"offset":offset,"paused":paused});
                    if let Some(error) = payload.get("error").and_then(Value::as_str) { forwarded["error"] = json!(error); }
                    // Keep ownership locked through forwarding. A simultaneous
                    // teardown cannot retire then receive an old speaking=true.
                    mobile::emit(self, "mobile-speech-state", forwarded);
                    let status = if paused { "paused" } else { "reading" };
                    let state = if !speaking && !paused { "terminal" } else { "active" };
                    self.read_aloud_activity(json!({
                        "state": state, "sessionId": native_id,
                        "title": payload.get("title"), "voice": payload.get("voice"),
                        "status": status, "statusLabel": payload.get("statusLabel"),
                        "offset": offset, "total": payload.get("total"),
                    }));
                }
            }
            if let Some(capability) = cleanup { self.speech_cleanup(vec![capability]); }
            if released {
                let service = self.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = service.mobile_call("speechEngineStop", json!({})).await;
                    service.engine_now_playing(false, false, None, None).await;
                });
            }
            if released {
                let service = self.clone();
                tauri::async_runtime::spawn(async move { service.mobile_speech_released().await; });
            }
        }

        /// Notification and lock-screen transport re-enters here so every
        /// action crosses the same ownership and native command path as the UI.
        pub(crate) fn mobile_speech_action(self: &Arc<Self>, payload: Value) {
            let Some(action) = payload.get("action").and_then(Value::as_str)
                .filter(|action| matches!(*action, "play" | "pause" | "stop" | "previous" | "next" | "rewind" | "forward")) else { return; };
            let service = self.clone();
            let action = action.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = service.clone().speech_notification_action(&action).await {
                    mobile::error(&service, "speech-action", error);
                }
            });
        }
        async fn speech_notification_action(self: Arc<Self>, action: &str) -> Result<(), String> {
            let Some((native_id, token, owner)) = self.speech_active_capability() else {
                self.read_aloud_activity(json!({"state": "clear"}));
                return Ok(());
            };
            let capability = Capability { native_id: native_id.clone(), token: token.clone() };
            let kokoro = self.speech.lock().registrations.get(&native_id).is_some_and(|entry| entry.kokoro);
            if kokoro {
                if action == "stop" {
                    // The Rust worker exits silently on Command::Stop, unlike
                    // NativeSpeech. Retire first, then publish the terminal
                    // state so a concurrent frontend retire cannot race the
                    // native cleanup. The header, highlight and media
                    // exemption must not remain stuck.
                    let request_id = self.speech.lock().registrations
                        .get(&native_id).map(|entry| entry.id.clone());
                    self.kokoro.stop(&native_id, &token);
                    let _ = self.mobile_call("speechEngineStop", json!({})).await;
                    self.engine_now_playing(false, false, None, None).await;
                    self.read_aloud_activity(json!({"state": "terminal", "sessionId": native_id}));
                    let result = match self.speech_app() {
                        Ok(app) => cleanup(&app, &self.speech, &capability).await,
                        Err(error) => Err(error),
                    };
                    if let Some(request_id) = request_id {
                        mobile::emit(&self, "mobile-speech-state", json!({
                            "id": request_id, "speaking": false, "paused": false, "offset": 0,
                        }));
                    }
                    self.clone().mobile_speech_released().await;
                    if let Err(error) = result { eprintln!("Kiwix speech-action stop cleanup: {error}"); }
                    return Ok(());
                } else {
                    self.speech_validate_retained(&owner).await?;
                    let command = match action {
                        "play" => crate::service::kokoro::Command::Resume,
                        "pause" => crate::service::kokoro::Command::Pause,
                        "previous" => crate::service::kokoro::Command::Previous,
                        "next" => crate::service::kokoro::Command::Next,
                        "rewind" => crate::service::kokoro::Command::Rewind(15.0),
                        "forward" => crate::service::kokoro::Command::Forward(15.0),
                        _ => return Ok(()),
                    };
                    if !self.kokoro.command(&native_id, &token, command) {
                        return Err("SPEECH_SESSION_RETIRED".into());
                    }
                    let paused = action == "pause";
                    if action == "pause" || action == "play" {
                        let _ = self.mobile_call("speechEngineUpdate", json!({
                            "speaking": !paused, "paused": paused, "canPrevious": true,
                        })).await;
                        self.engine_now_playing(true, !paused, None, None).await;
                        let text_length = self.speech.lock().registrations
                            .get(&native_id).map(|entry| entry.text_length).unwrap_or(0);
                        let locale = crate::localization::Localizer::for_app(&self.store.settings()?.locale);
                        self.read_aloud_activity(json!({
                            "state": "active", "sessionId": native_id,
                            "status": if paused { "paused" } else { "reading" },
                            "statusLabel": if paused { locale.text("Paused", &[]) } else { locale.text("Reading aloud", &[]) },
                            "offset": self.kokoro.active_offset(&native_id, &token).unwrap_or(0),
                            "total": text_length,
                        }));
                    }
                    return Ok(());
                }
            }
            if action == "stop" {
                let app = self.speech_app()?;
                let result = cleanup(&app, &self.speech, &capability).await;
                self.clone().mobile_speech_released().await;
                return result.map(|_| ());
            }
            self.speech_validate_retained(&owner).await?;
            let command: &'static str = match action {
                "play" => "speechResume",
                "pause" => "speechPause",
                "previous" => "speechPrevious",
                "next" => "speechNext",
                "rewind" => "speechRewind",
                "forward" => "speechForward",
                _ => return Ok(()),
            };
            if let Err(error) = self.mobile_call(command, json!({"id":native_id,"token":token})).await {
                let app = self.speech_app()?;
                let _ = cleanup(&app, &self.speech, &capability).await;
                self.clone().mobile_speech_released().await;
                return Err(error);
            }
            Ok(())
        }

        /// iOS and Android retain reader origins while read-aloud holds the
        /// document. When the last started session ends in the background,
        /// retire them exactly once unless archive media/PiP still retains it.
        pub(in crate::service) async fn mobile_speech_released(self: Arc<Self>) {
            #[cfg(any(target_os = "ios", target_os = "android"))]
            {
                if self.mobile_active() || self.mobile_speech_retained() { return; }
                #[cfg(target_os = "ios")]
                if self.mobile_reader_media_retains() { return; }
                let _serial = self.mobile.serial.lock().await;
                if self.mobile_active() || self.mobile_speech_retained() { return; }
                #[cfg(target_os = "ios")]
                if self.mobile_reader_media_retains() { return; }
                if !self.mobile.media_retained.swap(false, std::sync::atomic::Ordering::SeqCst) { return; }
                let _mode = self.privacy_gate.write().await;
                self.privacy_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Err(error) = crate::service::retire_readers(&mut *self.readers.lock().await, None).await {
                    mobile::error(&self, "speech-reader-retirement", error);
                }
                if let Err(error) = self.mobile_note_retirement() {
                    mobile::error(&self, "speech-retirement", error);
                }
                if let Ok(mut archives) = self.archives.lock() { archives.clear(); }
                let spelling = self.spelling.clone();
                if let Err(error) = crate::service::blocking(move || spelling.set_writable(false)).await {
                    mobile::error(&self, "speech-spelling-retirement", error);
                }
            }
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            let _ = self;
        }
    }
}

/// Desktop read-aloud: system voices stay in the browser's speechSynthesis;
/// only Kokoro neural voices cross into Rust. The same ownership machine and
/// reader contract (`mobile_speech_*` commands, `mobile-speech-state` events)
/// as mobile apply, without any native synthesizer or plugin call.
#[cfg(desktop)]
mod transport {
    use super::*;
    use crate::service::{text, Service};
    use serde_json::{json, Value};
    use tauri::Emitter;

    impl Service {
        fn speech_cleanup(&self, capabilities: Vec<Capability>) {
            for capability in capabilities {
                // Keyed by the exact id and token, so a stale capability is a no-op.
                self.kokoro.stop(&capability.native_id, &capability.token);
                self.speech.lock().finish_retire(&capability);
            }
        }
        /// Retirement the reader did not ask for (another window started
        /// Kokoro, a privacy change, a migration, a removed installation):
        /// stop the audio and tell each owning reader its session ended so its
        /// controls do not keep showing "Pause". Never used for a start error
        /// or an explicit stop, whose caller already reports the outcome.
        fn speech_retire_notify(&self, capabilities: Vec<Capability>) {
            for capability in capabilities {
                let notice = self.speech.lock().registrations.get(&capability.native_id)
                    .filter(|entry| entry.kokoro)
                    .map(|entry| (entry.owner.window.clone(), entry.id.clone()));
                self.speech_cleanup(vec![capability]);
                if let Some((window, id)) = notice {
                    self.speech_emit(&window, json!({"id":id,"speaking":false,"paused":false,"offset":0}));
                }
            }
        }
        fn speech_emit(&self, window: &str, payload: Value) {
            if let Ok(app) = self.app.lock() {
                if let Some(app) = app.as_ref() { let _ = app.emit_to(window, "mobile-speech-state", payload); }
            }
        }
        pub(in crate::service) fn mobile_speech_block(&self, scope: Scope) -> Retirement {
            let (guard, retired) = self.speech.block(scope);
            self.speech_retire_notify(retired);
            guard
        }
        /// Called synchronously before operation waits on transfer/privacy gates.
        /// Removing a book retires only that installation (`forget_installation`),
        /// so readers of other books in other windows keep playing.
        pub(in crate::service) fn mobile_speech_lifecycle(&self, command: &str, args: &Value, label: &str) -> Result<Option<Retirement>, String> {
            let scope = match command {
                "sensor_activate" | "close_window" => Some(Scope::Window(label.into())),
                "sensor_deactivate" => Some(Scope::Session(label.into(), text(args, "sessionId")?.into())),
                "relink_book" | "migrate_library" | "move_archives" | "recover_migrations" => Some(Scope::All),
                "save_settings" => args.get("settings").filter(|settings| {
                    settings.get("privateMode").is_some() || settings.get("readOnly").is_some()
                }).map(|_| Scope::All),
                _ => None,
            };
            Ok(scope.map(|scope| self.mobile_speech_block(scope)))
        }
        /// Identity checks that remain meaningful from the audio worker's
        /// event path: the app is running, the privacy generation is current
        /// and the archive installation still exists.
        fn speech_validate_identity(&self, owner: &Owner) -> Result<(), String> {
            self.ensure_running()?;
            if self.launch_generation() != owner.generation { return Err("Browsing session expired after a privacy transition".into()); }
            self.store.book(&owner.archive)?.filter(|book| book.available && book.installation_id == owner.installation)
                .ok_or("SPEECH_ARCHIVE_INSTALLATION_RETIRED")?;
            Ok(())
        }
        fn speech_validate(&self, owner: &Owner) -> Result<(), String> {
            self.speech_validate_identity(owner)?;
            self.ensure_window_active(&owner.window)?;
            if !self.sensor_manager(&owner.window).owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin) {
                return Err("SPEECH_READER_SESSION_RETIRED".into());
            }
            Ok(())
        }
        async fn speech_source(&self, args: &Value, label: &str) -> Result<Owner, String> {
            let readers = self.readers.lock().await;
            let archive = text(args, "archiveId")?;
            let origin = text(args, "origin")?;
            let reader = readers.get(&format!("{label}:{archive}")).filter(|reader| reader.origin == origin)
                .ok_or("SPEECH_READER_LISTENER_RETIRED")?;
            let owner = Owner {
                window: label.into(), session: text(args, "sessionId")?.into(), archive: archive.into(),
                installation: reader.installation_id.clone(), origin: origin.into(), generation: self.ensure_generation(args)?,
            };
            let state = self.speech.lock();
            if state.blocked(&owner) { return Err("SPEECH_SOURCE_RETIRING".into()); }
            self.speech_validate(&owner)?;
            Ok(owner)
        }
        pub(in crate::service) async fn mobile_speech_command(self: Arc<Self>, command: &str, args: Value, label: &str) -> Result<Value, String> {
            let attempted = if command == "mobile_speech_start" {
                let state = self.speech.lock();
                args.get("id").and_then(Value::as_str).zip(args.get("token").and_then(Value::as_str))
                    .and_then(|(id, token)| state.owned(label, id, token).map(|native_id| Capability { native_id, token: token.into() }))
            } else { None };
            let result = self.clone().speech_execute(command, args, label).await;
            if result.is_err() {
                if let Some(capability) = attempted {
                    let retire = {
                        let mut state = self.speech.lock();
                        state.registrations.get_mut(&capability.native_id).is_some_and(|entry| {
                            if entry.phase == Phase::Retiring { return false; }
                            entry.phase = Phase::Retiring;
                            true
                        })
                    };
                    if retire { self.speech_cleanup(vec![capability]); }
                }
            }
            result
        }
        async fn speech_execute(self: Arc<Self>, command: &str, args: Value, label: &str) -> Result<Value, String> {
            if matches!(command, "mobile_speech_stop" | "mobile_speech_retire") {
                // Ownership only: no active-window/privacy/session guard.
                let id = text(&args, "id")?;
                let token = text(&args, "token")?;
                let capability = {
                    let mut state = self.speech.lock();
                    state.owned(label, id, token).map(|native_id| {
                        state.registrations.get_mut(&native_id).unwrap().phase = Phase::Retiring;
                        Capability { native_id, token: token.into() }
                    })
                };
                let Some(capability) = capability else {
                    return Ok(if command == "mobile_speech_stop" {
                        json!({"id":id,"speaking":false,"offset":0,"stopped":false})
                    } else { json!({"id":id,"retired":false}) });
                };
                let offset = self.kokoro.active_offset(&capability.native_id, &capability.token);
                self.speech_cleanup(vec![capability]);
                return Ok(if command == "mobile_speech_stop" {
                    json!({"id":id,"speaking":false,"offset":offset.unwrap_or(0),"stopped":offset.is_some()})
                } else { json!({"id":id,"retired":true}) });
            }
            if matches!(command, "mobile_speech_pause" | "mobile_speech_resume" | "mobile_speech_previous" |
                "mobile_speech_next" | "mobile_speech_rewind" | "mobile_speech_forward" | "mobile_speech_seek") {
                let id = text(&args, "id")?;
                let token = text(&args, "token")?;
                let owned = {
                    let state = self.speech.lock();
                    state.owned(label, id, token).and_then(|native_id| {
                        state.registrations.get(&native_id)
                            .filter(|entry| entry.kokoro)
                            .map(|entry| (native_id, entry.text_length))
                    })
                };
                let Some((native_id, text_length)) = owned else { return Err("SPEECH_SESSION_RETIRED".into()); };
                let position = if command == "mobile_speech_seek" {
                    Some(args.get("offset").and_then(Value::as_u64).filter(|offset| *offset <= text_length)
                        .ok_or("SPEECH_INVALID_REQUEST")? as usize)
                } else { None };
                let seconds = if matches!(command, "mobile_speech_rewind" | "mobile_speech_forward") {
                    Some(args.get("seconds").and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 300.0)
                        .ok_or("SPEECH_INVALID_REQUEST")?)
                } else { None };
                let transport = match command {
                    "mobile_speech_pause" => crate::service::kokoro::Command::Pause,
                    "mobile_speech_resume" => crate::service::kokoro::Command::Resume,
                    "mobile_speech_previous" => crate::service::kokoro::Command::Previous,
                    "mobile_speech_next" => crate::service::kokoro::Command::Next,
                    "mobile_speech_rewind" => crate::service::kokoro::Command::Rewind(seconds.unwrap_or(15.0)),
                    "mobile_speech_forward" => crate::service::kokoro::Command::Forward(seconds.unwrap_or(15.0)),
                    _ => crate::service::kokoro::Command::Seek(position.unwrap_or(0)),
                };
                if !self.kokoro.command(&native_id, token, transport) {
                    return Err("SPEECH_SESSION_RETIRED".into());
                }
                let offset = self.kokoro.active_offset(&native_id, token).unwrap_or(0);
                return Ok(json!({"id":id,"paused":command == "mobile_speech_pause","offset":offset}));
            }
            self.ensure_window_active(label)?;
            self.ensure_generation(&args)?;
            if command == "mobile_speech_voices" {
                if self.speech.lock().blocks.values().any(|scope| match scope {
                    Scope::All => true,
                    Scope::Window(window) => window == label,
                    _ => false,
                }) { return Err("SPEECH_SOURCE_RETIRING".into()); }
                // Kokoro caches synthesized article audio on disk, so it is not
                // offered in private or read-only mode; system voices remain.
                if self.store.private_mode()? || self.store.read_only()? {
                    return Ok(json!({"available": true, "voices": []}));
                }
                self.kokoro.ensure_performance_probe();
                // Only neural voices: the browser lists the system voices.
                let voices = if self.kokoro.usable() { crate::service::kokoro::voice_entries() } else { Vec::new() };
                return Ok(json!({"available": true, "voices": voices}));
            }
            if !matches!(command, "mobile_speech_prepare" | "mobile_speech_start") { return Err("Unknown native speech command".into()); }
            let id = text(&args, "id")?.to_string();
            if uuid::Uuid::parse_str(&id).is_err() { return Err("SPEECH_INVALID_REQUEST".into()); }
            let owner = self.speech_source(&args, label).await?;
            if command == "mobile_speech_prepare" {
                let _preparation = self.speech_prepare.lock().await;
                let mut state = self.speech.lock();
                self.speech_validate(&owner)?;
                let native_id = state.reserve(&id, owner)?;
                // No native engine issues the capability: mint it here.
                let token = uuid::Uuid::new_v4().to_string();
                let entry = state.registrations.get_mut(&native_id).ok_or("SPEECH_SESSION_RETIRED")?;
                entry.token = Some(token.clone());
                entry.phase = Phase::Prepared;
                return Ok(json!({"id":id,"token":token}));
            }
            let token = text(&args, "token")?.to_string();
            let speech = text(&args, "text")?;
            let length = speech.encode_utf16().count() as u64;
            if length > 1_000_000 { return Err("SPEECH_INVALID_REQUEST".into()); }
            let voice = text(&args, "voice")?.to_string();
            if !voice.starts_with("kokoro:") { return Err("SPEECH_INVALID_REQUEST".into()); }
            if self.store.private_mode()? || self.store.read_only()? {
                return Err("Natural voices are off in private and read-only mode; choose a system voice".into());
            }
            if !text(&args, "lang")?.to_ascii_lowercase().starts_with("en") {
                return Err("Natural voices are only available for English; choose an installed system voice".into());
            }
            let rate = args.get("rate").and_then(Value::as_f64).filter(|rate| rate.is_finite() && *rate > 0.0)
                .ok_or("Missing speech argument: rate")?;
            let title = args.get("title").and_then(Value::as_str).unwrap_or_default().chars().take(160).collect::<String>();
            let locale = crate::localization::Localizer::for_app(&self.store.settings()?.locale);
            let reading_label = locale.text("Reading aloud", &[]);
            let paused_label = locale.text("Paused", &[]);
            // Kokoro is one engine shared by every window. Serialize admission
            // from begin_start through finish_start, so a near-simultaneous
            // start in another window is always seen (and retired) here rather
            // than silently replaced inside the engine.
            let _admission = self.speech_prepare.lock().await;
            let capability = {
                let mut state = self.speech.lock();
                self.speech_validate(&owner)?;
                let native_id = state.owned(label, &id, &token).ok_or("SPEECH_SESSION_RETIRED")?;
                let capability = Capability { native_id, token };
                match state.begin_start(&capability.native_id, &owner, length) {
                    Ok(mut retired) => {
                        // Kokoro is one engine: this start silences every other
                        // window's neural session, so retire them explicitly.
                        for (other_id, other) in state.registrations.iter_mut() {
                            if *other_id != capability.native_id && other.kokoro && other.phase != Phase::Retiring {
                                other.phase = Phase::Retiring;
                                if let Some(token) = &other.token {
                                    retired.push(Capability { native_id: other_id.clone(), token: token.clone() });
                                }
                            }
                        }
                        drop(state);
                        self.speech_retire_notify(retired);
                        capability
                    }
                    Err(error) => {
                        state.registrations.get_mut(&capability.native_id).unwrap().phase = Phase::Retiring;
                        drop(state);
                        self.speech_cleanup(vec![capability]);
                        return Err(error);
                    }
                }
            };
            let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let started = self.kokoro.start(crate::service::kokoro::StartRequest {
                id: capability.native_id.clone(), token: capability.token.clone(), text: speech.to_string(), voice, rate,
                title: if title.is_empty() { reading_label.clone() } else { title }, reading_label, paused_label,
            }, events);
            let admission = {
                let mut state = self.speech.lock();
                let valid = self.speech_validate(&owner);
                if started.is_ok() && valid.is_ok() && state.finish_start(&capability.native_id) {
                    if let Some(entry) = state.registrations.get_mut(&capability.native_id) { entry.kokoro = true; }
                    Ok(())
                } else {
                    if let Some(entry) = state.registrations.get_mut(&capability.native_id) { entry.phase = Phase::Retiring; }
                    Err(started.err().or_else(|| valid.err()).unwrap_or_else(|| "SPEECH_SESSION_RETIRED".into()))
                }
            };
            if let Err(error) = admission {
                self.speech_cleanup(vec![capability]);
                return Err(error);
            }
            let service = self.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = receiver.recv().await { service.mobile_speech_event(event); }
            });
            Ok(json!({"id":id,"accepted":true}))
        }
        /// Retire the reader-visible session before its model files disappear.
        pub(crate) fn mobile_speech_model_deleted(self: &Arc<Self>) {
            if let Some((native_id, _token)) = self.kokoro.active_identity() {
                self.mobile_speech_event(json!({"id": native_id, "speaking": false, "paused": false, "offset": 0}));
            }
        }
        pub(crate) fn mobile_speech_event(self: &Arc<Self>, payload: Value) {
            let Some(native_id) = payload.get("id").and_then(Value::as_str) else { return; };
            let Some(speaking) = payload.get("speaking").and_then(Value::as_bool) else { return; };
            let Some(offset) = payload.get("offset").and_then(Value::as_u64) else { return; };
            let paused = !speaking && payload.get("paused").and_then(Value::as_bool).unwrap_or(false);
            let mut cleanup = None;
            {
                let mut state = self.speech.lock();
                let Some(entry) = state.registrations.get_mut(native_id) else { return; };
                if !matches!(entry.phase, Phase::Starting | Phase::Started) { return; }
                let valid = offset <= entry.text_length && self.speech_validate_identity(&entry.owner).is_ok();
                if entry.retire_for_event(valid, speaking, paused) {
                    cleanup = entry.token.as_ref().map(|token| Capability { native_id: native_id.into(), token: token.clone() });
                }
                if valid {
                    let mut forwarded = json!({"id":entry.id,"speaking":speaking,"offset":offset,"paused":paused});
                    if let Some(error) = payload.get("error").and_then(Value::as_str) { forwarded["error"] = json!(error); }
                    // Keep ownership locked through forwarding so a concurrent
                    // teardown cannot retire and then deliver speaking=true.
                    self.speech_emit(&entry.owner.window, forwarded);
                }
            }
            if let Some(capability) = cleanup { self.speech_cleanup(vec![capability]); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(session: &str) -> Owner {
        Owner { window: "main".into(), session: session.into(), archive: "archive".into(), installation: "installation".into(), origin: "origin".into(), generation: 1 }
    }
    fn prepared(state: &mut State, id: &str) -> (String, Capability) {
        let native_id = state.reserve(id, owner("session")).unwrap();
        let entry = state.registrations.get_mut(&native_id).unwrap();
        entry.token = Some(uuid::Uuid::new_v4().to_string());
        entry.phase = Phase::Prepared;
        let capability = Capability { native_id: native_id.clone(), token: entry.token.clone().unwrap() };
        (native_id, capability)
    }
    #[test]
    fn terminal_before_start_ack_never_resurrects_and_old_token_cannot_cancel_reused_id() {
        let mut state = State::default();
        let (native_id, old) = prepared(&mut state, "same-root-id");
        state.begin_start(&native_id, &owner("session"), 10).unwrap();
        state.retire(&Scope::All);
        assert!(!state.finish_start(&native_id));
        state.finish_retire(&old);
        let (new_id, _) = prepared(&mut state, "same-root-id");
        assert_ne!(new_id, native_id);
        state.finish_retire(&old);
        assert!(state.registrations.contains_key(&new_id));
        assert!(state.owned("main", "same-root-id", &old.token).is_none());
        assert!(state.owned("other-window", "same-root-id", &old.token).is_none());
    }
    #[test]
    fn source_retirement_blocks_replacement_admission_and_preserves_pending_slot() {
        let manager = SpeechManager::default();
        let pending = manager.lock().reserve("pending", owner("session")).unwrap();
        let (guard, _) = manager.block(Scope::Window("main".into()));
        {
            let mut state = manager.lock();
            assert_eq!(state.registrations[&pending].phase, Phase::Retiring);
            assert!(state.reserve("new", owner("replacement")).is_err());
        }
        drop(guard);
        let mut state = manager.lock();
        assert!(state.reserve("new", owner("replacement")).is_ok());
        assert!(state.registrations.contains_key(&pending));
    }
    #[test]
    fn later_start_consumes_older_preparations_and_draining_slots_remain_bounded() {
        let mut state = State::default();
        let (older, _) = prepared(&mut state, "older");
        let (newer, _) = prepared(&mut state, "newer");
        let retired = state.begin_start(&newer, &owner("session"), 4).unwrap();
        assert_eq!(retired.len(), 1);
        assert_eq!(state.registrations[&older].phase, Phase::Retiring);
        assert!(state.begin_start(&older, &owner("session"), 4).is_err());
        for index in 0..6 { state.reserve(&format!("pending-{index}"), owner("session")).unwrap(); }
        state.retire(&Scope::All);
        assert!(state.reserve("overflow", owner("session")).is_err());
        assert_eq!(state.registrations.len(), 8);
    }
    #[test]
    fn background_preserves_only_a_started_speech_capability() {
        assert!(!background_retires_speech(true));
        assert!(background_retires_speech(false));
    }

    #[test]
    fn retained_reader_restore_preserves_started_speech_and_rejects_replacement_or_retired_owners() {
        let owner = Owner {
            window: "main".into(), session: "retained-session".into(),
            archive: "01234567-89ab-4cde-8fab-0123456789ab".into(),
            installation: "01234567-89ab-4cde-8fab-0123456789ac".into(),
            origin: "http://01234567-89ab-4cde-8fab-0123456789ab-0123456789abcdef.localhost:8123".into(),
            generation: 7,
        };
        let sensors = crate::permissions::SensorManager::default();
        sensors.activate(owner.session.clone(), owner.archive.clone(), owner.installation.clone(), owner.origin.clone()).unwrap();
        let mut state = State::default();
        let native_id = state.reserve("original-request", owner.clone()).unwrap();
        state.registrations.get_mut(&native_id).unwrap().token = Some("original-capability".into());
        state.registrations.get_mut(&native_id).unwrap().phase = Phase::Prepared;
        assert!(state.begin_start(&native_id, &owner, 100).unwrap().is_empty());
        assert!(state.finish_start(&native_id));
        sensors.suspend(owner.generation); // Production mobile background deactivation.
        assert!(!sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin));
        let mut foreground = false;
        state.restore_before_foreground(&owner, &sensors, || {
            assert!(sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin));
            foreground = true;
        }).unwrap();
        assert!(sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin));
        // Native foreground restores before publishing active. Exercise the
        // production progress-event retirement policy before any JS restore.
        let valid = foreground && sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin);
        assert!(!state.registrations.get_mut(&native_id).unwrap().retire_for_event(valid, true, false));
        assert_eq!(state.registrations[&native_id].phase, Phase::Started);
        assert_eq!(state.registrations[&native_id].token.as_deref(), Some("original-capability"));
        assert_eq!(state.owned("main", "original-request", "original-capability"), Some(native_id.clone()));
        for field in ["window", "session", "archive", "installation", "origin", "generation"] {
            let mut changed = owner.clone();
            match field {
                "window" => changed.window = "replacement".into(),
                "session" => changed.session = "replacement".into(),
                "archive" => changed.archive = "replacement".into(),
                "installation" => changed.installation = "replacement".into(),
                "origin" => changed.origin = "replacement".into(),
                _ => changed.generation += 1,
            }
            assert!(state.restore_reader(&changed, &sensors).is_err(), "{field}");
        }
        state.blocks.insert(1, Scope::All);
        assert!(state.restore_reader(&owner, &sensors).is_err(), "privacy retirement blocks the old owner");
        sensors.suspend(owner.generation);
        let mut published = false;
        assert!(state.restore_before_foreground(&owner, &sensors, || { published = true; }).is_err());
        assert!(!published, "retirement must win before foreground admission");
        assert!(!sensors.owns_session_id(&owner.session));
        state.blocks.clear();
        let retired = state.retire(&Scope::Window("main".into()));
        assert_eq!(retired.len(), 1, "ordinary window/session replacement still retires speech");
        sensors.deactivate();
        assert!(state.restore_reader(&owner, &sensors).is_err(), "stopped speech cannot be resurrected");
        assert!(!sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin));
    }
    #[test]
    fn retained_archive_media_restores_without_any_speech_registration() {
        let owner = Owner {
            window: "main".into(), session: "archive-media".into(),
            archive: "01234567-89ab-4cde-8fab-0123456789ab".into(),
            installation: "01234567-89ab-4cde-8fab-0123456789ac".into(),
            origin: "http://01234567-89ab-4cde-8fab-0123456789ab-0123456789abcdef.localhost:8123".into(),
            generation: 7,
        };
        let sensors = crate::permissions::SensorManager::default();
        sensors.activate(owner.session.clone(), owner.archive.clone(), owner.installation.clone(), owner.origin.clone()).unwrap();
        sensors.suspend(owner.generation);
        let state = State::default();
        let mut foreground = false;
        state.restore_before_foreground(&owner, &sensors, || { foreground = true; }).unwrap();
        assert!(foreground);
        assert!(state.registrations.is_empty(), "archive media must not manufacture a TTS capability");
        assert!(sensors.owns_session(&owner.session, &owner.archive, &owner.installation, &owner.origin));
        sensors.deactivate();
        let mut published = false;
        assert!(state.restore_before_foreground(&owner, &sensors, || { published = true; }).is_err());
        assert!(!published, "a listener/window teardown cannot restore dormant authority");
    }
}
