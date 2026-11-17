// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Debug Android instrumentation only. No Tauri command or web IPC registration.
use crate::{archive::Archive, maps::evaluation};
use jni::{objects::{JClass, JObject, JString}, sys::jstring, JNIEnv};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Seek, SeekFrom, Write}, path::{Component, Path, PathBuf}, time::Instant};

const MAX_JSON: u64 = 32 * 1024 * 1024;
const MAX_EVALUATION_SIZE: u64 = 8 * 1024 * 1024 * 1024;
static PHASE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
fn phase(value: i32) { PHASE.store(value, std::sync::atomic::Ordering::Relaxed); }

#[expect(unsafe_code, reason = "JNI export requires no_mangle")]
#[no_mangle]
pub extern "system" fn Java_org_kiwix_workbench_AndroidEvaluationBridge_phase(
    _env: JNIEnv<'_>, _class: JClass<'_>,
) -> jni::sys::jint { PHASE.load(std::sync::atomic::Ordering::Relaxed) }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest { schema: u32, cases: Vec<Case> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case { id: String, tier: String, recipe: String, local_candidates: Vec<String>, expected: Expected }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected { uuid: String, sha256: String, size: u64, languages: Vec<String>, articles: Vec<Article>, searches: Vec<Search> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Article { path: String, title: String, contains: String }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search { query: String, fulltext: bool, expected_path: String }

fn require(ok: bool, message: &str) -> Result<(), String> { if ok { Ok(()) } else { Err(message.into()) } }
fn identifier(value: &str) -> bool { !value.is_empty() && value.len() <= 80 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') }
fn contained(root: &Path, relative: &str) -> Result<PathBuf, String> {
    require(!relative.is_empty() && relative.len() <= 240 && !relative.contains('\\') && Path::new(relative).components().all(|part| matches!(part, Component::Normal(_))), "Invalid fixture relative path")?;
    let mut current = root.to_path_buf();
    for part in Path::new(relative).components() {
        current.push(part);
        require(!fs::symlink_metadata(&current).map_err(|e| e.to_string())?.file_type().is_symlink(), "Fixture symlink rejected")?;
    }
    require(current.is_file(), "Fixture must be a regular file")?;
    Ok(current)
}
struct OperationDirectory { path: PathBuf, cleaned: bool }
impl OperationDirectory {
    fn cleanup(&mut self) -> Result<(), String> {
        self.cleaned = true;
        fs::remove_dir_all(&self.path).map_err(|e| format!("Evaluation cleanup failed: {e}"))
    }
}
impl Drop for OperationDirectory {
    fn drop(&mut self) {
        if !self.cleaned {
            if let Err(error) = fs::remove_dir_all(&self.path) {
                eprintln!("Evaluation cleanup failed for {}: {error}", self.path.display());
            }
        }
    }
}
fn bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    require(file.metadata().map_err(|e| e.to_string())?.len() <= MAX_JSON, "JSON exceeds bound")?;
    let mut bytes = Vec::new();
    file.take(MAX_JSON + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    require(bytes.len() as u64 <= MAX_JSON, "JSON grew past bound")?;
    Ok(bytes)
}
fn sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop { let len = file.read(&mut buffer).map_err(|e| e.to_string())?; if len == 0 { break; } hash.update(&buffer[..len]); }
    Ok(hex::encode(hash.finalize()))
}
fn cpu_us() -> Option<u128> {
    // Process clock: the import and search workers this bridge measures are
    // native threads, so a thread clock would under-report their work.
    crate::cpu_time::process_cpu_time_micros()
}
fn io_counters() -> Value {
    match fs::read_to_string("/proc/self/io") {
        Ok(text) => { let mut values = serde_json::Map::new(); for line in text.lines() { if let Some((key, value)) = line.split_once(':') { if let Ok(value) = value.trim().parse::<u64>() { values.insert(key.into(), json!(value)); } } } Value::Object(values) },
        Err(_) => Value::Null,
    }
}
fn measure<T>(bytes: u64, operation: impl FnOnce() -> Result<T, String>) -> Result<(T, Value), String> {
    let before = io_counters(); let cpu = cpu_us(); let start = Instant::now();
    let value = operation()?;
    Ok((value, json!({"elapsedUs":start.elapsed().as_micros(),"processCpuUs":cpu_us().zip(cpu).map(|(end,start)| end.saturating_sub(start)),"bytes":bytes,"ioBefore":before,"ioAfter":io_counters()})))
}
fn tree_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?; let metadata = entry.metadata().map_err(|e| e.to_string())?;
        require(!entry.file_type().map_err(|e| e.to_string())?.is_symlink(), "Run symlink rejected")?;
        total = total.checked_add(if metadata.is_dir() { tree_bytes(&entry.path())? } else { metadata.len() }).ok_or("Run size overflow")?;
    }
    Ok(total)
}
fn run(files: &Path, run_id: &str, case_id: &str, mode: &str) -> Result<Value, String> {
    require(identifier(run_id) && identifier(case_id), "Invalid evaluation identifier")?;
    require(matches!(mode, "cold" | "warm" | "collision" | "corruption" | "cancel" | "boundaries"), "Invalid evaluation mode")?;
    let files = fs::canonicalize(files).map_err(|e| e.to_string())?;
    let run = files.join("android-evaluation").join(run_id);
    require(fs::canonicalize(&run).map_err(|e| e.to_string())? == run, "Run root must not traverse symlinks")?;
    let fixtures = run.join("fixtures");
    require(fs::canonicalize(&fixtures).map_err(|e| e.to_string())? == fixtures, "Fixture root must not traverse symlinks")?;
    let manifest_path = contained(&fixtures, "manifest.json")?;
    let manifest_bytes = bounded(&manifest_path)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    require(manifest.schema == 1 && !manifest.cases.is_empty() && manifest.cases.len() <= 12, "Unsupported manifest")?;
    let case = manifest.cases.iter().find(|case| case.id == case_id).ok_or("Unknown evaluation case")?;
    require(case.expected.size > 16 && case.expected.size <= MAX_EVALUATION_SIZE && case.local_candidates.len() <= 16 && !case.local_candidates.is_empty() && case.expected.articles.len() <= 32 && !case.expected.articles.is_empty() && case.expected.searches.len() <= 32 && !case.expected.searches.is_empty() && !case.expected.languages.is_empty(), "Case exceeds bounds or lacks validity assertions")?;
    for article in &case.expected.articles { require(article.path.len() <= 4096 && article.title.len() <= 4096 && !article.contains.is_empty() && article.contains.len() <= 65536, "Article assertion exceeds bound")?; }
    for search in &case.expected.searches { require(!search.query.is_empty() && search.query.len() <= 4096 && search.expected_path.len() <= 4096, "Search assertion exceeds bound")?; }
    let recipe_path = contained(&fixtures, &case.recipe)?;
    let recipe = bounded(&recipe_path)?;
    let candidates = case.local_candidates.iter().map(|name| contained(&fixtures, name)).collect::<Result<Vec<_>, _>>()?;
    // Every invocation gets an exclusive root. Repetition cannot reuse an old
    // published file or accidentally operate on the normal application profile.
    let operation = run.join(format!("{}-{}-{}", case_id, mode, uuid::Uuid::new_v4()));
    fs::create_dir(&operation).map_err(|e| e.to_string())?;
    let mut cleanup = OperationDirectory { path: operation.clone(), cleaned: false };
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    phase(1);
    let prepare_start = Instant::now();
    let prepared = runtime.block_on(evaluation::prepare(&operation, &recipe, candidates, case.expected.size, mode))?;
    let preparation_us = prepare_start.elapsed().as_micros();
    let sentinel = prepared.sentinel.clone();
    phase(2);
    let before = io_counters(); let cpu = cpu_us(); let start = Instant::now();
    let (result, job, phases, chunk_sha256) = runtime.block_on(prepared.compose());
    let composition = json!({"elapsedUs":start.elapsed().as_micros(),"processCpuUs":cpu_us().zip(cpu).map(|(end,start)| end.saturating_sub(start)),"bytes":case.expected.size,"ioBefore":before,"ioAfter":io_counters(),"phasesUs":phases});
    let mut report = json!({"schema":1,"caseId":case.id,"tier":case.tier,"mode":mode,"manifestSha256":hex::encode(Sha256::digest(&manifest_bytes)),"recipeSha256":hex::encode(Sha256::digest(&recipe)),"preparationUs":preparation_us,"composition":composition,"job":job,"operationDirectory":operation.to_string_lossy(),"workloadPolicy":"bounded-map-cache-raw-ranges-v2","cacheBudgetBytes":crate::map_chunk_cache::MAX_BYTES,"cachePolicy":if mode == "warm" {"read-only-staged-raw-ranges; deterministic-prefix-cache-prewarmed-up-to-budget; production-mask-SHA-cache-on-miss-inside-timing; OS-cache-uncontrolled"} else {"read-only-staged-raw-ranges; initially-empty-bounded-cache; production-mask-SHA-cache-on-miss-inside-timing; OS-cache-uncontrolled"}});
    report["composition"]["chunkSha256"] = chunk_sha256;
    report["nativeBuild"] = json!({"debugAssertions":cfg!(debug_assertions),"targetArch":std::env::consts::ARCH,"targetOs":std::env::consts::OS});
    if matches!(mode, "corruption" | "cancel") {
        let error = result.err().ok_or("Negative case unexpectedly published output")?;
        require(if mode == "cancel" { error == "Map composition cancelled" } else { error == "Composed ZIM MD5 mismatch" }, "Negative case failed for an unrelated reason")?;
        let job = job.ok_or("Negative case lost journal")?;
        require(job.written > 0 && job.path.is_none(), "Negative case did not exercise real writing")?;
        require(fs::read_dir(operation.join("output")).map_err(|e| e.to_string())?.next().is_none(), "Negative case left output or partial")?;
        report["assertions"] = json!({"observedError":error,"noPublishedOutput":true,"noPartial":true,"wroteBytes":job.written});
    } else {
        let path = result?;
        let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        require(size == case.expected.size, "Output size mismatch")?;
        phase(3);
        let (hash, hash_metrics) = measure(size, || sha256(&path))?;
        phase(0);
        require(hash == case.expected.sha256, "Final SHA256 mismatch")?;
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::End(-16)).map_err(|e| e.to_string())?;
        let mut footer_md5 = [0u8; 16];
        file.read_exact(&mut footer_md5).map_err(|e| e.to_string())?;
        let archive = Archive::open_files(path.parent().ok_or("Missing parent")?, &[(path.file_name().ok_or("Missing name")?.to_string_lossy().into_owned(), file)])?;
        phase(4);
        let (checksum, checksum_metrics) = measure(size - 16, || archive.evaluation_checksum())?;
        require(checksum, "Retained-byte ZIM checksum failed")?;
        phase(5);
        let (structure, structure_metrics) = measure(size, || archive.evaluation_structure())?;
        require(structure, "libzim structural validation failed")?;
        phase(6);
        let (integrity, integrity_metrics) = measure(size, || archive.check_integrity())?;
        require(integrity, "Production combined integrity check failed")?;
        report["combinedIntegrity"] = integrity_metrics;
        phase(7);
        let metadata = archive.metadata()?;
        require(metadata.id == case.expected.uuid && metadata.size == size, "Archive size does not match the expected file")?;
        let mut actual_languages: Vec<_> = metadata.language.split(';').map(str::to_owned).collect(); actual_languages.sort();
        let mut expected_languages = case.expected.languages.clone(); expected_languages.sort();
        require(actual_languages == expected_languages, "Archive language metadata mismatch")?;
        for article in &case.expected.articles {
            let entry = archive.entry(&article.path)?;
            require(entry.title == article.title && entry.size <= 1024 * 1024, "Article title/size mismatch")?;
            let bytes = archive.read(&article.path, 0, entry.size)?;
            require(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?.contains(&article.contains), "Article text missing")?;
        }
        for search in &case.expected.searches {
            require(!search.fulltext || metadata.has_fulltext_index, &format!("Requested fulltext index missing: query={:?}, fulltext={}, expectedPath={:?}", search.query, search.fulltext, search.expected_path))?;
            let found = archive.search(&search.query, search.fulltext, 0, 100)
                .map_err(|error| format!("Search failed: query={:?}, fulltext={}, expectedPath={:?}, error={error}", search.query, search.fulltext, search.expected_path))?;
            require(found.hits.iter().any(|hit| hit.path == search.expected_path), &format!(
                "Expected article absent from search: query={:?}, fulltext={}, expectedPath={:?}, total={}, hits={}",
                search.query, search.fulltext, search.expected_path, found.total,
                serde_json::to_string(&found.hits).map_err(|e| e.to_string())?
            ))?;
        }
        let mut boundary_assertions = Value::Null;
        if mode == "boundaries" {
            require(archive.read(&case.expected.articles[0].path, 0, crate::archive::MAX_READ_LENGTH + 1).is_err(), "Oversized article read accepted")?;
            require(archive.entry("A/__evaluation_missing__").is_err(), "Missing article unexpectedly exists")?;
            require(contained(&fixtures, "../manifest.json").is_err() && contained(&fixtures, "/manifest.json").is_err(), "Path escape accepted")?;
            boundary_assertions = json!({"oversizedReadRejected":true,"missingArticleRejected":true,"relativeTraversalRejected":true,"absolutePathRejected":true});
        }
        if let Some(sentinel) = sentinel {
            require(fs::read(&sentinel).map_err(|e| e.to_string())? == b"evaluation-owned-existing-output" && sentinel != path, "Collision replaced existing output")?;
            report["collisionPreserved"] = json!(true);
        }
        drop(archive);
        // Deliberately change a retained output byte, then exercise exactly the
        // production retained-byte checksum rather than an independent fork.
        let tampered_rejected = if mode == "boundaries" {
            let mut file = fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Start(size - 1)).map_err(|e| e.to_string())?;
            let mut byte = [0]; file.read_exact(&mut byte).map_err(|e| e.to_string())?; byte[0] ^= 1;
            file.seek(SeekFrom::Start(size - 1)).map_err(|e| e.to_string())?; file.write_all(&byte).map_err(|e| e.to_string())?; file.sync_all().map_err(|e| e.to_string())?;
            let archive = Archive::open_files(path.parent().unwrap(), &[(path.file_name().unwrap().to_string_lossy().into_owned(), file)])?;
            require(!archive.evaluation_checksum()?, "Tampered retained checksum accepted")?; true
        } else { false };
        report["validity"] = json!({"sha256":hash,"size":size,"metadata":metadata,"retainedChecksum":checksum,"structural":structure,"articlesAsserted":case.expected.articles.len(),"searchesAsserted":case.expected.searches.len(),"boundaries":boundary_assertions,"tamperedChecksumRejected":tampered_rejected});
        report["validity"]["zimFooterMd5"] = json!(hex::encode(footer_md5));
        report["identitySha256"] = hash_metrics; report["retainedChecksum"] = checksum_metrics; report["structuralValidation"] = structure_metrics;
    }
    report["runLogicalBytesBeforeCleanup"] = json!(tree_bytes(&run)?);
    cleanup.cleanup()?;
    report["ok"] = json!(true);
    Ok(report)
}

