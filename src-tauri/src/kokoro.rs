// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline Kokoro read-aloud.
//!
//! The model is an explicit, user-initiated download. Until it is present the
//! existing platform voice path remains the only engine. Synthesis and playback
//! run on a dedicated thread because sherpa-onnx is CPU-bound and rodio owns a
//! platform audio stream; neither may block the Tauri command executor.
#![forbid(unsafe_code)]

#[path = "kokoro/install.rs"]
mod install;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{BufReader, Seek, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Arc, LazyLock, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;
use misaki_rs::{G2P, Language};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig};

/// Pinned model pack. The bytes feed a native ONNX parser, so the archive and
/// the extracted required files are both checked before they become visible.
const MODEL_DIR: &str = "kokoro-en-fp16-v1_0";
#[cfg(test)]
const MODEL_ARCHIVE: &str = "kokoro-en-fp16-v1_0.tar.bz2";
const MODEL_URL: &str =
    "https://github.com/jasontitus/CastCircle/releases/download/kokoro-en-fp16-v1/kokoro-en-fp16-v1_0.tar.bz2";
const MODEL_SHA256: &str = "4cafe1c49bf4b0a7f9c2fab9f2b010b05544dde2b28b66e3211e832308a1a1f9";
const MODEL_BYTES: u64 = 181_886_349;
const CACHE_BUDGET: u64 = 300 * 1024 * 1024;
const PERFORMANCE_FILE: &str = "performance-v2.json";
const PERFORMANCE_ERROR_FILE: &str = "performance-error-v2.json";
const BUNDLED_MARKER: &str = "bundled-model";
const BUNDLED_INSTALLING: &str = "bundled-model-installing";
const BUNDLED_REMOVED: &str = "bundled-model-removed";
const PROBE_TEXT: &str = "Animals are multicellular, eukaryotic organisms in the biological kingdom Animalia. Most animals consume organic material and breathe oxygen.";
const REALTIME_MARGIN: f64 = 1.10;

/// English Kokoro v1.0 speakers. The pack's first 28 voices are the US/GB
/// English set; the remaining speakers are other languages and are not
/// advertised by this English-only pack.
pub(crate) const VOICES: [(&str, i32, &str, &str); 28] = [
    ("af_alloy", 0, "Alloy", "en-US"),
    ("af_aoede", 1, "Aoede", "en-US"),
    ("af_bella", 2, "Bella", "en-US"),
    ("af_heart", 3, "Heart", "en-US"),
    ("af_jessica", 4, "Jessica", "en-US"),
    ("af_kore", 5, "Kore", "en-US"),
    ("af_nicole", 6, "Nicole", "en-US"),
    ("af_nova", 7, "Nova", "en-US"),
    ("af_river", 8, "River", "en-US"),
    ("af_sarah", 9, "Sarah", "en-US"),
    ("af_sky", 10, "Sky", "en-US"),
    ("am_adam", 11, "Adam", "en-US"),
    ("am_echo", 12, "Echo", "en-US"),
    ("am_eric", 13, "Eric", "en-US"),
    ("am_fenrir", 14, "Fenrir", "en-US"),
    ("am_liam", 15, "Liam", "en-US"),
    ("am_michael", 16, "Michael", "en-US"),
    ("am_onyx", 17, "Onyx", "en-US"),
    ("am_puck", 18, "Puck", "en-US"),
    ("am_santa", 19, "Santa", "en-US"),
    ("bf_alice", 20, "Alice", "en-GB"),
    ("bf_emma", 21, "Emma", "en-GB"),
    ("bf_isabella", 22, "Isabella", "en-GB"),
    ("bf_lily", 23, "Lily", "en-GB"),
    ("bm_daniel", 24, "Daniel", "en-GB"),
    ("bm_fable", 25, "Fable", "en-GB"),
    ("bm_george", 26, "George", "en-GB"),
    ("bm_lewis", 27, "Lewis", "en-GB"),
];

pub(crate) fn voice_uri(id: &str) -> String { format!("kokoro:{id}") }

pub(crate) fn voice_entries() -> Vec<Value> {
    VOICES.iter().map(|(id, _, name, lang)| json!({
        "name": format!("Natural · {name}"),
        "lang": lang,
        "voiceURI": voice_uri(id),
        "default": false,
        "quality": 500,
        "localService": true,
    })).collect()
}

fn voice_sid(voice: &str) -> Option<i32> {
    let id = voice.strip_prefix("kokoro:")?;
    VOICES.iter().find(|(candidate, ..)| *candidate == id).map(|(_, sid, ..)| *sid)
}

static CURRENCY_WITH_MAGNITUDE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\$\s*([0-9][0-9,]*(?:\.[0-9]+)?)\s*(thousand|million|billion|trillion|m|bn|tn)\b").expect("currency regex")
});

/// "`$500 million`" must be spoken as "five hundred million dollars", not
/// "five hundred dollars million". Move the currency after the magnitude
/// before Misaki's number expansion.
fn normalize_currency(text: &str) -> String {
    CURRENCY_WITH_MAGNITUDE.replace_all(text, |caps: &regex::Captures<'_>| {
        let suffix = caps[2].to_ascii_lowercase();
        let magnitude = match suffix.as_str() {
            "m" => "million",
            "bn" => "billion",
            "tn" => "trillion",
            other => other,
        };
        format!("{} {} dollars", &caps[1], magnitude)
    }).into_owned()
}

pub(crate) struct KokoroManager {
    root: PathBuf,
    state: Mutex<State>,
}

