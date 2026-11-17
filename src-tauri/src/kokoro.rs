// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline Kokoro read-aloud.
//!
//! The model is an explicit, user-initiated download. Until it is present the
//! existing platform voice path remains the only engine. Synthesis and playback
//! run on a dedicated thread because sherpa-onnx is CPU-bound and rodio owns a
//! platform audio stream; neither may block the Tauri command executor.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufReader, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Arc, LazyLock, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use misaki_rs::{G2P, Language};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig};

/// Pinned model pack. The bytes feed a native ONNX parser, so the archive and
/// the extracted required files are both checked before they become visible.
const MODEL_DIR: &str = "kokoro-en-fp16-v1_0";
const MODEL_ARCHIVE: &str = "kokoro-en-fp16-v1_0.tar.bz2";
const MODEL_URL: &str =
    "https://github.com/jasontitus/CastCircle/releases/download/kokoro-en-fp16-v1/kokoro-en-fp16-v1_0.tar.bz2";
const MODEL_SHA256: &str = "4cafe1c49bf4b0a7f9c2fab9f2b010b05544dde2b28b66e3211e832308a1a1f9";
const MODEL_BYTES: u64 = 181_886_349;
const CACHE_BUDGET: u64 = 300 * 1024 * 1024;
const PERFORMANCE_FILE: &str = "performance.json";
const PERFORMANCE_ERROR_FILE: &str = "performance-error.json";
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
    downloaded: bool,
    download: Option<Download>,
    engine: Option<Arc<OfflineTts>>,
    active: Option<Active>,
    error: Option<String>,
    performance: Option<Performance>,
    probing: bool,
    performance_error: Option<String>,
    probe_generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct Performance {
    ratio: f64,
}

impl Performance {
    fn realtime(self) -> bool { realtime_for(self.ratio) }
}

fn realtime_for(ratio: f64) -> bool {
    ratio.is_finite() && ratio > 0.0 && ratio <= REALTIME_MARGIN
}