fn string(env: &mut JNIEnv<'_>, value: &JString<'_>, max: usize) -> Result<String, String> {
    let length = env.call_method(value, "length", "()I", &[]).and_then(|value| value.i()).map_err(|e| e.to_string())?;
    require(length >= 0 && length as usize <= max, "JNI string exceeds bound")?;
    env.get_string(value).map(|value| value.into()).map_err(|e| e.to_string())
}

#[expect(unsafe_code, reason = "JNI export requires no_mangle")]
#[no_mangle]
pub extern "system" fn Java_org_kiwix_workbench_AndroidEvaluationBridge_evaluate(
    mut env: JNIEnv<'_>, _class: JClass<'_>, context: JObject<'_>, run_id: JString<'_>, case_id: JString<'_>, mode: JString<'_>,
) -> jstring {
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value, String> {
        let package = env.call_method(&context, "getPackageName", "()Ljava/lang/String;", &[]).and_then(|value| value.l()).map_err(|e| e.to_string())?;
        require(string(&mut env, &JString::from(package), 128)? == "org.kiwix.workbench", "Unexpected target package")?;
        let file = env.call_method(&context, "getFilesDir", "()Ljava/io/File;", &[]).and_then(|value| value.l()).map_err(|e| e.to_string())?;
        let path = env.call_method(file, "getAbsolutePath", "()Ljava/lang/String;", &[]).and_then(|value| value.l()).map_err(|e| e.to_string())?;
        let path = string(&mut env, &JString::from(path), 4096)?;
        let run_id = string(&mut env, &run_id, 80)?; let case_id = string(&mut env, &case_id, 80)?; let mode = string(&mut env, &mode, 32)?;
        run(Path::new(&path), &run_id, &case_id, &mode)
    }));
    let value = match response { Ok(Ok(value)) => value, Ok(Err(error)) => json!({"schema":1,"ok":false,"error":error}), Err(_) => json!({"schema":1,"ok":false,"error":"Native evaluation panicked"}) };
    phase(0);
    if env.exception_check().unwrap_or(false) { let _ = env.exception_clear(); }
    match env.new_string(value.to_string()) { Ok(value) => value.into_raw(), Err(_) => std::ptr::null_mut() }
}