struct State {
    source: Option<install::SourceStamp>,
    downloaded: bool,
    download: Option<Download>,
    last_job: Option<Arc<DownloadControl>>,
    engine: Option<Arc<OfflineTts>>,
    active: Option<Active>,
    error: Option<String>,
    performance: Option<Performance>,
    probing: bool,
    probe: Option<(u64, CancellationToken, CancellationToken)>,
    performance_error: Option<String>,
    probe_generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Performance {
    ratio: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Verdict<T> {
    source: install::SourceStamp,
    value: T,
}

impl Performance {
    fn realtime(self) -> bool { realtime_for(self.ratio) }
}

fn realtime_for(ratio: f64) -> bool {
    ratio.is_finite() && ratio > 0.0 && ratio <= REALTIME_MARGIN
}

pub(crate) struct DownloadControl {
    cancel: CancellationToken,
    publishing: Mutex<bool>,
    selected: Mutex<Option<install::SourceStamp>>,
    explicit_bundled: bool,
}

impl DownloadControl {
    fn new() -> Self { Self { cancel: CancellationToken::new(), publishing: Mutex::new(false), selected: Mutex::default(), explicit_bundled: false } }
    fn stop(&self) -> bool {
        let published = self.publishing.lock().unwrap_or_else(|error| error.into_inner());
        if *published { return false; }
        self.cancel.cancel();
        true
    }
}

struct Download {
    downloaded: u64,
    total: u64,
    bundled: bool,
    cancel: Arc<DownloadControl>,
    finished: CancellationToken,
}

struct Active {
    id: String,
    token: String,
    commands: Sender<Command>,
    stopped: Arc<AtomicBool>,
    offset: Arc<std::sync::atomic::AtomicUsize>,
}

pub(crate) enum Command {
    Pause,
    Resume,
    Previous,
    Next,
    Seek(usize),
    Rewind(f64),
    Forward(f64),
    Stop,
}

#[derive(Clone)]
pub(crate) struct StartRequest {
    pub id: String,
    pub token: String,
    pub text: String,
    pub voice: String,
    pub rate: f64,
    pub title: String,
    pub reading_label: String,
    pub paused_label: String,
}

struct Chunk {
    text: String,
    start_utf16: usize,
}

impl KokoroManager {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let root = data_dir.join("models");
        let source = install::source_stamp(&root).ok();
        let downloaded = source.is_some();
        let performance = source.as_ref().and_then(|s| load_value(&root,PERFORMANCE_FILE,s));
        let performance_error = source.as_ref().and_then(|s| load_value(&root,PERFORMANCE_ERROR_FILE,s));
        Self { root, state: Mutex::new(State {
            source, downloaded, download: None, last_job: None, engine: None, active: None, error: None,
            performance, probing: false, probe: None, performance_error, probe_generation: 0,
        }) }
    }

    pub(crate) fn status(&self) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        refresh_downloaded(&self.root, &mut state);
        let download = state.download.as_ref();
        let (legacy_model,legacy_audio) = install::preserved_legacy(&self.root);
        json!({
            "available": state.downloaded,
            "legacyRetained": legacy_model, "legacyAudioRetained": legacy_audio,
            "repairAvailable": repair_available(&state),
            "removalPending": install::removal_pending(&self.root),
            "downloading": download.is_some(),
            "downloaded": download.map(|download| download.downloaded).unwrap_or(if state.downloaded { MODEL_BYTES } else { 0 }),
            "total": download.map(|download| download.total).unwrap_or(MODEL_BYTES),
            "voices": if state.downloaded { voice_entries() } else { Vec::<Value>::new() },
            "error": state.error,
            "performance": match state.performance {
                Some(performance) if performance.realtime() => "realtime",
                Some(_) => "slow",
                None => "unknown",
            },
            "probing": state.probing,
            "performanceRatio": state.performance.map(|performance| performance.ratio),
            "performanceError": state.performance_error,
            "bundled": download.is_some_and(|active| active.bundled) || install::bundled_record(&self.root) || self.root.join(BUNDLED_MARKER).is_file() ||
                self.root.join(BUNDLED_INSTALLING).is_file(),
        })
    }

    pub(crate) fn downloaded(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        refresh_downloaded(&self.root, &mut state);
        state.downloaded
    }

    /// Kokoro is offered only after this device has demonstrated that it can
    /// synthesize at least as fast as playback. Unknown and failed probes keep
    /// the system voice path, which is always available.
    pub(crate) fn usable(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        refresh_downloaded(&self.root, &mut state);
        state.downloaded && state.performance.is_some_and(Performance::realtime)
    }