struct Download {
    downloaded: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
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
        let downloaded = ready(&root);
        let performance = downloaded.then(|| load_performance(&root)).flatten();
        let performance_error = downloaded.then(|| load_performance_error(&root)).flatten();
        Self { root, state: Mutex::new(State {
            downloaded, download: None, engine: None, active: None, error: None,
            performance, probing: false, performance_error, probe_generation: 0,
        }) }
    }

    pub(crate) fn status(&self) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        refresh_downloaded(&self.root, &mut state);
        let download = state.download.as_ref();
        json!({
            "available": state.downloaded,
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
            "bundled": self.root.join(BUNDLED_MARKER).is_file() ||
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
        let generation = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            refresh_downloaded(&self.root, &mut state);
            if !state.downloaded || state.performance.is_some() || state.probing ||
                state.performance_error.is_some() { return; }
            state.probing = true;
            state.probe_generation = state.probe_generation.wrapping_add(1);
            state.probe_generation
        };
        let manager = self.clone();
        if let Err(error) = thread::Builder::new().name("kokoro-performance".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| manager.measure_performance()))
                .unwrap_or_else(|_| Err("The natural voice performance check failed unexpectedly".into()));
            {
                let mut state = manager.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.probe_generation != generation || !state.downloaded { return; }
                state.probing = false;
                match result {
                    Ok(performance) => {
                        if !performance.realtime() { state.engine = None; }
                        state.performance = Some(performance);
                        state.performance_error = None;
                        let _ = fs::remove_file(manager.root.join(PERFORMANCE_ERROR_FILE));
                        // Keep the publish under the state lock so a concurrent
                        // delete cannot remove the model and then have this
                        // thread recreate a stale performance file.
                        let _ = save_performance(&manager.root, performance);
                    }
                    Err(error) => {
                        state.engine = None;
                        state.performance_error = Some(error.clone());
                        let _ = save_performance_error(&manager.root, &error);
                    }
                }
            }
        }) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.probing = false;
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
        let _ = fs::remove_file(self.root.join(PERFORMANCE_FILE));
        let _ = fs::remove_file(self.root.join(PERFORMANCE_ERROR_FILE));
        self.ensure_performance_probe();
    }

    fn measure_performance(&self) -> Result<Performance, String> {
        let engine = self.engine()?;
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
        let audio = engine.generate_with_config(PROBE_TEXT, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or("The natural voice performance check produced no audio")?;
        let elapsed = started.elapsed().as_secs_f64();
        let audio_seconds = audio.samples().len() as f64 / audio.sample_rate() as f64;
        if !elapsed.is_finite() || !audio_seconds.is_finite() || audio_seconds <= 0.0 {
            return Err("The natural voice performance check produced no measurable audio".into());
        }
        Ok(Performance { ratio: elapsed / audio_seconds })
    }

    pub(crate) fn begin_download(&self) -> Result<Arc<AtomicBool>, String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.download.is_some() { return Err("The voice model is already downloading".into()); }
        if state.downloaded { return Err("The voice model is already installed".into()); }
        if self.root.join(BUNDLED_MARKER).is_file() || self.root.join(BUNDLED_INSTALLING).is_file() {
            return Err("This build includes the natural voice model; no download is needed".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.error = None;
        state.download = Some(Download { downloaded: 0, total: MODEL_BYTES, cancel: cancel.clone() });
        Ok(cancel)
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
        state.download = None;
        match result {
            Ok(()) => {
                state.downloaded = true;
                state.error = None;
                state.performance = None;
                state.performance_error = None;
                state.probing = false;
                state.probe_generation = state.probe_generation.wrapping_add(1);
                let _ = fs::remove_file(self.root.join(PERFORMANCE_FILE));
                let _ = fs::remove_file(self.root.join(PERFORMANCE_ERROR_FILE));
                Ok(())
            }
            Err(error) => {
                state.error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn cancel_download(&self) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(download) = state.download.as_ref() { download.cancel.store(true, Ordering::SeqCst); }
    }

    pub(crate) fn delete(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.download.is_some() { return Err("Wait for the voice model download to finish or cancel it first".into()); }
        if let Some(active) = state.active.take() { active.stopped.store(true, Ordering::SeqCst); let _ = active.commands.send(Command::Stop); }
        state.engine = None;
        let dir = self.root.join(MODEL_DIR);
        let cache = self.root.join("cache");
        let bundled = self.root.join(BUNDLED_MARKER).is_file() || self.root.join(BUNDLED_INSTALLING).is_file();
        let model_failure = if dir.exists() {
            fs::remove_dir_all(&dir).err().map(|error| format!("Removing the voice model failed: {error}"))
        } else { None };
        let cache_failure = if cache.exists() {
            fs::remove_dir_all(&cache).err().map(|error| format!("Removing the voice cache failed: {error}"))
        } else { None };
        // Clear the verdict even when file removal partially fails: the model
        // is no longer advertised, and a later reinstall must not inherit it.
        // Reflect the filesystem if removal failed part-way; otherwise a
        // partially removed model would reappear on the next launch.
        state.downloaded = ready(&self.root);
        state.error = None;
        state.performance = None;
        state.performance_error = None;
        state.probing = false;
        state.probe_generation = state.probe_generation.wrapping_add(1);
        let _ = fs::remove_file(self.root.join(PERFORMANCE_FILE));
        let _ = fs::remove_file(self.root.join(PERFORMANCE_ERROR_FILE));
        let _ = fs::remove_file(self.root.join(BUNDLED_MARKER));
        let _ = fs::remove_file(self.root.join(BUNDLED_INSTALLING));
        let _ = fs::remove_file(self.root.join(MODEL_ARCHIVE));
        let _ = fs::remove_file(self.root.join(format!("{MODEL_ARCHIVE}.part")));
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(".extract-") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        if bundled { let _ = fs::write(self.root.join(BUNDLED_REMOVED), b""); }
        if let Some(error) = model_failure.or(cache_failure) { return Err(error); }
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

    fn engine(&self) -> Result<Arc<OfflineTts>, String> {
        {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(engine) = state.engine.as_ref() { return Ok(engine.clone()); }
        }
        let dir = self.root.join(MODEL_DIR);
        if !ready(&self.root) { return Err("The natural voice model is not installed".into()); }
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
        let engine = Arc::new(engine);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.engine = Some(engine.clone());
        Ok(engine)
    }

    fn synthesize(&self, engine: &OfflineTts, g2p: &G2P, text: &str, sid: i32, rate: f64) -> Result<PathBuf, String> {
        let cache = self.root.join("cache");
        fs::create_dir_all(&cache).map_err(|error| format!("Preparing the voice cache failed: {error}"))?;
        let key = format!("misaki-v1\u{0}{text}\u{0}{sid}\u{0}{rate:.3}");
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        let output = cache.join(format!("{digest}.wav"));
        if output.is_file() { return Ok(output); }
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
        let audio = engine.generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or("The natural voice produced no audio")?;
        let temporary = cache.join(format!("{digest}.{}.wav.part", uuid::Uuid::new_v4()));
        let _ = fs::remove_file(&temporary);
        let wav = wav_bytes(audio.samples(), audio.sample_rate());
        fs::write(&temporary, wav).map_err(|error| format!("Writing synthesized audio failed: {error}"))?;
        fs::rename(&temporary, &output).map_err(|error| format!("Publishing synthesized audio failed: {error}"))?;
        prune_cache(&cache);
        Ok(output)
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

fn wav_seconds(path: &Path) -> Option<f64> {
    let bytes = fs::metadata(path).ok()?.len().checked_sub(44)?;
    Some(bytes as f64 / (2.0 * 24_000.0))
}

fn spawn_prefetch(
    manager: Arc<KokoroManager>, engine: Arc<OfflineTts>, g2p: Arc<G2P>,
    text: String, sid: i32, rate: f64,
) -> Option<thread::JoinHandle<Result<PathBuf, String>>> {
    thread::Builder::new().name("kokoro-synth-prefetch".into()).spawn(move || {
        manager.synthesize(&engine, &g2p, &text, sid, rate)
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
    let engine = manager.engine()?;
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
    let mut prefetched: Option<(usize, thread::JoinHandle<Result<PathBuf, String>>)> = None;
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
        let path = if let Some((prefetched_index, handle)) = prefetched.take() {
            if prefetched_index == index {
                handle.join().map_err(|_| "The natural voice prefetch worker panicked")??
            } else {
                manager.synthesize(&engine, &g2p, &chunk.text, sid, request.rate)?
            }
        } else {
            manager.synthesize(&engine, &g2p, &chunk.text, sid, request.rate)?
        };
        // A superseded or stopped session must not play, or synthesize and
        // cache, even one more chunk.
        if stopped.load(Ordering::SeqCst) { return Ok(()); }
        if index + 1 < chunks.len() && prefetched.is_none() {
            prefetched = spawn_prefetch(manager.clone(), engine.clone(), g2p.clone(),
                chunks[index + 1].text.clone(), sid, request.rate).map(|handle| (index + 1, handle));
        }
        let _ = events.send(activity(true, false, chunk.start_utf16));
        let file = File::open(&path).map_err(|error| format!("Opening synthesized audio failed: {error}"))?;
        let source = Decoder::new(BufReader::new(file)).map_err(|error| format!("Decoding synthesized audio failed: {error}"))?;
        let next = Sink::try_new(&stream)
            .map_err(|error| format!("Starting the audio output failed: {error}"))?;
        next.append(source);
        sink = Some(next);
        let duration = wav_seconds(&path).unwrap_or(0.0);
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

fn prune_cache(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else { return; };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries.filter_map(|entry| {
        let entry = entry.ok()?;
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() { return None; }
        Some((metadata.modified().ok()?, metadata.len(), entry.path()))
    }).collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= CACHE_BUDGET { return; }
    files.sort_by_key(|(modified, ..)| *modified);
    for (_, size, path) in files {
        if total <= CACHE_BUDGET / 2 { break; }
        if fs::remove_file(&path).is_ok() { total = total.saturating_sub(size); }
    }
}

fn refresh_downloaded(root: &Path, state: &mut State) {
    if !state.downloaded && ready(root) { state.downloaded = true; }
}

fn load_performance(root: &Path) -> Option<Performance> {
    let performance: Performance = serde_json::from_str(&fs::read_to_string(root.join(PERFORMANCE_FILE)).ok()?).ok()?;
    (performance.ratio.is_finite() && performance.ratio > 0.0).then_some(performance)
}

fn load_performance_error(root: &Path) -> Option<String> {
    let error: String = serde_json::from_str(&fs::read_to_string(root.join(PERFORMANCE_ERROR_FILE)).ok()?).ok()?;
    (!error.trim().is_empty()).then_some(error)
}

fn save_performance_error(root: &Path, error: &str) -> Result<(), String> {
    let temporary = root.join(format!(".performance-error-{}.json", uuid::Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec(error).map_err(|failure| format!("Saving the natural voice performance failure failed: {failure}"))?)
        .map_err(|failure| format!("Saving the natural voice performance failure failed: {failure}"))?;
    fs::rename(&temporary, root.join(PERFORMANCE_ERROR_FILE))
        .map_err(|failure| format!("Publishing the natural voice performance failure failed: {failure}"))
}

fn save_performance(root: &Path, performance: Performance) -> Result<(), String> {
    let temporary = root.join(format!(".performance-{}.json", uuid::Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec(&performance).map_err(|error| format!("Saving the natural voice performance result failed: {error}"))?)
        .map_err(|error| format!("Saving the natural voice performance result failed: {error}"))?;
    fs::rename(&temporary, root.join(PERFORMANCE_FILE))
        .map_err(|error| format!("Publishing the natural voice performance result failed: {error}"))
}

fn path_string(path: &Path) -> String { path.to_string_lossy().into_owned() }

fn ready(root: &Path) -> bool {
    let dir = root.join(MODEL_DIR);
    dir.join("model.fp16.onnx").is_file()
        && dir.join("voices.bin").is_file()
        && dir.join("tokens.txt").is_file()
        && dir.join("lexicon-us-en.txt").is_file()
        && dir.join("lexicon-gb-en.txt").is_file()
        && dir.join("espeak-ng-data").is_dir()
}

/// Download, verify and install the model. Runs on the async runtime; archive
/// extraction is moved to a blocking worker because it is large and CPU-bound.
pub(crate) async fn download(root: &Path, cancel: Arc<AtomicBool>, progress: impl Fn(u64, u64) + Send + Sync + 'static) -> Result<(), String> {
    let root = root.join("models");
    fs::create_dir_all(&root).map_err(|error| format!("Preparing the voice model directory failed: {error}"))?;
    let archive = root.join(MODEL_ARCHIVE);
    let partial = root.join(format!("{MODEL_ARCHIVE}.part"));
    let _ = fs::remove_file(&partial);
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
    let total = response.content_length().unwrap_or(MODEL_BYTES).max(1);
    let mut file = File::create(&partial).map_err(|error| format!("Creating the voice model download failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut last_report = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|error| format!("Downloading the voice model failed: {error}"))? {
        if cancel.load(Ordering::SeqCst) {
            drop(file); let _ = fs::remove_file(&partial); return Err("The voice model download was cancelled".into());
        }
        if downloaded.saturating_add(chunk.len() as u64) > MODEL_BYTES {
            drop(file); let _ = fs::remove_file(&partial);
            return Err("The voice model download exceeded its pinned size".into());
        }
        if downloaded.is_multiple_of(4 * 1024 * 1024) {
            crate::transfer_policy::require_internet()
                .inspect_err(|_| { let _ = fs::remove_file(&partial); })?;
        }
        file.write_all(&chunk).map_err(|error| format!("Writing the voice model download failed: {error}"))?;
        hasher.update(&chunk);
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded.saturating_sub(last_report) >= 512 * 1024 || downloaded == total {
            progress(downloaded, total);
            last_report = downloaded;
        }
    }
    file.sync_all().map_err(|error| format!("Saving the voice model download failed: {error}"))?;
    drop(file);
    let digest = hex::encode(hasher.finalize());
    if digest != MODEL_SHA256 {
        let _ = fs::remove_file(&partial);
        return Err("The voice model download failed verification and was discarded".into());
    }
    let _ = fs::remove_file(&archive);
    fs::rename(&partial, &archive).map_err(|error| format!("Publishing the voice model download failed: {error}"))?;
    progress(downloaded, total);
    tokio::task::spawn_blocking(move || extract(&root, &archive)).await
        .map_err(|error| format!("Extracting the voice model failed: {error}"))?
}

fn extract(root: &Path, archive: &Path) -> Result<(), String> {
    let temporary = root.join(format!(".extract-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temporary).map_err(|error| format!("Preparing voice model extraction failed: {error}"))?;
    let result = (|| {
        let file = File::open(archive).map_err(|error| format!("Opening the voice model archive failed: {error}"))?;
        let decoder = bzip2::read::BzDecoder::new(file);
        let mut tar = tar::Archive::new(decoder);
        for entry in tar.entries().map_err(|error| format!("Reading the voice model archive failed: {error}"))? {
            let mut entry = entry.map_err(|error| format!("Reading the voice model archive failed: {error}"))?;
            let path = entry.path().map_err(|error| format!("Reading a voice model path failed: {error}"))?;
            if path.is_absolute() || path.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
                return Err("The voice model archive contains an unsafe path".into());
            }
            if entry.header().entry_type().is_symlink() || entry.header().entry_type().is_hard_link() {
                return Err("The voice model archive contains an unsupported link".into());
            }
            entry.unpack_in(&temporary).map_err(|error| format!("Extracting the voice model failed: {error}"))?;
        }
        let source = temporary.join(MODEL_DIR);
        if !ready(&temporary) || !source.is_dir() { return Err("The voice model archive is incomplete".into()); }
        let destination = root.join(MODEL_DIR);
        if destination.exists() { fs::remove_dir_all(&destination).map_err(|error| format!("Replacing the voice model failed: {error}"))?; }
        fs::rename(&source, &destination).map_err(|error| format!("Installing the voice model failed: {error}"))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temporary);
    let _ = fs::remove_file(archive);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