    /// Measure real-time throughput once per installed model. The probe runs
    /// off the command executor and deliberately bypasses the WAV cache so a
    /// previous cached synthesis cannot masquerade as real-time hardware.
    pub(crate) fn ensure_performance_probe(self: &Arc<Self>) {
        let (generation, cancel, finished) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            refresh_downloaded(&self.root, &mut state);
            if !state.downloaded || state.performance.is_some() || state.probing ||
                state.performance_error.is_some() { return; }
            state.probing = true;
            state.probe_generation = state.probe_generation.wrapping_add(1);
            let cancel = CancellationToken::new();
            let finished = CancellationToken::new();
            state.probe = Some((state.probe_generation, cancel.clone(), finished.clone()));
            (state.probe_generation, cancel, finished)
        };
        let manager = self.clone();
        let finished = finished.drop_guard();
        if let Err(error) = thread::Builder::new().name("kokoro-performance".into()).spawn(move || {
            let _finished = finished;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| manager.measure_performance(&cancel)))
                .unwrap_or_else(|_| Err("The natural voice performance check failed unexpectedly".into()));
            {
                let mut state = manager.state.lock().unwrap_or_else(|error| error.into_inner());
                refresh_downloaded(&manager.root, &mut state);
                if !complete_probe(&mut state, generation) { return; }
                match result {
                    Ok(performance) => {
                        if !performance.realtime() { state.engine = None; }
                        state.performance = Some(performance);
                        state.performance_error = None;
                        let _ = install::clear_state(&manager.root, PERFORMANCE_ERROR_FILE);
                        // Keep the publish under the state lock so a concurrent
                        // delete cannot remove the model and then have this
                        // thread recreate a stale performance file.
                        if let Some(source)=&state.source { let _ = save_value(&manager.root, PERFORMANCE_FILE,source,performance); }
                    }
                    Err(error) => {
                        state.engine = None;
                        state.performance_error = Some(error.clone());
                        if let Some(source)=&state.source { let _ = save_value(&manager.root,PERFORMANCE_ERROR_FILE,source,error); }
                    }
                }
            }
        }) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !complete_probe(&mut state, generation) { return; }
            state.performance_error = Some(format!("Starting the natural voice performance check failed: {error}"));
        }
    }

    /// Clear a cached verdict and start a fresh measurement. Used by the
    /// Settings retry control after thermal or load conditions change.
    pub(crate) fn reprobe(self: &Arc<Self>) {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !state.downloaded || state.probing { return; }
            state.performance = None;
            state.performance_error = None;
            state.probing = false;
            state.probe_generation = state.probe_generation.wrapping_add(1);
        }
        let _ = install::clear_state(&self.root, PERFORMANCE_FILE);
        let _ = install::clear_state(&self.root, PERFORMANCE_ERROR_FILE);
        self.ensure_performance_probe();
    }

    fn measure_performance(&self, cancel: &CancellationToken) -> Result<Performance, String> {
        let engine = self.engine(Some(cancel))?;
        if cancel.is_cancelled() { return Err("The natural voice performance check was cancelled".into()); }
        let started = Instant::now();
        let g2p = G2P::new(Language::EnglishUS);
        let (phonemes, _) = g2p.g2p(PROBE_TEXT)
            .map_err(|error| format!("Preparing the natural voice performance check failed: {error}"))?;
        let phonemes: Vec<i32> = phonemes.chars().filter(|character| *character != '❓')
            .map(|character| character as i32).collect();
        if phonemes.is_empty() { return Err("The natural voice performance check found no pronounceable text".into()); }
        let config = GenerationConfig {
            sid: 11, speed: 1.0,
            phoneme_codepoints: Some(vec![phonemes]),
            ..Default::default()
        };
        let cancel = cancel.clone();
        let audio = engine.generate_with_config(PROBE_TEXT, &config, Some(move |_: &[f32], _: f32| !cancel.is_cancelled()))
            .ok_or("The natural voice performance check produced no audio")?;
        let elapsed = started.elapsed().as_secs_f64();
        let audio_seconds = audio.samples().len() as f64 / audio.sample_rate() as f64;
        if !elapsed.is_finite() || !audio_seconds.is_finite() || audio_seconds <= 0.0 {
            return Err("The natural voice performance check produced no measurable audio".into());
        }
        Ok(Performance { ratio: elapsed / audio_seconds })
    }

    pub(crate) fn begin_download(&self) -> Result<Arc<DownloadControl>, String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.download.is_some() { return Err("The voice model is already downloading".into()); }
        refresh_downloaded(&self.root, &mut state);
        if state.downloaded && !repair_available(&state) { return Err("The voice model is already installed".into()); }
        Ok(Self::new_download(&mut state, false, false))
    }

    #[cfg(any(target_os = "android", test))]
    pub(crate) fn begin_bundled(&self) -> Result<Option<Arc<DownloadControl>>, String> {
        self.begin_bundled_mode(false)
    }

    #[cfg(any(target_os = "android", test))]
    pub(crate) fn begin_bundled_explicit(&self) -> Result<Option<Arc<DownloadControl>>, String> {
        self.begin_bundled_mode(true)
    }

    #[cfg(any(target_os = "android", test))]
    fn begin_bundled_mode(&self, explicit: bool) -> Result<Option<Arc<DownloadControl>>, String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        refresh_downloaded(&self.root, &mut state);
        if state.download.is_some() || (state.downloaded && (!explicit || !repair_available(&state)))
            || (!explicit && self.root.join(BUNDLED_REMOVED).try_exists().map_err(|e| e.to_string())?) {
            return Ok(None);
        }
        Ok(Some(Self::new_download(&mut state, true, explicit)))
    }

    fn new_download(state: &mut State, bundled: bool, explicit_bundled: bool) -> Arc<DownloadControl> {
        let cancel = Arc::new(DownloadControl { explicit_bundled, ..DownloadControl::new() });
        state.error = None;
        state.last_job = Some(cancel.clone());
        state.download = Some(Download { downloaded: 0, total: MODEL_BYTES, bundled, cancel: cancel.clone(), finished: CancellationToken::new() });
        cancel
    }

    pub(crate) fn download_progress(&self, downloaded: u64, total: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(download) = state.download.as_mut() {
            download.downloaded = downloaded;
            download.total = total.max(downloaded);
        }
    }

    pub(crate) fn finish_download(&self, result: Result<(), String>) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let download = state.download.take().ok_or("The natural voice installation is no longer active")?;
        let _finished = download.finished.drop_guard();
        let result = result.and_then(|()| {
            let actual = install::source_stamp(&self.root).map_err(|e| format!("Reopening the installed natural voice model failed: {e}"))?;
            if download.cancel.selected.lock().unwrap_or_else(|e| e.into_inner()).as_ref() != Some(&actual) {
                return Err("The selected natural voice model changed. Check Settings and retry.".into());
            }
            state.source = Some(actual);
            state.downloaded = true;
            Ok(())
        });
        match result {
            Ok(()) => {
                state.error = None;
                state.performance = None;
                state.performance_error = None;
                state.probing = false;
                state.probe_generation = state.probe_generation.wrapping_add(1);
                let _ = install::clear_state(&self.root, PERFORMANCE_FILE);
                let _ = install::clear_state(&self.root, PERFORMANCE_ERROR_FILE);
                Ok(())
            }
            Err(error) => {
                state.error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn report_job(&self, job: &Arc<DownloadControl>, success: bool, emit: impl FnOnce()) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        refresh_downloaded(&self.root, &mut state);
        if !state.last_job.as_ref().is_some_and(|last| Arc::ptr_eq(last,job)) ||
            (success && (!state.downloaded || state.source.as_ref() != job.selected.lock().unwrap_or_else(|e| e.into_inner()).as_ref())) { return false; }
        emit();
        true
    }

    pub(crate) fn cancel_download(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.download.as_ref().is_some_and(|download| download.cancel.stop())
    }

    /// Call under the service's exclusive mode-change guard so a new download
    /// cannot start while its predecessor is discarding its staging files.
    pub(crate) async fn drain_download(&self) {
        let (download, probe) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.last_job = None;
            state.probe_generation = state.probe_generation.wrapping_add(1);
            if let Some(active) = state.active.as_ref() {
                active.stopped.store(true, Ordering::SeqCst);
                let _ = active.commands.send(Command::Stop);
            }
            let probe = state.probe.take().map(|(_, cancel, finished)| {
                cancel.cancel();
                finished
            });
            let download = state.download.as_ref().map(|download| {
                download.cancel.stop();
                download.finished.clone()
            });
            (download, probe)
        };
        if let Some(finished) = download { finished.cancelled().await; }
        if let Some(finished) = probe { finished.cancelled().await; }
        self.state.lock().unwrap_or_else(|error| error.into_inner()).probing = false;
    }

    pub(crate) fn delete(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.download.is_some() { return Err("Wait for the voice model download to finish or cancel it first".into()); }
        state.last_job = None;
        if let Some(active) = state.active.take() { active.stopped.store(true, Ordering::SeqCst); let _ = active.commands.send(Command::Stop); }
        state.engine = None;
        let cache = self.root.join("voice-audio-v2");
        // Owned generations carry an installation receipt. Older model trees are preserved.
        install::delete_model(&self.root).map_err(|e| format!("Removing the natural voice model failed. Its remaining files were preserved: {e}"))?;
        let cache_failure = install::clean_cache(&cache, true).err().map(|e| format!("Removing cached natural voice audio failed: {e}"));
        // Clear the verdict even when file removal partially fails: the model
        // is no longer advertised, and a later reinstall must not inherit it.
        // Reflect the filesystem if removal failed part-way; otherwise a
        // partially removed model would reappear on the next launch.
        state.source = install::source_stamp(&self.root).ok();
        state.downloaded = state.source.is_some();
        state.error = None;
        state.performance = None;
        state.performance_error = None;
        state.probing = false;
        state.probe_generation = state.probe_generation.wrapping_add(1);
        let _ = install::clear_state(&self.root, PERFORMANCE_FILE);
        let _ = install::clear_state(&self.root, PERFORMANCE_ERROR_FILE);
        if let Some(error) = cache_failure { return Err(error); }
        Ok(())
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        request: StartRequest,
        events: tokio::sync::mpsc::UnboundedSender<Value>,
    ) -> Result<(), String> {
        if !self.downloaded() { return Err("Download the natural voice model in Settings first".into()); }
        if !self.usable() { return Err("Natural voices are disabled on this device because synthesis cannot keep up in real time; choose a system voice".into()); }
        let sid = voice_sid(&request.voice).ok_or("Unknown natural voice")?;
        let chunks = chunks(&request.text);
        if chunks.is_empty() { return Err("The article has no readable text".into()); }
        let (commands, receiver) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let offset = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(previous) = state.active.take() {
                previous.stopped.store(true, Ordering::SeqCst);
                let _ = previous.commands.send(Command::Stop);
            }
            state.active = Some(Active {
                id: request.id.clone(), token: request.token.clone(),
                commands, stopped: stopped.clone(), offset: offset.clone(),
            });
        }
        let manager = self.clone();
        let worker_id = request.id.clone();
        if let Err(error) = thread::Builder::new().name("kokoro-read-aloud".into()).spawn(move || {
            let result = run(&manager, request, sid, chunks, receiver, stopped.clone(), offset.clone(), &events);
            if let Err(error) = result {
                let _ = events.send(json!({"id": worker_id, "speaking": false, "paused": false, "offset": 0, "error": error}));
            }
            let mut state = manager.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.active.as_ref().is_some_and(|active| active.stopped.as_ptr() == stopped.as_ptr()) {
                state.active = None;
            }
        }) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.active = None;
            return Err(format!("Starting read-aloud failed: {error}"));
        }
        Ok(())
    }

    pub(crate) fn command(&self, id: &str, token: &str, command: Command) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(active) = state.active.as_ref() else { return false; };
        if active.id != id || active.token != token { return false; }
        active.commands.send(command).is_ok()
    }

    pub(crate) fn stop(&self, id: &str, token: &str) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(active) = state.active.as_ref() else { return false; };
        if active.id != id || active.token != token { return false; }
        // Set the flag as well as sending Stop, so a worker busy synthesizing
        // neither plays nor prefetches another chunk before it reads the command.
        active.stopped.store(true, Ordering::SeqCst);
        active.commands.send(Command::Stop).is_ok()
    }

    pub(crate) fn active_identity(&self) -> Option<(String, String)> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active.as_ref().map(|active| (active.id.clone(), active.token.clone()))
    }

    pub(crate) fn active_offset(&self, id: &str, token: &str) -> Option<usize> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active.as_ref().filter(|active| active.id == id && active.token == token)
            .map(|active| active.offset.load(Ordering::SeqCst))
    }

    fn engine(&self, cancel: Option<&CancellationToken>) -> Result<Arc<OfflineTts>, String> {
        let generation = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(engine) = state.engine.as_ref() { return Ok(engine.clone()); }
            state.probe_generation
        };
        let model = install::engine_files(&self.root, cancel).map_err(|e| format!("Verifying the natural voice model failed: {e}"))?;
        let dir = &model.dir;
        let config = OfflineTtsConfig {
            model: sherpa_onnx::OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: Some(path_string(&dir.join("model.fp16.onnx"))),
                    voices: Some(path_string(&dir.join("voices.bin"))),
                    tokens: Some(path_string(&dir.join("tokens.txt"))),
                    length_scale: 1.0,
                    ..Default::default()
                },
                num_threads: 4,
                debug: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = OfflineTts::create(&config).ok_or("The natural voice engine could not start")?;
        model.unchanged().map_err(|e| format!("The natural voice model changed while loading: {e}"))?;
        let engine = Arc::new(engine);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.probe_generation != generation || !state.downloaded { return Err("The natural voice model changed before it could be loaded".into()); }
        state.engine = Some(engine.clone());
        Ok(engine)
    }

    // Stop, replacement, privacy and deletion take the same lock. A worker that
    // was running native synthesis cannot perform later cache I/O after retirement.
    fn with_playback<T>(&self, stopped: &Arc<AtomicBool>, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if stopped.load(Ordering::SeqCst) || state.active.as_ref().is_none_or(|active| !Arc::ptr_eq(&active.stopped, stopped)) {
            return Err("The natural voice session ended".into());
        }
        f()
    }

    fn synthesize(&self, engine: &OfflineTts, g2p: &G2P, text: &str, sid: i32, rate: f64, stopped: &Arc<AtomicBool>) -> Result<File, String> {
        let cache = self.root.join("voice-audio-v2");
        let key = format!("misaki-v1\u{0}{text}\u{0}{sid}\u{0}{rate:.3}");
        let digest = chksum_hash_sha2_256::hash(key.as_bytes()).to_hex_lowercase();
        let output = cache.join(format!("{digest}.wav"));
        let name = output.file_name().ok_or("Invalid voice cache file name")?;
        if let Some(file) = self.with_playback(stopped, || install::cached_audio(&cache, name).map_err(|e| format!("Reading cached natural voice audio failed: {e}")))? {
            return Ok(file);
        }
        let spoken = normalize_currency(text);
        let (phonemes, _) = g2p.g2p(&spoken).map_err(|error| format!("Preparing pronunciation failed: {error}"))?;
        let phonemes: Vec<i32> = phonemes.chars().filter(|character| *character != '❓')
            .map(|character| character as i32).collect();
        if phonemes.is_empty() { return Err("The natural voice found no pronounceable text".into()); }
        let config = GenerationConfig {
            sid, speed: rate as f32,
            phoneme_codepoints: Some(vec![phonemes]),
            ..Default::default()
        };
        let running = stopped.clone();
        let audio = engine.generate_with_config(text, &config, Some(move |_: &[f32], _: f32| !running.load(Ordering::SeqCst)))
            .ok_or("The natural voice produced no audio")?;
        let wav = wav_bytes(audio.samples(), audio.sample_rate());
        self.with_playback(stopped, || {
            let file = install::publish_audio(&cache, Path::new(name), &wav).map_err(|e| format!("Saving cached natural voice audio failed: {e}"))?;
            let _ = install::clean_cache(&cache, false);
            Ok(file)
        })
    }
}

/// UTF-16 offsets of sentence starts inside one synthesized chunk. Kokoro
/// produces a single WAV per chunk, so paint the sentence boundaries while
/// that WAV plays instead of waiting for the next chunk.
fn sentence_starts(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = vec![0usize];
    let mut utf16 = 0usize;
    let mut boundary = false;
    let mut last_start = 0usize;
    for (index, ch) in chars.iter().enumerate() {
        if boundary && !ch.is_whitespace() {
            starts.push(utf16);
            boundary = false;
            last_start = utf16;
        }
        let ends_sentence = matches!(ch, '.' | '!' | '?' | '\n')
            && chars.get(index + 1).is_none_or(|next| next.is_whitespace())
            && utf16.saturating_sub(last_start) >= 20;
        if ends_sentence { boundary = true; }
        utf16 += ch.len_utf16();
    }
    starts
}

fn wav_seconds(file: &File) -> Option<f64> {
    let bytes = file.metadata().ok()?.len().checked_sub(44)?;
    Some(bytes as f64 / (2.0 * 24_000.0))
}

fn spawn_prefetch(
    manager: Arc<KokoroManager>, engine: Arc<OfflineTts>, g2p: Arc<G2P>,
    text: String, sid: i32, rate: f64, stopped: Arc<AtomicBool>,
) -> Option<thread::JoinHandle<Result<File, String>>> {
    thread::Builder::new().name("kokoro-synth-prefetch".into()).spawn(move || {
        manager.synthesize(&engine, &g2p, &text, sid, rate, &stopped)
    }).ok()
}

#[allow(unused_assignments, clippy::too_many_arguments)]
fn run(
    manager: &Arc<KokoroManager>,
    request: StartRequest,
    sid: i32,
    chunks: Vec<Chunk>,
    commands: mpsc::Receiver<Command>,
    stopped: Arc<AtomicBool>,
    offset: Arc<std::sync::atomic::AtomicUsize>,
    events: &tokio::sync::mpsc::UnboundedSender<Value>,
) -> Result<(), String> {
    use rodio::{Decoder, OutputStream, Sink};
    if stopped.load(Ordering::SeqCst) { return Ok(()); }
    let engine = manager.engine(None)?;
    let g2p = Arc::new(G2P::new(if request.voice.starts_with("kokoro:b") { Language::EnglishGB } else { Language::EnglishUS }));
    let total_utf16 = request.text.encode_utf16().count() as u64;
    let activity = |speaking: bool, paused: bool, offset: usize| json!({
        "id": &request.id, "speaking": speaking, "paused": paused, "offset": offset,
        "title": &request.title, "voice": &request.voice, "total": total_utf16,
        "statusLabel": if paused { &request.paused_label } else { &request.reading_label },
    });
    let (_stream, stream) = OutputStream::try_default()
        .map_err(|error| format!("Opening the audio output failed: {error}"))?;
    let mut sink: Option<Sink> = None;
    let mut prefetched: Option<(usize, thread::JoinHandle<Result<File, String>>)> = None;
    let mut index = 0usize;
    let mut paused = false;
    loop {
        if stopped.load(Ordering::SeqCst) { return Ok(()); }
        if index >= chunks.len() {
            let _ = events.send(activity(false, false, request.text.encode_utf16().count()));
            return Ok(());
        }
        let chunk = &chunks[index];
        offset.store(chunk.start_utf16, Ordering::SeqCst);
        if paused {
            match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(Command::Resume) => { paused = false; }
                Ok(Command::Stop) => return Ok(()),
                Ok(Command::Previous) => { index = index.saturating_sub(1); }
                Ok(Command::Next) => { index = (index + 1).min(chunks.len()); }
                Ok(Command::Seek(offset)) => { index = chunk_index(&chunks, offset); }
                Ok(Command::Rewind(seconds)) => { index = chunk_index(&chunks, chunk.start_utf16.saturating_sub((seconds * 15.0) as usize)); }
                Ok(Command::Forward(seconds)) => { index = chunk_index(&chunks, chunk.start_utf16.saturating_add((seconds * 15.0) as usize)); }
                Ok(Command::Pause) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            continue;
        }
        let file = if let Some((prefetched_index, handle)) = prefetched.take() {
            if prefetched_index == index {
                handle.join().map_err(|_| "The natural voice prefetch worker panicked")??
            } else {
                manager.synthesize(&engine, &g2p, &chunk.text, sid, request.rate, &stopped)?
            }
        } else {
            manager.synthesize(&engine, &g2p, &chunk.text, sid, request.rate, &stopped)?
        };
        // A superseded or stopped session must not play, or synthesize and
        // cache, even one more chunk.
        if stopped.load(Ordering::SeqCst) { return Ok(()); }
        if index + 1 < chunks.len() && prefetched.is_none() {
            prefetched = spawn_prefetch(manager.clone(), engine.clone(), g2p.clone(),
                chunks[index + 1].text.clone(), sid, request.rate, stopped.clone()).map(|handle| (index + 1, handle));
        }
        let _ = events.send(activity(true, false, chunk.start_utf16));
        let duration = wav_seconds(&file).unwrap_or(0.0);
        let source = Decoder::new(BufReader::new(file)).map_err(|error| format!("Decoding synthesized audio failed: {error}"))?;
        let next = Sink::try_new(&stream)
            .map_err(|error| format!("Starting the audio output failed: {error}"))?;
        manager.with_playback(&stopped, || { next.append(source); Ok(()) })?;
        sink = Some(next);
        let starts = sentence_starts(&chunk.text);
        let chunk_utf16 = chunk.text.encode_utf16().count().max(1);
        let mut next_sentence = 1usize;
        let mut advance = true;
        loop {
            if stopped.load(Ordering::SeqCst) { return Ok(()); }
            match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(Command::Pause) => {
                    if let Some(sink) = sink.as_ref() { sink.pause(); }
                    paused = true;
                    advance = false;
                    let _ = events.send(activity(false, true, chunk.start_utf16));
                    break;
                }
                Ok(Command::Stop) => return Ok(()),
                Ok(Command::Previous) => { if let Some(sink) = sink.as_ref() { sink.stop(); } index = index.saturating_sub(1); advance = false; break; }
                Ok(Command::Next) => { if let Some(sink) = sink.as_ref() { sink.stop(); } index = (index + 1).min(chunks.len()); advance = false; break; }
                Ok(Command::Seek(offset)) => { if let Some(sink) = sink.as_ref() { sink.stop(); } index = chunk_index(&chunks, offset); advance = false; break; }
                Ok(Command::Rewind(seconds)) => { if let Some(sink) = sink.as_ref() { sink.stop(); } index = chunk_index(&chunks, chunk.start_utf16.saturating_sub((seconds * 15.0) as usize)); advance = false; break; }
                Ok(Command::Forward(seconds)) => { if let Some(sink) = sink.as_ref() { sink.stop(); } index = chunk_index(&chunks, chunk.start_utf16.saturating_add((seconds * 15.0) as usize)); advance = false; break; }
                Ok(Command::Resume) => {}
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(player) = sink.as_ref() {
                        let elapsed = player.get_pos().as_secs_f64();
                        while next_sentence < starts.len() && duration > 0.0 {
                            let fraction = starts[next_sentence] as f64 / chunk_utf16 as f64;
                            if elapsed + 0.05 < duration * fraction { break; }
                            let _ = events.send(activity(true, false,
                                chunk.start_utf16 + starts[next_sentence]));
                            next_sentence += 1;
                        }
                    }
                    if sink.as_ref().is_some_and(|sink| sink.empty()) {
                        if let Some(sink) = sink.take() { sink.sleep_until_end(); }
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
        sink = None;
        if advance { index += 1; }
    }
}

fn chunk_index(chunks: &[Chunk], offset: usize) -> usize {
    chunks.iter().position(|chunk| chunk.start_utf16 >= offset)
        .unwrap_or_else(|| chunks.len().saturating_sub(1))
}

fn chunk_boundary(chars: &[(usize, char)], index: usize) -> bool {
    let character = chars[index].1;
    if character == '\n' { return true; }
    if !matches!(character, '.' | '!' | '?' | ';' | ':') { return false; }
    // A decimal, version, URL, or time must not become a speech boundary.
    chars.get(index + 1).is_none_or(|(_, next)| next.is_whitespace())
}

fn chunks(text: &str) -> Vec<Chunk> {
    let mut result = Vec::new();
    let mut start = 0usize;
    let mut start_utf16 = 0usize;
    let mut utf16 = 0usize;
    let mut last_boundary = 0usize;
    let mut last_utf16 = 0usize;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (index, &(byte, ch)) in chars.iter().enumerate() {
        let width = ch.len_utf16();
        let end = byte + ch.len_utf8();
        let boundary = chunk_boundary(&chars, index);
        if boundary {
            last_boundary = end;
            last_utf16 = utf16 + width;
        }
        utf16 += width;
        let too_long = end - start >= 700;
        let end_of_text = index + 1 == chars.len();
        if (boundary && end - start >= 20) || too_long || end_of_text {
            let end = if too_long && last_boundary > start { last_boundary } else { end };
            let end_utf16 = if too_long && last_boundary > start { last_utf16 } else { utf16 };
            let piece = &text[start..end];
            let trimmed = piece.trim();
            if !trimmed.is_empty() {
                let leading = piece.len().saturating_sub(piece.trim_start().len());
                let leading_utf16 = piece[..leading].encode_utf16().count();
                result.push(Chunk { text: trimmed.to_owned(), start_utf16: start_utf16.saturating_add(leading_utf16) });
            }
            start = end;
            start_utf16 = end_utf16;
            last_boundary = start;
            last_utf16 = start_utf16;
        }
    }
    result
}

fn wav_bytes(samples: &[f32], sample_rate: i32) -> Vec<u8> {
    let data_len = samples.len().saturating_mul(2).min(u32::MAX as usize) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sample_rate as u32).to_le_bytes());
    out.extend_from_slice(&((sample_rate as u32).saturating_mul(2)).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn repair_available(state: &State) -> bool {
    state.performance_error.as_deref().is_some_and(|e| e.starts_with("Verifying the natural voice model failed"))
}

fn complete_probe(state: &mut State, generation: u64) -> bool {
    if !state.probe.as_ref().is_some_and(|(job,_,_)| *job == generation) { return false; }
    state.probing = false;
    state.probe = None;
    state.probe_generation == generation && state.downloaded
}

fn refresh_downloaded(root: &Path, state: &mut State) {
    let source = install::source_stamp(root).ok();
    if source == state.source { return; }
    state.downloaded = source.is_some();
    state.source = source;
    state.engine = None;
    state.performance = None;
    state.performance_error = None;
    if let Some((_,cancel,_)) = &state.probe { cancel.cancel(); }
    if let Some(active) = &state.active { active.stopped.store(true, Ordering::SeqCst); let _ = active.commands.send(Command::Stop); }
    state.probe_generation = state.probe_generation.wrapping_add(1);
}

fn load_value<T: serde::de::DeserializeOwned>(root: &Path, name: &str, source: &install::SourceStamp) -> Option<T> {
    let record: Verdict<T> = serde_json::from_slice(&install::read_state(root,name).ok()?).ok()?;
    (record.source == *source).then_some(record.value)
}

fn save_value<T: Serialize>(root: &Path, name: &str, source: &install::SourceStamp, value: T) -> Result<(), String> {
    let bytes=serde_json::to_vec(&Verdict {source:source.clone(),value}).map_err(|e|e.to_string())?;
    install::save_state(root,name,&bytes).map_err(|e|format!("Saving the natural voice performance result failed: {e}"))
}

fn path_string(path: &Path) -> String { path.to_string_lossy().into_owned() }

#[cfg(test)]
fn ready(root: &Path) -> bool { install::location(root).is_ok() }

/// Keep the bytes being authenticated on a private file handle through extraction.
struct PackDownload {
    file: File,
    hash: chksum_hash_sha2_256::Update,
    bytes: u64,
}

impl PackDownload {
    fn new(file: File) -> Self { Self { file, hash: chksum_hash_sha2_256::new(), bytes: 0 } }

    fn append(&mut self, chunk: &[u8]) -> Result<(), String> {
        if chunk.len() as u64 > MODEL_BYTES - self.bytes {
            return Err("The voice model download exceeded its pinned size".into());
        }
        self.file.write_all(chunk).map_err(|error| format!("Writing the voice model download failed: {error}"))?;
        self.hash.update(chunk);
        self.bytes += chunk.len() as u64;
        Ok(())
    }

    fn verified_file(mut self, size: u64, sha256: &str) -> Result<File, String> {
        if self.bytes != size || self.hash.digest().to_hex_lowercase() != sha256 {
            return Err("The voice model download failed verification and was discarded".into());
        }
        self.file.rewind().map_err(|error| format!("Reading the verified voice model failed: {error}"))?;
        Ok(self.file)
    }
}

/// Download, verify and install the model. Runs on the async runtime; archive
/// extraction is moved to a blocking worker because it is large and CPU-bound.
pub(crate) async fn download(root: &Path, control: Arc<DownloadControl>, progress: impl Fn(u64, u64) + Send + Sync + 'static) -> Result<(), String> {
    let root = root.join("models");
    let cancel = control.cancel.clone();
    let lease = tokio::task::spawn_blocking(move || {
        let lease = install::Lease::for_install(&root, true, &cancel)?;
        lease.space_for(true)?;
        Ok::<_, std::io::Error>(lease)
    }).await.map_err(|e| format!("Preparing the natural voice download failed: {e}"))?
        .map_err(|e| format!("Preparing the natural voice download failed: {e}"))?;
    let internet = crate::transfer_policy::internet_revision()?;
    crate::transfer_policy::require_internet_since(internet)?;
    let verified = tokio::select! {
        biased;
        _ = control.cancel.cancelled() => return Err("The voice model download was cancelled".into()),
        _ = crate::transfer_policy::internet_denied_since(internet) => return Err("The network policy changed during the voice model download".into()),
        result = receive_pack(&lease, internet, progress) => result?,
    };
    tokio::task::spawn_blocking(move || install::extract(lease, verified, &control).map_err(|e| format!("Installing the natural voice model failed: {e}"))).await
        .map_err(|error| format!("Extracting the voice model failed: {error}"))?
}

#[cfg(any(target_os = "android", test))]
pub(crate) async fn install_bundled(data_dir: &Path, reader: impl std::io::Read + Send + 'static, control: Arc<DownloadControl>) -> Result<(), String> {
    let root = data_dir.join("models");
    tokio::task::spawn_blocking(move || {
        let lease = install::Lease::for_install(&root,control.explicit_bundled,&control.cancel)?;
        lease.space_for(false)?;
        install::bundled(lease, reader, &control)
    }).await.map_err(|e| format!("Preparing the bundled natural voice model failed: {e}"))?
        .map_err(|e| format!("Installing the bundled natural voice model failed: {e}"))
}

async fn receive_pack(lease: &install::Lease, internet: u64, progress: impl Fn(u64, u64)) -> Result<File, String> {
    let client = reqwest::Client::builder()
        .user_agent("kiwix-workbench")
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("Creating the download client failed: {error}"))?;
    let mut response = client.get(MODEL_URL).send().await
        .map_err(|error| format!("Downloading the voice model failed: {error}"))?;
    if !response.status().is_success() { return Err(format!("Downloading the voice model failed (HTTP {})", response.status())); }
    let total = MODEL_BYTES;
    let mut pack = PackDownload::new(lease.scratch().map_err(|e| format!("Creating the voice download scratch file failed: {e}"))?);
    let mut next_policy_check = 0;
    let mut last_report = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|error| format!("Downloading the voice model failed: {error}"))? {
        if pack.bytes >= next_policy_check {
            crate::transfer_policy::require_internet_since(internet)?;
            next_policy_check = pack.bytes + 4 * 1024 * 1024;
        }
        pack.append(&chunk)?;
        let downloaded = pack.bytes;
        if downloaded.saturating_sub(last_report) >= 512 * 1024 || downloaded == total {
            progress(downloaded, total);
            last_report = downloaded;
        }
    }
    crate::transfer_policy::require_internet_since(internet)?;
    let downloaded = pack.bytes;
    let verified = pack.verified_file(MODEL_BYTES, MODEL_SHA256)?;
    progress(downloaded, total);
    Ok(verified)
}

#[cfg(test)]
mod fixtures {
    use std::{fs, io, path::Path};

    pub(super) fn profile() -> io::Result<tempfile::TempDir> {
        #[cfg_attr(windows, allow(unused_mut))]
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        builder.tempdir()
    }

    pub(super) fn write_private(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
        use io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?.write_all(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn voice_pack_verification_retains_the_received_file() {
        use std::io::Read;
        let directory = tempfile::tempdir().unwrap();
        let mut pack = PackDownload::new(tempfile::tempfile_in(directory.path()).unwrap());
        pack.append(b"a").unwrap();
        pack.append(b"bc").unwrap();
        let named_archive = directory.path().join(MODEL_ARCHIVE);
        fs::write(&named_archive, b"other file").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut verified = pack.verified_file(3, sha).unwrap();
        let mut contents = Vec::new();
        verified.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"abc");
        assert_eq!(fs::read(named_archive).unwrap(), b"other file");
        for (size, digest) in [(2, sha), (3, "incorrect")] {
            let mut pack = PackDownload::new(tempfile::tempfile_in(directory.path()).unwrap());
            pack.append(b"abc").unwrap();
            assert!(pack.verified_file(size, digest).is_err());
        }
    }

    #[tokio::test]
    async fn cutover_waits_for_download_cleanup_before_admitting_a_retry() {
        let dir = fixtures::profile().unwrap();
        let manager = Arc::new(KokoroManager::new(dir.path()));
        let token = manager.begin_download().unwrap();
        let draining = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.drain_download().await })
        };
        token.cancel.cancelled().await;
        assert!(!draining.is_finished());
        assert!(manager.begin_download().is_err());
        manager.finish_download(Err("cancelled".into())).unwrap_err();
        draining.await.unwrap();
        assert!(!manager.begin_download().unwrap().cancel.is_cancelled());
    }

    /// Exercise the pinned distributor artifact from an explicitly supplied fixture.
    #[test]
    #[ignore = "requires KIWIX_TEST_KOKORO_PACK containing the pinned voice pack"]
    fn pinned_voice_pack_verifies_and_extracts_without_speech() {
        use std::io::Read;
        let mut source = File::open(std::env::var_os("KIWIX_TEST_KOKORO_PACK").unwrap()).unwrap();
        let directory = fixtures::profile().unwrap();
        let mut pack = PackDownload::new(tempfile::tempfile_in(directory.path()).unwrap());
        let mut chunk = vec![0; 128 * 1024];
        loop {
            let read = source.read(&mut chunk).unwrap();
            if read == 0 { break; }
            pack.append(&chunk[..read]).unwrap();
        }
        let verified = pack.verified_file(MODEL_BYTES, MODEL_SHA256).unwrap();
        // A replacement at the historical archive name must never reach the extractor.
        let decoy = directory.path().join(MODEL_ARCHIVE);
        fs::write(&decoy, b"unrelated").unwrap();
        let root = directory.path().canonicalize().unwrap().join("models");
        let partial=install::partial_fixture(&root,b"; 1\n: 2");
        let lease = install::Lease::admit(&root).unwrap();
        install::extract(lease, verified, &DownloadControl::new()).unwrap();
        assert!(!partial.exists());
        assert!(ready(&root));
        let checked = install::engine_files(&root, None).unwrap();
        checked.unchanged().unwrap();
        let tokens = checked.dir.join("tokens.txt");
        let original = fs::read(&tokens).unwrap();
        drop(checked);
        let mut altered = original.clone();
        altered[0] ^= 1;
        fs::write(&tokens, altered).unwrap();
        assert!(install::engine_files(&root, None).is_err());
        fs::write(&tokens, original).unwrap();
        let manager = KokoroManager::new(&directory.path().canonicalize().unwrap());
        assert_eq!(manager.status()["available"], true);
        install::delete_model(&root).unwrap();
        assert_eq!(manager.status()["available"], false);
        assert!(manager.begin_download().is_ok());
        assert!(!ready(&root));
        assert_eq!(fs::read(decoy).unwrap(), b"unrelated");
    }

    #[tokio::test]
    async fn explicit_bundled_retry_keeps_removal_choice_until_a_complete_install() {
        let profile = fixtures::profile().unwrap();
        let manager = KokoroManager::new(&profile.path().canonicalize().unwrap());
        let root = profile.path().canonicalize().unwrap().join("models");
        crate::fs_ext::create_private_dir_all(&root).unwrap();
        fixtures::write_private(root.join(BUNDLED_REMOVED), b"").unwrap();
        assert!(manager.begin_bundled().unwrap().is_none());
        let control = manager.begin_bundled_explicit().unwrap().unwrap();
        let result = install_bundled(&profile.path().canonicalize().unwrap(), std::io::empty(), control).await;
        assert!(manager.finish_download(result).is_err());
        assert!(root.join(BUNDLED_REMOVED).exists());
        assert!(manager.begin_bundled().unwrap().is_none());
    }

    #[tokio::test]
    async fn retired_synthesis_cannot_create_or_publish_article_audio() {
        let profile = fixtures::profile().unwrap();
        let manager = Arc::new(KokoroManager::new(&profile.path().canonicalize().unwrap()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (command, _receiver) = mpsc::channel();
        manager.state.lock().unwrap().active = Some(Active {
            id: "session".into(), token: "owner".into(), stopped: stopped.clone(),
            commands: command, offset: Arc::default(),
        });
        let (ready, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let file = profile.path().join("article-audio");
        let worker = {
            let manager = manager.clone();
            let file = file.clone();
            thread::spawn(move || {
                ready.send(()).unwrap();
                wait.recv().unwrap();
                manager.with_playback(&stopped, || fs::write(file, b"sample").map_err(|e| e.to_string()))
            })
        };
        started.await.unwrap();
        manager.drain_download().await;
        release.send(()).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(!file.exists());
    }

    #[test]
    fn failed_reopen_and_superseded_jobs_do_not_report_a_live_install() {
        let profile = fixtures::profile().unwrap();
        let manager=KokoroManager::new(&profile.path().canonicalize().unwrap());
        let old=manager.begin_download().unwrap();
        assert!(manager.finish_download(Ok(())).unwrap_err().contains("Reopening"));
        assert!(!manager.report_job(&old,true,|| panic!("unavailable")));
        assert!(manager.report_job(&old,false,|| {}));
        let _new=manager.begin_download().unwrap();
        assert!(!manager.report_job(&old,false,|| panic!("superseded")));
    }

    #[test]
    fn source_change_retires_only_its_probe_and_persisted_verdict() {
        let profile=fixtures::profile().unwrap();
        let base=profile.path().canonicalize().unwrap();
        let root=base.join("models");
        let model=root.join(MODEL_DIR);
        let make=|| {
            crate::fs_ext::create_private_dir_all(model.join("espeak-ng-data")).unwrap();
            for name in ["model.fp16.onnx","tokens.txt","voices.bin","lexicon-us-en.txt","lexicon-gb-en.txt"] { fixtures::write_private(model.join(name), b"fixture").unwrap(); }
        };
        make();
        let manager=KokoroManager::new(&base);
        let old=install::source_stamp(&root).unwrap();
        save_value(&root,PERFORMANCE_FILE,&old,Performance{ratio:0.42}).unwrap();
        let cancel=CancellationToken::new();
        {
            let mut state=manager.state.lock().unwrap();
            state.probe_generation=7;
            state.probe=Some((7,cancel.clone(),CancellationToken::new()));
            state.probing=true;
        }
        fs::rename(&model,root.join("retained-old")).unwrap();
        make();
        assert_eq!(manager.status()["available"],true);
        assert!(cancel.is_cancelled());
        let mut state=manager.state.lock().unwrap();
        assert!(!complete_probe(&mut state,7));
        assert!(!state.probing);
        state.probe=Some((9,CancellationToken::new(),CancellationToken::new()));
        state.probing=true;
        assert!(!complete_probe(&mut state,7));
        assert!(state.probing);
        drop(state);
        let restored=KokoroManager::new(&base);
        assert!(restored.state.lock().unwrap().performance.is_none());
    }

    #[test]
    fn publication_and_cancel_have_one_winner() {
        let control = DownloadControl::new();
        assert!(control.stop());
        assert!(control.cancel.is_cancelled());
        let control = DownloadControl::new();
        *control.publishing.lock().unwrap() = true;
        assert!(!control.stop());
        assert!(!control.cancel.is_cancelled());
    }

    #[tokio::test]
    #[ignore = "requires KIWIX_TEST_KOKORO_BUNDLED containing the verified factory asset"]
    async fn bundled_install_reopen_remove_and_explicitly_reinstall_without_speech() {
        let path = std::env::var_os("KIWIX_TEST_KOKORO_BUNDLED").unwrap();
        let profile = fixtures::profile().unwrap();
        let manager = KokoroManager::new(&profile.path().canonicalize().unwrap());
        let control = manager.begin_bundled().unwrap().unwrap();
        let result = install_bundled(&profile.path().canonicalize().unwrap(), File::open(&path).unwrap(), control).await;
        manager.finish_download(result).unwrap();
        let reopened = KokoroManager::new(&profile.path().canonicalize().unwrap());
        assert_eq!(reopened.status()["bundled"], true);
        let root = profile.path().canonicalize().unwrap().join("models");
        assert!(install::engine_files(&root, None).is_ok());
        let old=install::location(&root).unwrap();
        let retry=install::Lease::for_install(&root,true,&CancellationToken::new()).unwrap();
        assert!(install::bundled(retry,std::io::empty(),&DownloadControl::new()).is_err());
        assert_eq!(install::location(&root).unwrap(),old);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".voice-pending-")));
        reopened.delete().unwrap();
        assert!(reopened.begin_bundled().unwrap().is_none());
        let control = reopened.begin_bundled_explicit().unwrap().unwrap();
        let result = install_bundled(&profile.path().canonicalize().unwrap(), File::open(path).unwrap(), control).await;
        reopened.finish_download(result).unwrap();
        // An explicit repair may have been admitted after a transient read
        // failure. A fully authenticated old generation can be reclaimed.
        let previous=install::location(&root).unwrap();
        let lease=install::Lease::for_install(&root,true,&CancellationToken::new()).unwrap();
        let input=File::open(std::env::var_os("KIWIX_TEST_KOKORO_BUNDLED").unwrap()).unwrap();
        let control=DownloadControl { explicit_bundled:true,..DownloadControl::new() };
        install::bundled(lease,input,&control).unwrap();
        assert!(!previous.exists());
        assert!(root.join(BUNDLED_REMOVED).exists());
        assert_eq!(reopened.status()["available"],true);
        assert!(install::engine_files(&root, None).is_ok());
    }

    #[test]
    fn realtime_requires_synthesis_within_playback_time() {
        assert!(realtime_for(0.25));
        assert!(realtime_for(1.0));
        assert!(realtime_for(1.10));
        assert!(!realtime_for(1.11));
        assert!(!realtime_for(3.3));
        assert!(!realtime_for(f64::NAN));
        assert!(!realtime_for(0.0));
    }

    #[test]
    fn chunks_keep_utf16_offsets_and_boundaries() {
        let text = "First sentence is definitely long enough. Second sentence is also long enough!\nThird paragraph.";
        let pieces = chunks(text);
        assert!(pieces.len() >= 2);
        assert_eq!(pieces[0].start_utf16, 0);
        assert!(pieces[0].text.starts_with("First"));
        let joined: String = pieces.iter().map(|piece| piece.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("Second"));
        assert!(pieces.iter().any(|piece| piece.text.contains("Second")));
        assert_eq!(pieces.iter().find(|piece| piece.text.contains("Second")).unwrap().start_utf16,
            text[..text.find("Second").unwrap()].encode_utf16().count());
        assert_eq!(pieces.last().unwrap().start_utf16,
            text[..text.rfind("Third").unwrap()].encode_utf16().count());
    }

    #[test]
    fn sentence_starts_are_utf16_offsets_inside_a_chunk() {
        let text = "A sentence long enough. Two sentences are also long enough!";
        let starts = sentence_starts(text);
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[1], text.find("Two").unwrap());
        // A decimal/abbreviation inside a sentence is not a sentence boundary.
        assert_eq!(sentence_starts("The value 3.5 percent is long enough."), vec![0]);
    }

    #[test]
    fn chunks_do_not_split_decimal_points() {
        let text = "The adult animal is about 33.6 meters long and weighs roughly 1,234.5 kilograms.";
        let pieces = chunks(text);
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].text.contains("33.6 meters"));
        assert!(pieces[0].text.contains("1,234.5 kilograms"));
    }

    #[test]
    fn voice_ids_are_stable_and_unknown_voices_fail() {
        assert_eq!(voice_sid("kokoro:af_heart"), Some(3));
        assert_eq!(voice_sid("af_heart"), None);
        assert!(voice_entries().iter().any(|voice| voice["voiceURI"] == "kokoro:af_heart"));
    }

    #[test]
    fn currency_magnitude_moves_after_the_number() {
        assert_eq!(normalize_currency("$500 million"), "500 million dollars");
        assert_eq!(normalize_currency("$1.5bn"), "1.5 billion dollars");
        assert_eq!(normalize_currency("costs $2,000"), "costs $2,000");
    }

    #[test]
    fn wav_header_is_mono_16_bit() {
        let wav = wav_bytes(&[0.0, 0.5, -0.5], 24_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        assert_eq!(wav.len(), 44 + 6);
    }
}
