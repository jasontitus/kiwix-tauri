// SPDX-License-Identifier: GPL-3.0-or-later
// Unsafe operations stay wrapped in their own `unsafe {}` block with a SAFETY
// comment, including inside `unsafe fn` bodies, so each precondition is
// documented at the operation it guards rather than at the function header.
#![deny(unsafe_op_in_unsafe_fn)]
mod acquisition;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_requests;
mod archive;
mod archive_metadata;
mod archive_search;
mod archive_source;
mod casefold;
mod catalog;
mod edition_updates;
mod crash;
// CPU-clock readings for diagnostics: Unix via rustix, Windows via
// `GetThreadTimes`. No other target reads CPU time.
#[cfg(any(unix, windows))]
mod cpu_time;
// The build script owns the SDK validation; compiling it here runs its unit
// tests (root resolution) under `cargo test --lib`.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../native/desktop_sdk.rs"]
mod desktop_sdk;
#[cfg(desktop)]
mod desktop_workflow;
#[cfg(any(mobile, test))]
mod device_scan;
mod diagnostics;
mod disk_warning;
mod exports;
#[cfg(target_os = "macos")]
mod apple_window;
#[cfg(any(target_os = "ios", target_os = "macos", target_os = "android"))]
mod ffi;
mod fs_ext;
#[cfg(any(target_os = "android", test))]
mod hotspot;
mod imports;
#[cfg(desktop)]
mod instance;
mod integrity;
#[cfg(any(desktop, test))]
mod launch_search;
mod library_files;
#[cfg(any(mobile, test))]
mod library_root;
mod localization;
mod localswarm;
mod map_chunk_cache;
mod maps;
#[cfg(all(target_os = "android", native_location))]
mod location_android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod location_apple;
#[cfg(all(target_os = "linux", native_location))]
mod location_linux;
#[cfg(target_os = "windows")]
mod location_windows;
#[cfg(desktop)]
mod menus;
mod migration;
mod native_pdf;
mod nearby_log;
mod nearby_public_link;
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod notifications;
mod partial_donor;
mod permissions;
mod publication;
#[cfg(desktop)]
mod portable;
mod power_activity;
mod profile_paths;
mod private_sync;
mod private_sync_identity;
mod reading_data;
mod provider_reference;
#[cfg(desktop)]
mod quit;
#[cfg(desktop)]
mod os_open;
mod sandbox;
mod search;
mod server;
mod service;
mod shared_library;
mod publisher;
mod sharing;
mod spelling;
mod storage;
mod store;
mod timestamps;
mod torrent;
mod transfer_policy;
mod transfer_response;
#[cfg(desktop)]
mod update;
mod update_admission;
mod user_error;
mod mobile_backup;
#[cfg(desktop)]
mod watch;
#[cfg(desktop)]
mod windows;
mod zim_scheme;
#[cfg(mobile)]
mod mobile;
#[cfg(target_os = "android")]
mod android_tls;
#[cfg(any(mobile, test))]
mod mobile_rebase;
#[cfg(any(target_os = "macos", target_os = "ios", test))]
mod apple_replies;
#[cfg(any(target_os = "ios", test))]
mod ios_background;
#[cfg(all(feature = "android-evaluation", not(all(target_os = "android", debug_assertions))))]
compile_error!("android-evaluation requires a debug Android build");
#[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
mod android_evaluation;

use serde_json::{json, Value};
use service::Service;
use std::sync::Arc;
#[cfg(desktop)]
use std::{
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(desktop)]
use tauri::{Emitter, Manager};
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;

#[cfg(desktop)]
#[derive(Default)]
struct Options {
    data_dir: Option<PathBuf>,
    headless: bool,
    private: bool,
    read_only: bool,
    port: u16,
    article: Option<String>,
    paths: Vec<String>,
    /// "Look up in Kiwix": an already sanitised query to prefill library search.
    search: Option<String>,
}
#[cfg(desktop)]
fn desktop_path(argument: String) -> String {
    if let Ok(url) = url::Url::parse(&argument) {
        if url.scheme() == "file" {
            if let Ok(path) = url.to_file_path() {
                return path.to_string_lossy().into_owned();
            }
        }
    }
    argument
}

#[cfg(desktop)]
fn parse_options(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut result = Options::default();
    let args: Vec<String> = args.skip(1).collect();
    // Linux desktop entries pass the opened URL directly (`Exec=… %U`). Accept
    // one bare URL so a file link still opens and a kiwix:// link only prefills
    // a search; anything else keeps the normal option/path parser below.
    if args.len() == 1 {
        if let Ok(url) = url::Url::parse(&args[0]) {
            match url.scheme() {
                "kiwix" => {
                    result.search = Some(launch_search::parse_url(&args[0])?);
                    return Ok(result);
                }
                "file" => {
                    if let Ok(path) = url.to_file_path() {
                        result.paths.push(path.to_string_lossy().into_owned());
                        return Ok(result);
                    }
                }
                _ => {}
            }
        }
    }
    // The registered kiwix:// handler is `"<exe>" --open-url "%1"`. Accept it
    // only as the whole command line, so a crafted link can never smuggle in a
    // second flag (a profile, a mode or a path) beside it.
    if args.first().map(String::as_str) == Some("--open-url") {
        if args.len() != 2 {
            return Err("--open-url takes exactly one kiwix:// link and no other arguments".into());
        }
        result.search = Some(launch_search::parse_url(&args[1])?);
        return Ok(result);
    }
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open-url" => return Err("--open-url must be the only argument".into()),
            "--search" => {
                let text = args.next().ok_or("--search requires text to look up")?;
                result.search = Some(
                    launch_search::sanitize(&text).ok_or("--search has no searchable text")?,
                );
            }
            "--headless" => result.headless = true,
            "--private" => result.private = true,
            "--read-only" => result.read_only = true,
            "--data-dir" => {
                result.data_dir = Some(PathBuf::from(
                    args.next().ok_or("--data-dir requires a path")?,
                ))
            }
            "--port" => {
                result.port = args
                    .next()
                    .ok_or("--port requires a number")?
                    .parse()
                    .map_err(|_| "Invalid port")?
            }
            "--article" => {
                result.article = Some(
                    args.next()
                        .ok_or("--article requires an archive article path")?,
                )
            }
            "--help" | "-h" => {
                print_cli_answer("Kiwix — offline knowledge reader\n\nUsage: kiwix-workbench [OPTIONS] [ARCHIVE.zim ...]\n  --article PATH     Open a path, query, and fragment in the specified archive\n  --search TEXT      Look up TEXT in the library (also kiwix://search?q=TEXT)\n  --data-dir PATH    Isolated application state directory\n  --private          Do not persist browsing/session/archive storage changes\n  --read-only        Existing profile only; prohibit filesystem mutation\n  --headless         Serve the real app and API on authenticated loopback\n  --port NUMBER      Headless port (default: an available port)\n  --version          Display version");
                std::process::exit(0);
            }
            "--version" => {
                print_cli_answer(&format!("Kiwix Rust {}", env!("CARGO_PKG_VERSION")));
                std::process::exit(0);
            }
            "--" => {
                result.paths.extend(args.map(desktop_path));
                break;
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option: {arg}")),
            _ => result.paths.push(desktop_path(arg)),
        }
    }
    if result.article.is_some() && result.paths.len() != 1 {
        return Err("--article requires exactly one archive path".into());
    }
    if result.search.is_some() && (!result.paths.is_empty() || result.headless) {
        return Err("--search cannot be combined with archive paths or --headless".into());
    }
    Ok(result)
}

/// Answer `--help` or `--version` on stdout. A Windows release build has no
/// console (`windows_subsystem = "windows"`), so unless stdout was redirected
/// its handle is null and the answer would vanish: show it in a message box
/// there instead.
#[cfg(desktop)]
fn print_cli_answer(text: &str) {
    println!("{text}");
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        if std::io::stdout().as_raw_handle().is_null() {
            show_native_message("Kiwix", text, false);
        }
    }
}

/// A forwarded launch must never run the first-launch help/version exit path:
/// this process is already the running instance and owns its profile/session.
#[cfg(desktop)]
fn parse_forwarded_options(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let args: Vec<String> = args.collect();
    if args.iter().any(|arg| matches!(arg.as_str(), "--help" | "-h" | "--version")) {
        return Err("--help and --version are only valid for the first launch".into());
    }
    parse_options(args.into_iter())
}

#[tauri::command]
async fn dispatch(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<Service>>,
    command: String,
    mut args: Value,
    android_window_incarnation: Option<String>,
    android_activity_generation: Option<String>,
) -> Result<Value, String> {
    if window.label().starts_with("pdf-render-") {
        return Err("Close this window to continue".into());
    }
    // The app-owned camera window may only hand back its photo or cancel.
    #[cfg(desktop)]
    if window.label() == crate::service::photo_capture::CAMERA_LABEL
        && !matches!(command.as_str(), "camera_capture_complete" | "camera_capture_cancel")
    {
        return Err("Not available".into());
    }
    let object = args
        .as_object_mut()
        .ok_or("This action could not be started. Try again")?;
    object.insert("__windowLabel".into(), json!(window.label()));
    #[cfg(target_os = "android")]
    let result = state.inner().clone().android_dispatch(
        &command, args, android_window_incarnation, android_activity_generation,
    ).await;
    #[cfg(not(target_os = "android"))]
    let result = {
        let _ = (android_window_incarnation, android_activity_generation);
        state.inner().clone().dispatch(&command, args).await
    };
    // One boundary for every command: native machine codes and background-task
    // failures become a sentence a person can act on.
    result.map_err(|error| crate::user_error::plain(&error))
}

#[cfg(desktop)]
fn data_dir(options: &Options) -> Result<PathBuf, String> {
    if let Some(profile) = portable::PortableProfile::detect()? {
        if let Some(requested) = &options.data_dir {
            if !profile.matches(requested)? {
                return Err("Portable mode requires adjacent data/; conflicting --data-dir is not permitted (the matching data path is allowed)".into());
            }
        }
        return Ok(profile.data);
    }
    options
        .data_dir
        .clone()
        .or_else(|| {
            directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench")
                .map(|dirs| dirs.data_dir().to_path_buf())
        })
        .ok_or("Cannot locate application data directory; provide --data-dir".into())
}

#[cfg(desktop)]
fn profile_identity(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    match std::fs::canonicalize(&absolute) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut normalized = PathBuf::new();
            for component in absolute.components() {
                match component {
                    Component::CurDir => {}
                    Component::ParentDir => {
                        normalized.pop();
                    }
                    component => normalized.push(component.as_os_str()),
                }
            }
            Ok(normalized)
        }
        Err(error) => Err(format!(
            "Cannot identify profile {}: {error}",
            absolute.display()
        )),
    }
}

#[cfg(desktop)]
fn validate_secondary_launch(
    service: &Service,
    options: &Options,
    cwd: &Path,
) -> Result<(), String> {
    let settings = service.store.settings()?;
    let incompatible = if options.private && !settings.private_mode {
        Some("--private requires a private-mode instance")
    } else if options.read_only && !settings.read_only {
        Some("--read-only requires a read-only instance")
    } else if let Some(requested) = &options.data_dir {
        let existing_cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        if profile_identity(requested, cwd)?
            != profile_identity(service.store.data_dir(), &existing_cwd)?
        {
            Some("--data-dir selects a different profile from the running instance")
        } else {
            None
        }
    } else {
        None
    };
    if let Some(reason) = incompatible {
        return Err(format!(
            "{reason}. No archives were opened or imported. Quit the running Kiwix instance, then repeat this command to use the requested profile and mode."
        ));
    }
    Ok(())
}

/// Returns what `open_paths` returns: the archives of this launch that could
/// not be opened, after the readable ones were.
#[cfg(desktop)]
async fn open_secondary_paths(
    service: Arc<Service>,
    mut options: Options,
    cwd: &Path,
) -> Result<Vec<String>, String> {
    let generation = service.launch_generation();
    validate_secondary_launch(&service, &options, cwd)?;
    if let Some(query) = options.search.take() {
        return open_search(service, query, true, Some(generation))
            .await
            .map(|()| Vec::new());
    }
    for path in &mut options.paths {
        if Path::new(path).is_relative() {
            *path = cwd.join(&*path).to_string_lossy().into_owned();
        }
    }
    open_paths(
        service,
        options.paths,
        options.article,
        true,
        Some(generation),
    )
    .await
}

#[cfg(desktop)]
fn spawn_secondary_paths(
    app: &tauri::AppHandle,
    service: Arc<Service>,
    options: Options,
    cwd: PathBuf,
) {
    let app = app.clone();
    let title = if options.search.is_some() { "Look up in Kiwix" } else { "Import archives" };
    tauri::async_runtime::spawn(async move {
        let outcome = open_secondary_paths(service.clone(), options, &cwd).await;
        report_launch_outcome(&app, &service, title, outcome);
    });
}

/// What to tell the person after a launch asked a running Kiwix to open
/// something: the whole launch was refused, or some of its archives could not
/// be opened (one line each) after the readable ones were. `None` when there is
/// nothing to report.
#[cfg(desktop)]
fn launch_failure_detail(outcome: Result<Vec<String>, String>) -> Option<String> {
    match outcome {
        Ok(failures) if failures.is_empty() => None,
        Ok(failures) => Some(failures.join("\n")),
        Err(error) => Some(error),
    }
}

/// Show a failed or partly failed launch in a native dialog. A second launch,
/// "Open with" and a double-clicked archive have no other place to report to:
/// stderr is not visible, and the window that came forward shows nothing new.
#[cfg(desktop)]
fn report_launch_outcome(
    app: &tauri::AppHandle,
    service: &Service,
    title: &str,
    outcome: Result<Vec<String>, String>,
) {
    let Some(detail) = launch_failure_detail(outcome) else {
        return;
    };
    eprintln!("{title}: {detail}");
    let locale = service.store.locale().unwrap_or_else(|_| "system".into());
    let strings = localization::Localizer::for_app(&locale);
    app.dialog()
        .message(format!(
            "{}\n\n{detail}",
            strings.text("The operation could not be completed.", &[])
        ))
        .title(strings.text(title, &[]))
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

/// One line for each archive of a launch that could not be opened: its file
/// name and the reason the import gave.
#[cfg(desktop)]
fn launch_failures(errors: &Value) -> Vec<String> {
    errors
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let reason = entry
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("The archive could not be opened");
            match entry
                .get("path")
                .and_then(Value::as_str)
                .and_then(|path| Path::new(path).file_name())
            {
                Some(name) => format!("{}: {reason}", name.to_string_lossy()),
                None => reason.to_owned(),
            }
        })
        .collect()
}

/// Startup has no window to hang a dialog on yet: keep each unopened archive
/// in the shell's startup notice instead. Returns whether there was anything
/// to report.
#[cfg(desktop)]
fn note_startup_launch_outcome(service: &Service, outcome: Result<Vec<String>, String>) -> bool {
    let failures = match outcome {
        Ok(failures) => failures,
        Err(error) => vec![error],
    };
    for failure in &failures {
        eprintln!("Opening startup archives: {failure}");
        service.note_startup_warning(format!("Could not open startup archive: {failure}"));
    }
    !failures.is_empty()
}

#[cfg(desktop)]
fn desktop_launch_target(service: &Service) -> Result<(String, tauri::WebviewWindow), String> {
    let app = service
        .app
        .lock()
        .map_err(|_| "Application handle unavailable")?
        .clone()
        .ok_or("No native application is available to open the archive")?;
    if quit::is_preparing(&app) {
        return Err("Cannot open archives while application Quit is pending".into());
    }
    let mut windows: Vec<_> = app.webview_windows().into_iter().collect();
    windows.sort_by(|a, b| a.0.cmp(&b.0));
    let candidates: Vec<_> = windows
        .iter()
        .map(|(label, window)| (label.as_str(), window.is_focused().unwrap_or(false)))
        .collect();
    let index = launch_target_index(&candidates).ok_or("No native window is available to open the archive")?;
    let (label, window) = windows.remove(index);
    Ok((label, window))
}

#[cfg(desktop)]
/// Which window a launch opens archives in and raises: the focused reader
/// window, else `main`, else the first reader window by label. Only reader
/// shell windows (`main`, `window-*`) qualify. Internal windows, such as the
/// hidden `pdf-render-*` WebView on Windows, must never be shown.
fn launch_target_index(windows: &[(&str, bool)]) -> Option<usize> {
    let shell = |label: &str| quit::shell_label(label);
    windows
        .iter()
        .position(|(label, focused)| *focused && shell(label))
        .or_else(|| windows.iter().position(|(label, _)| *label == "main"))
        .or_else(|| windows.iter().position(|(label, _)| shell(label)))
}

/// Bring the instance that owns the profile to the front. Used by the Unix
/// profile rendezvous (instance.rs) and the Windows single-instance plugin
/// whenever a second launch reaches the running instance.
#[cfg(desktop)]
fn activate_instance(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(state) = app.try_state::<Arc<Service>>() {
        if let Ok((_, window)) = desktop_launch_target(&state) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

/// Forward a launch received over the profile rendezvous. Parsing and profile
/// validation stay in the same path a normal secondary launch uses.
#[cfg(all(desktop, unix))]
fn forward_launch(app: &tauri::AppHandle, args: Vec<String>, cwd: PathBuf) {
    match parse_forwarded_options(args.into_iter()) {
        Ok(options) => {
            if let Some(state) = app.try_state::<Arc<Service>>() {
                spawn_secondary_paths(app, state.inner().clone(), options, cwd);
            } else {
                eprintln!("Opening additional archives: the running instance is still starting; retry once its window appears.");
            }
        }
        Err(error) => eprintln!("Command-line arguments: {error}"),
    }
}

/// A startup failure that is really "another instance owns this profile". The
/// config window already exists by the time setup runs, so close it, explain
/// the conflict, and exit 0 without touching the owner's profile.
///
/// Unix normally decides this before the builder, but the same handler stays
/// available there so a store that reports a busy profile can never reach
/// Tauri as a setup error (which aborts the process).
#[cfg(desktop)]
fn profile_busy_in_setup(
    app: &mut tauri::App,
    path: &Path,
    error: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for (_, window) in app.webview_windows() {
        let _ = window.close();
    }
    let message = instance::busy_dialog_message(path, error);
    app.handle()
        .dialog()
        .message(message)
        .title("Kiwix — data directory in use")
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .show(|_| {
            // The library service was never constructed, so there is nothing
            // to retire; exiting here also bypasses the quit handshake, which
            // cannot run without a service.
            std::process::exit(0);
        });
    eprintln!("Kiwix: {error}");
    Ok(())
}

/// Title of the dialog that explains why Kiwix did not start. Like the
/// busy-profile dialog it is English: no profile, and so no saved language, has
/// been read when it is needed.
#[cfg(desktop)]
const STARTUP_FAILURE_TITLE: &str = "Kiwix — cannot start";

/// How a fatal startup error reaches the person who launched Kiwix.
#[cfg(desktop)]
#[derive(Debug, PartialEq, Eq)]
struct StartupFailure {
    /// Text for a native dialog. `None` for a `--headless` launch, whose
    /// operator reads stderr and must never be left waiting on a dialog.
    dialog: Option<String>,
    /// The process exit status, unchanged by how the error is shown.
    exit_code: i32,
}

/// A windowed launch from Finder, the Start menu or a desktop launcher has no
/// visible stderr (and a Windows release build has no console at all), so the
/// error also goes to a dialog. The store's own recovery text is kept whole.
#[cfg(desktop)]
fn startup_failure(error: &str, headless: bool) -> StartupFailure {
    StartupFailure {
        dialog: (!headless).then(|| format!("Kiwix could not start.\n\n{error}")),
        exit_code: 1,
    }
}

/// Show one message and wait for it to be dismissed, before any Tauri runtime
/// exists. Returns whether a dialog was shown.
#[cfg(all(desktop, unix))]
fn show_native_message(title: &str, message: &str, error: bool) -> bool {
    let kind = if error {
        instance::NativeMessageKind::Error
    } else {
        instance::NativeMessageKind::Warning
    };
    instance::show_native_message(kind, title, message)
}

/// Show one message and wait for it to be dismissed, before any Tauri runtime
/// exists. Returns whether a dialog was shown.
#[cfg(windows)]
fn show_native_message(title: &str, message: &str, error: bool) -> bool {
    // `windows` alone names this crate's window module here.
    use ::windows::{
        core::HSTRING,
        Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
        },
    };
    let title = HSTRING::from(title);
    let message = HSTRING::from(message);
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    // SAFETY: `message` and `title` are owned, NUL-terminated UTF-16 strings
    // that live until the call returns, and the call keeps neither pointer.
    // No owner window is passed, which the API permits.
    let pressed = unsafe { MessageBoxW(None, &message, &title, MB_OK | icon | MB_SETFOREGROUND) };
    pressed.0 != 0
}

/// Report a fatal error from before the Tauri runtime exists, then exit. The
/// stderr line stays for terminals, logs and `--headless`.
#[cfg(desktop)]
fn exit_after_startup_failure(error: &str, headless: bool) -> ! {
    eprintln!("Kiwix: {error}");
    let failure = startup_failure(error, headless);
    if let Some(message) = &failure.dialog {
        show_native_message(STARTUP_FAILURE_TITLE, message, true);
    }
    std::process::exit(failure.exit_code);
}

/// A fatal error inside Tauri's setup hook. Returning it would make Tauri
/// panic ("Failed to setup app"), which closes the app with no explanation, so
/// it is shown here and the process exits with the same status once the dialog
/// is dismissed. Same pattern as `profile_busy_in_setup`.
#[cfg(desktop)]
fn startup_failed_in_setup(app: &mut tauri::App, error: &str) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Kiwix: {error}");
    let failure = startup_failure(error, false);
    let exit_code = failure.exit_code;
    let Some(message) = failure.dialog else {
        std::process::exit(exit_code);
    };
    // A window that was already built has nothing to show. Hide it instead of
    // closing it: closing the last window asks for an ordinary Quit, which
    // would end the process with status 0 before the message is read.
    for (_, window) in app.webview_windows() {
        let _ = window.hide();
    }
    app.handle()
        .dialog()
        .message(message)
        .title(STARTUP_FAILURE_TITLE)
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(move |_| {
            // Startup did not finish, so there is no running session to save
            // and no Quit handshake to run.
            std::process::exit(exit_code);
        });
    Ok(())
}

/// Wrap the setup hook so that no error from it reaches Tauri as `Err`.
#[cfg(desktop)]
fn setup_reporting_failure<F>(
    setup: F,
) -> impl FnOnce(&mut tauri::App) -> Result<(), Box<dyn std::error::Error>> + Send + 'static
where
    F: FnOnce(&mut tauri::App) -> Result<(), Box<dyn std::error::Error>> + Send + 'static,
{
    move |app| match setup(app) {
        Ok(()) => Ok(()),
        Err(error) => startup_failed_in_setup(app, &error.to_string()),
    }
}

/// The exit was authorized: relaunch if an installed update asked for it,
/// otherwise exit with the requested status.
#[cfg(desktop)]
fn finish_authorized_quit(app: &tauri::AppHandle, code: Option<i32>) {
    if update::take_restart_after_quit() {
        // An installed update relaunches only now that the service is retired.
        app.request_restart();
        return;
    }
    app.exit(code.unwrap_or(0));
}

/// "Quit Without Saving": attempt every stage of `Service::shutdown` once
/// more, so transfers still stop at a checkpoint and listeners, locks and
/// native resources are still released, then let the exit proceed whatever it
/// reported. Only what could not be written is given up.
#[cfg(desktop)]
async fn retire_without_saving(service: &Service) {
    if let Err(error) = service.shutdown().await {
        eprintln!("Quitting without saving; not everything could be saved or stopped cleanly: {error}");
    }
}

/// The exit handler's state, shared with the failed-Quit dialog.
#[cfg(desktop)]
#[derive(Clone, Default)]
struct QuitFlags {
    /// A Quit attempt is running or its failed-Quit dialog is open. Further
    /// exit requests are held back.
    exiting: Arc<AtomicBool>,
    /// The process may exit: the service retired, or the person chose to quit
    /// without saving.
    authorized: Arc<AtomicBool>,
    /// An exit request was held back while `exiting` was set. The dialog reads
    /// it when it closes, so a Quit chosen meanwhile is not lost.
    requested_again: Arc<AtomicBool>,
    /// How many failed-Quit dialogs in a row could not be shown.
    unshown_dialogs: Arc<std::sync::atomic::AtomicU32>,
}

/// The application window the failed-Quit dialog belongs to: the focused
/// one, else `main`, else the first by label.
#[cfg(desktop)]
fn failed_quit_dialog_parent(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    let mut windows: Vec<_> = app.webview_windows().into_iter().collect();
    windows.sort_by(|a, b| a.0.cmp(&b.0));
    let candidates: Vec<_> = windows
        .iter()
        .map(|(label, window)| (label.as_str(), window.is_focused().unwrap_or(false)))
        .collect();
    let index = launch_target_index(&candidates)?;
    Some(windows.remove(index).1)
}

/// Quit could not save or retire cleanly. Ask the person whether to retry,
/// keep Kiwix open, or quit knowing what is lost. `exiting` stays set until
/// they answer, so a second Quit cannot stack another dialog, and nothing but
/// the explicit "Quit Without Saving" button authorizes an exit that gives up
/// state (`quit::failed_quit_action`). Cancel keeps Kiwix open only while a
/// window is left to return to (`quit::settle_failed_quit`).
#[cfg(desktop)]
fn ask_after_failed_quit(
    app: tauri::AppHandle,
    service: Arc<Service>,
    failure: quit::QuitFailure,
    error: String,
    code: Option<i32>,
    flags: QuitFlags,
) {
    let locale = service.store.locale().unwrap_or_else(|_| "system".into());
    let strings = localization::Localizer::for_app(&locale);
    let labels = quit::FailedQuitLabels {
        retry: strings.text("Retry", &[]),
        quit_without_saving: strings.text("Quit Without Saving", &[]),
        cancel: strings.text("Cancel", &[]),
    };
    let mut dialog = app
        .dialog()
        .message(quit::failed_quit_message(&strings, failure, &error))
        .title(strings.text("Quit", &[]))
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::YesNoCancelCustom(
            labels.retry.clone(),
            labels.quit_without_saving.clone(),
            labels.cancel.clone(),
        ));
    if let Some(window) = failed_quit_dialog_parent(&app) {
        // Attached to a window, the question cannot sit unseen behind it.
        let _ = window.unminimize();
        dialog = dialog.parent(&window);
    }
    // Only requests that arrive from here on are "while the dialog is open".
    flags.requested_again.store(false, Ordering::SeqCst);
    let asked_at = std::time::Instant::now();
    dialog.show_with_result(move |result| {
        let answer = quit::failed_quit_action(failure, &result, &labels);
        // A dialog the platform could not display comes back at once as
        // Cancel. Count those, so that retrying cannot spin.
        let unshown = if quit::failed_quit_dialog_was_shown(&answer, asked_at.elapsed()) {
            flags.unshown_dialogs.store(0, Ordering::SeqCst);
            0
        } else {
            flags.unshown_dialogs.fetch_add(1, Ordering::SeqCst).saturating_add(1)
        };
        let shell_window_remains = app
            .webview_windows()
            .keys()
            .any(|label| quit::shell_label(label));
        let action = quit::settle_failed_quit(
            answer,
            shell_window_remains,
            flags.requested_again.swap(false, Ordering::SeqCst),
        );
        match action {
            quit::FailedQuitAction::Retry => {
                let delay = quit::unshown_dialog_retry_delay(unshown);
                if delay.is_zero() {
                    flags.exiting.store(false, Ordering::SeqCst);
                    app.exit(code.unwrap_or(0));
                } else {
                    // No window and no dialog: keep trying, slowly, and never
                    // give up unsaved state without being told to.
                    eprintln!(
                        "Application Quit: its question could not be shown ({unshown} in a row). Nothing was discarded; trying Quit again in {} s. Cause: {error}",
                        delay.as_secs()
                    );
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(delay).await;
                        flags.exiting.store(false, Ordering::SeqCst);
                        app.exit(code.unwrap_or(0));
                    });
                }
            }
            quit::FailedQuitAction::QuitWithoutSaving => {
                // `exiting` stays set: this attempt ends in the exit below.
                tauri::async_runtime::spawn(async move {
                    retire_without_saving(&service).await;
                    flags.authorized.store(true, Ordering::SeqCst);
                    finish_authorized_quit(&app, code);
                });
            }
            quit::FailedQuitAction::Stay { clear_relaunch } => {
                if unshown > 0 {
                    eprintln!("Application Quit: its question could not be shown; Kiwix stays open. Cause: {error}");
                }
                if clear_relaunch {
                    // The shells resumed; a later plain Quit must not relaunch.
                    update::clear_restart_after_quit();
                }
                flags.exiting.store(false, Ordering::SeqCst);
                // A Quit held back in the instant before that store is still wanted.
                if flags.requested_again.swap(false, Ordering::SeqCst) {
                    app.exit(code.unwrap_or(0));
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
async fn open_zim_links(service: Arc<Service>, links: Vec<String>) -> Result<(), String> {
    if links.is_empty() {
        return Ok(());
    }
    let (label, window) = desktop_launch_target(&service)?;
    let generation = service.launch_generation();
    service
        .queue_zim_links(&label, links, generation)
        .await?;
    // Queue before waking the shell: successful event emission is not delivery.
    window.emit("kiwix-open", ()).map_err(|e| e.to_string())?;
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// Ask, in a folder panel, for each folder of `paths` that the macOS App
/// Sandbox does not let Kiwix open yet. Does nothing outside the sandbox.
/// Prompts from separate Open With events are shown one after another.
#[cfg(target_os = "macos")]
async fn request_folder_access(app: &tauri::AppHandle, paths: &[String]) {
    static PROMPTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serialized = PROMPTS.lock().await;
    for folder in sandbox::folders_needing_access(paths.iter().map(PathBuf::from)) {
        let app = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_directory(&folder)
                .set_title("Allow Kiwix to read the folder that contains the archive")
                .blocking_pick_folder()
        })
        .await;
    }
}

/// "Look up in Kiwix": prefill library search in the launch target window.
/// Startup has no native window yet and belongs to the default main shell.
#[cfg(desktop)]
async fn open_search(
    service: Arc<Service>,
    query: String,
    emit: bool,
    expected_generation: Option<u64>,
) -> Result<(), String> {
    let generation = expected_generation.unwrap_or_else(|| service.launch_generation());
    if !emit {
        return service.queue_launch_search("main", query, generation).await;
    }
    let (label, window) = desktop_launch_target(&service)?;
    // Queue before waking the shell: successful event emission is not delivery.
    service.queue_launch_search(&label, query, generation).await?;
    window.emit("kiwix-open", ()).map_err(|e| e.to_string())?;
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// Open the given archives in the launch target window. `Ok` lists the archives
/// that could not be opened (see `launch_failures`) after every readable one
/// was queued for the window. `Err` means the launch did not reach a window;
/// archives that were readable may already be in the library, and the error
/// text then also names the ones that were not. The caller must show either
/// result to the person, because nothing else does.
#[cfg(desktop)]
async fn open_paths(
    service: Arc<Service>,
    paths: Vec<String>,
    article: Option<String>,
    emit: bool,
    expected_generation: Option<u64>,
) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(app) = service
        .app
        .lock()
        .map_err(|_| "Application handle unavailable")?
        .as_ref()
    {
        if quit::is_preparing(app) {
            return Err("Cannot open archives while application Quit is pending".into());
        }
    }
    let launch_generation = expected_generation.unwrap_or_else(|| service.launch_generation());
    let read_only = service.store.read_only()?;
    let imported = if read_only {
        let state = service.store.read()?;
        let mut books = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let canonical = match std::fs::canonicalize(&path) {
                Ok(canonical) => canonical,
                Err(error) => {
                    // One unreadable path must not abort the rest of the batch,
                    // matching the writable import branch.
                    errors.push(json!({"path":path,"error":error.to_string()}));
                    continue;
                }
            };
            if let Some(book) = state
                .books
                .iter()
                .find(|b| std::path::Path::new(&b.path) == canonical)
            {
                books.push(book.clone());
            } else {
                errors.push(
                    json!({"path":path,"error":"Read-only mode can open registered archives only"}),
                );
            }
        }
        json!({"books":books.iter().map(store::Book::shell_value).collect::<Vec<_>>(),"errors":errors})
    } else {
        service
            .clone()
            .dispatch(
                "import_archives",
                json!({"paths":paths,"privacyGeneration":launch_generation}),
            )
            .await?
    };
    // `mainPath` in this payload is already an encoded archive location, the one
    // `Book::shell_value` produced for the shell. Reading it back into a `Book`
    // would put a location in `metadata.main_path`, and encoding that again opened
    // a `%2520`-style path that does not exist: the reader showed "Entry not
    // found" over a blank frame for every archive launched from the command line,
    // Finder or a secondary instance.
    let locations = imported["books"]
        .as_array()
        .ok_or("Imported archives payload is missing its book list")?
        .iter()
        .map(|value| {
            store::ShellBook::new(value).map(|book| book.tab_location(article.as_deref()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let failures = launch_failures(&imported["errors"]);
    // Queue before waking the shell: successful event emission is not delivery.
    // Startup has no native window yet and belongs to the default main shell.
    if locations.is_empty() {
        return Ok(failures);
    }
    let delivered = async {
        if emit {
            let (label, window) = desktop_launch_target(&service)?;
            service
                .queue_launch_locations(&label, locations, launch_generation)
                .await?;
            window.emit("kiwix-open", ()).map_err(|e| e.to_string())?;
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        } else {
            service
                .queue_launch_locations("main", locations, launch_generation)
                .await?;
        }
        Ok::<(), String>(())
    }
    .await;
    match delivered {
        Ok(()) => Ok(failures),
        Err(error) => Err(undelivered_launch_error(error, &failures)),
    }
}

/// The error for a launch whose readable archives could not be handed to a
/// window. The archives that could not be opened at all are still named.
#[cfg(desktop)]
fn undelivered_launch_error(error: String, failures: &[String]) -> String {
    if failures.is_empty() {
        error
    } else {
        format!("{error}\n{}", failures.join("\n"))
    }
}

#[cfg(desktop)]
async fn maintenance(service: Arc<Service>) {
    service.agent_connector_reconcile();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut cycles = 0u32;
    #[cfg(desktop)]
    if let Err(error) = service.start_folder_watch() {
        eprintln!("Monitored folders: {error}");
    }
    if let Err(error) = service
        .clone()
        .dispatch("recover_migrations", json!({}))
        .await
    {
        eprintln!("Library migration recovery: {error}");
    }
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = service.imports_changed() => {},
        }
        if service.ensure_running().is_err() {
            break;
        }
        let Ok(read_only) = service.store.read_only() else {
            continue;
        };
        if read_only {
            continue;
        }
        if let Err(error) = service.clone().dispatch("downloads", json!({})).await {
            eprintln!("Download maintenance: {error}");
        }
        if service.store.data_dir().join("maps").exists() {
            let _ = service.clone().dispatch("map_status", json!({})).await;
        }
        cycles = cycles.wrapping_add(1);
        // Reconcile archive availability and recovered imports on the first
        // cycle as well as every fifth: a file deleted while the app was closed
        // must not be offered as available for the first 15 seconds.
        if !maintenance_reconciles(cycles) {
            continue;
        }
        if let Err(error) = service
            .clone()
            .dispatch("library_maintenance", json!({}))
            .await
        {
            eprintln!("Library maintenance: {error}");
        }
    }
}

/// Availability reconciliation runs on the first maintenance cycle, not only
/// every fifth tick: an archive deleted while the app was closed must not be
/// offered as available for the first 15 seconds (Apple #1301, desktop
/// #1493/#1560).
#[cfg(desktop)]
fn maintenance_reconciles(cycles: u32) -> bool {
    cycles == 1 || cycles.is_multiple_of(5)
}

/// Navigation policy for every desktop webview.
///
/// Archive documents are sandboxed but may still set `location.href` or a meta
/// refresh; CSP does not govern a document's own navigation. Only the shell,
/// the exact origins of open readers, local hand-off schemes, and (in debug
/// builds) the dev server may be navigated to. External `http(s)`, `data:`,
/// `file:` and arbitrary loopback ports are refused before a request.
#[cfg(desktop)]
pub(crate) fn navigation_allowed(url: &tauri::Url, reader_origin: impl FnOnce(&str) -> bool) -> bool {
    match url.scheme() {
        "tauri" => true,
        "about" => matches!(url.as_str(), "about:blank" | "about:srcdoc"),
        // Blob URLs are origin-bound; only accept one whose embedded origin is
        // already allowed (the app's download hand-off).
        "blob" => url
            .as_str()
            .strip_prefix("blob:")
            .and_then(|inner| tauri::Url::parse(inner).ok())
            .is_some_and(|inner| origin_allowed(&inner, reader_origin)),
        "http" | "https" => origin_allowed(url, reader_origin),
        _ => false,
    }
}

#[cfg(desktop)]
fn origin_allowed(url: &tauri::Url, reader_origin: impl FnOnce(&str) -> bool) -> bool {
    let host = url.host_str().unwrap_or_default();
    // Tauri's shell origin on Windows; the macOS shell uses `tauri://localhost`.
    if host == "tauri.localhost" {
        return true;
    }
    #[cfg(debug_assertions)]
    if url.scheme() == "http" && host == "127.0.0.1" && url.port() == Some(1420) {
        return true;
    }
    let origin = url.origin().ascii_serialization();
    reader_origin(&origin)
}

#[cfg(desktop)]
fn shell_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "linux"
    }
}

/// Dark canvas colours from src/platform.css. The window's native background
/// uses the same value so the webview's first frame cannot flash white.
#[cfg(desktop)]
fn shell_dark_background() -> tauri::utils::config::Color {
    #[cfg(target_os = "macos")]
    {
        tauri::utils::config::Color(0x1e, 0x1e, 0x1e, 0xff)
    }
    #[cfg(target_os = "windows")]
    {
        tauri::utils::config::Color(0x20, 0x20, 0x20, 0xff)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        tauri::utils::config::Color(0x22, 0x22, 0x26, 0xff)
    }
}

/// Light canvas colours from src/platform.css, one per desktop skin. The
/// window's native background uses the same value so the webview's first frame
/// cannot flash a different light tint.
#[cfg(desktop)]
fn shell_light_background() -> tauri::utils::config::Color {
    #[cfg(target_os = "macos")]
    {
        tauri::utils::config::Color(0xf6, 0xf6, 0xf6, 0xff)
    }
    #[cfg(target_os = "windows")]
    {
        tauri::utils::config::Color(0xf3, 0xf3, 0xf3, 0xff)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        tauri::utils::config::Color(0xfa, 0xfa, 0xfb, 0xff)
    }
}

/// Runs in the main frame before the page's own scripts. index.html's
/// theme-boot.js consumes it, so the shell paints the saved theme before the
/// first snapshot IPC. Archive frames never receive this main-frame script.
#[cfg(desktop)]
fn shell_boot_script(theme: &str, platform: &str) -> String {
    format!(
        "\n;window.__KIWIX_BOOT__ = Object.freeze({{ theme: {}, platform: {} }});",
        serde_json::to_string(theme).unwrap_or_else(|_| "\"system\"".into()),
        serde_json::to_string(platform).unwrap_or_else(|_| "\"linux\"".into()),
    )
}

#[cfg(desktop)]
fn run_application(options: Options) -> Result<(), String> {
    let path = data_dir(&options)?;
    // Reopen user-granted archives and folders before anything reads the profile.
    if sandbox::is_sandboxed() {
        sandbox::restore(&path);
    }
    // AppKit requires automatic window tabbing to be enabled before the first
    // window exists; the config-created main window is created below.
    #[cfg(target_os = "macos")]
    windows::install_native_tabbing();
    // The saved theme is a paint hint for the first frame. It is read from the
    // profile before any webview exists and validated again by the store.
    let portable = portable::PortableProfile::detect()?;
    // A saved strict read-only policy is startup authority, not just a store
    // setting: the profile must not gain a lock file or a rendezvous socket.
    // One inspection supplies the policy, the first-paint theme and the exact
    // bytes the store reuses, so a normal profile reads state.json once.
    let startup = match &portable {
        Some(_) => None,
        None => Some(store::startup_hint(&path)),
    };
    let shell_theme = match &startup {
        Some(hint) => hint.theme.clone(),
        None => store::saved_theme_hint(&path),
    }
    .unwrap_or_else(|| "system".into());
    let profile_read_only = match &startup {
        Some(hint) => hint.read_only.clone()?,
        None => match &portable {
            Some(profile) => profile.saved_read_only()?,
            None => false,
        },
    };
    let startup_bytes = startup.as_ref().and_then(|hint| hint.bytes.clone());
    let startup_identity = startup.as_ref().and_then(|hint| hint.identity.clone());
    // Install before any Service/store work so a startup panic is captured.
    // Read-only profiles stay inert because they must not mutate the profile.
    crash::install(&path, options.read_only || profile_read_only);
    if options.headless {
        let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        return runtime.block_on(async move {
            // Headless startup owns the profile too, so it claims it the same
            // way the windowed path does. Losing that claim is a second
            // instance, not a startup failure: hand the launch to the owner
            // and exit 0.
            let service = match Service::new_checked(path.clone(), options.private, options.read_only, None) {
                Ok(service) => service,
                #[cfg(unix)]
                Err(store::StoreOpenError::Busy(_)) => {
                    instance::report_busy_profile(
                        &path,
                        std::env::args().collect(),
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                    );
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            };
            let unopened = open_paths(service.clone(), options.paths, options.article, false, None).await?;
            // A headless launch reports on stderr and in the served shell.
            note_startup_launch_outcome(&service, Ok(unopened));
            let background = tokio::spawn(maintenance(service.clone()));
            let url = server::start_preview(service.clone(), options.port).await?;
            println!("KIWIX_READY {url}");
            tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
            background.abort();
            service.shutdown().await
        });
    }
    #[cfg(target_os = "macos")]
    notifications::install_early();
    if let Some(profile) = &portable {
        profile.prepare(options.read_only || profile_read_only)?;
    }
    #[cfg(unix)]
    let initialization_read_only = options.read_only || profile_read_only;
    // Claim the profile before Tauri creates any window. A second instance has
    // to be decided here: a setup hook only fails after the config window
    // exists, and returning that error aborts the process. On Unix the same
    // claim publishes the profile rendezvous, so builds with different bundle
    // identifiers can still activate each other.
    #[cfg(unix)]
    let claimed_lock = match store::claim_profile_lock(&path, initialization_read_only) {
        Ok(lock) => lock,
        Err(store::ProfileLockError::Busy { .. }) => {
            instance::report_busy_profile(
                &path,
                std::env::args().collect(),
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            );
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    };
    // A read-only profile must not gain runtime files, so it publishes no
    // rendezvous: ownership is still the lock, and a second launch gets the
    // dialog instead of an activation.
    #[cfg(unix)]
    let rendezvous = if initialization_read_only {
        None
    } else {
        match instance::Rendezvous::bind(&path) {
            Ok(rendezvous) => Some(rendezvous),
            Err(error) => {
                eprintln!("Instance rendezvous unavailable: {error}");
                None
            }
        }
    };
    // On non-Unix the identifier-keyed plugin forwards/exits secondary
    // invocations before setup takes store.lock; Unix uses the profile
    // rendezvous instead, so this queue stays empty there.
    let pending_launches = Arc::new(std::sync::Mutex::new(Vec::<(Options, PathBuf)>::new()));
    #[cfg(not(unix))]
    let early_launches = pending_launches.clone();
    let mut context = tauri::generate_context!();
    // Every window is created in setup so it can carry the navigation policy.
    // Archive content must never navigate the webview to a network origin.
    for window in &mut context.config_mut().app.windows {
        window.create = false;
    }
    if let Some(profile) = &portable {
        use sha2::Digest;
        // The singleton key must never forward a portable launch to a normal
        // profile or another portable distribution that happens to be running.
        let identity = hex::encode(sha2::Sha256::digest(profile.data.as_os_str().as_encoded_bytes()));
        context.config_mut().identifier.push_str(&format!(".portable.p{}", &identity[..24]));
        // Override application-owned Tauri config/cache/log roots before any
        // WebView exists. WKWebView remains the existing ephemeral incognito
        // store; its OS-managed transient files are not a portability promise.
        context.config_mut().app.app_directories_override =
            Some(tauri::utils::config::AppDirectoriesOverride::Root(profile.data.join("profile")));
    }
    if shell_theme != "system" {
        // AppKit draws the title bar and window chrome before the webview
        // paints; match the saved theme so that chrome cannot flash either.
        let theme = if shell_theme == "dark" {
            tauri::Theme::Dark
        } else {
            tauri::Theme::Light
        };
        for window in &mut context.config_mut().app.windows {
            window.theme = Some(theme);
        }
    }
    let background = if shell_theme == "dark" {
        shell_dark_background()
    } else {
        shell_light_background()
    };
    for window in &mut context.config_mut().app.windows {
        window.background_color = Some(background);
    }
    // Runs in the main frame before the page's own scripts. index.html's
    // theme-boot.js consumes it, so the shell paints the saved theme before the
    // first snapshot IPC. Archive frames never receive this main-frame script.
    let boot_script = shell_boot_script(&shell_theme, shell_platform());
    let builder = tauri::Builder::default().append_invoke_initialization_script(boot_script);
    // Unix builds arbitrate by profile before the builder runs, so the
    // identifier-keyed plugin would never see a second instance. Keep it on the
    // platforms without a profile rendezvous so the same-bundle launch and
    // argument-forwarding path is unchanged there.
    #[cfg(not(unix))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(move |app, args, cwd| {
        // A second launch, with or without archives, brings the window forward,
        // as the Unix rendezvous does (instance.rs). The plugin has already
        // passed this process the foreground right; open_paths returns early
        // for an empty launch, so it cannot focus by itself.
        activate_instance(app);
        match parse_forwarded_options(args.into_iter()) {
            Ok(options) => {
                // Synchronize with setup's manage/drain handoff so an invocation
                // cannot fall between checking for Service and enqueueing.
                let Ok(mut pending) = early_launches.lock() else {
                    eprintln!("Opening additional archives: Startup launch queue unavailable; restart Kiwix and retry.");
                    return;
                };
                if let Some(state) = app.try_state::<Arc<Service>>() {
                    spawn_secondary_paths(app, state.inner().clone(), options, PathBuf::from(cwd));
                } else {
                    pending.push((options, PathBuf::from(cwd)));
                }
            }
            Err(error) => eprintln!("Command-line arguments: {error}"),
        }
    }));
    let builder = builder
        .manage(quit::QuitManager::default())
        .plugin(tauri_plugin_dialog::init());
    // The updater plugin owns signature verification and installation. Its
    // JavaScript commands stay unregistered in capabilities; the shell talks
    // to the app's own commands so privacy and transfer gates cannot be
    // bypassed from the page.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let app = builder
        .on_permission_request(|webview, kind| {
            let label = webview.label();
            // The camera window may use the camera, and only during a capture
            // the user started; nothing else, and never an archive.
            #[cfg(desktop)]
            if label == crate::service::photo_capture::CAMERA_LABEL {
                let active = webview
                    .try_state::<Arc<Service>>()
                    .is_some_and(|state| state.camera_capture_active());
                return if active && matches!(kind, tauri::webview::PermissionKind::Camera) {
                    tauri::webview::PermissionResponse::Allow
                } else {
                    tauri::webview::PermissionResponse::Deny
                };
            }
            if label == "main" || label.starts_with("window-") {
                if let Some(state) = webview.try_state::<Arc<Service>>() {
                    return state.sensor_manager(label).permission_response(label, kind);
                }
            }
            tauri::webview::PermissionResponse::Deny
        })
        .invoke_handler(tauri::generate_handler![
            dispatch,
            menus::update_native_menu,
            quit::validate_quit_flush,
            quit::ack_quit_flush
        ])
        .setup(setup_reporting_failure(move |app| {
            // The lock is held now. Reuse the pre-lock bytes only if the file
            // still has the identity that was inspected; otherwise let the
            // store read the profile once under its own lock.
            let startup_bytes = startup_bytes.filter(|_| {
                startup_identity.as_ref().is_some_and(|expected| {
                    store::state_identity(&path).as_ref() == Some(expected)
                })
            });
            // Unix startup already owns the lock claimed before the builder;
            // the store adopts that descriptor instead of locking again.
            #[cfg(unix)]
            let service_result = Service::new_with_claimed_lock(
                path.clone(),
                options.private,
                options.read_only,
                claimed_lock,
                startup_bytes,
            );
            #[cfg(not(unix))]
            let service_result = Service::new_checked(path.clone(), options.private, options.read_only, startup_bytes);
            let service = match service_result {
                Ok(service) => service,
                // Another live process owns the profile. Unix decides this
                // before the builder, so this arm is defensive there; keeping
                // it means a busy profile can never reach Tauri as a setup
                // error, which is what aborted the second instance.
                Err(store::StoreOpenError::Busy(error)) => {
                    return profile_busy_in_setup(app, &path, &error.to_string());
                }
                // The profile cannot be opened. `setup_reporting_failure`
                // shows this text and exits 1; Tauri never sees the error.
                Err(error) => return Err(error.to_string().into()),
            };
            *service
                .app
                .lock()
                .map_err(|_| "Application handle unavailable")? = Some(app.handle().clone());
            #[cfg(target_os = "macos")]
            service.initialize_notifications();
            let pending = {
                let mut pending = pending_launches.lock().map_err(|_| "Startup launch queue unavailable")?;
                app.manage(service.clone());
                std::mem::take(&mut *pending)
            };
            // One background check at most per day, after the window is
            // usable. It never downloads or installs anything.
            crate::update::start_daily_check(service.clone());
            // Config windows are created here, not by Tauri, so every one gets
            // the navigation policy that keeps archive content off the network.
            for configuration in &app.config().app.windows {
                let nav_service = service.clone();
                let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), configuration)?
                    .on_navigation(move |url| {
                        navigation_allowed(url, |origin| nav_service.has_reader_origin(origin))
                    });
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                let builder = if let Some(profile) = &portable {
                    builder.data_directory(profile.webview_directory(&configuration.label)?)
                } else {
                    builder
                };
                #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                let builder = {
                    let _ = &portable;
                    builder
                };
                let builder = if portable.is_some() {
                    builder.incognito(true)
                } else {
                    builder
                };
                builder.build()?;
            }
            menus::install(app.handle())?;
            // The config-created main window predates this bridge.
            #[cfg(target_os = "macos")]
            crate::windows::adopt_existing(app.handle());
            // A slow or failed frontend bundle must show the placeholder and
            // then diagnostics, never an unexplained white window.
            crate::windows::watch_startup(app.handle(), "main");
            let startup_generation = service.launch_generation();
            let startup_paths = options.paths;
            let startup_article = options.article;
            // A lookup only prefills a search: failing to queue it must never
            // stop the app from starting.
            if let Some(query) = options.search {
                if let Err(error) = tauri::async_runtime::block_on(open_search(service.clone(), query, false, None)) {
                    eprintln!("Look up in Kiwix: {error}");
                }
            }
            let state = service.store.read()?;
            // The config window predates this hook, so it has no under-page
            // colour yet. Set the shell paper before any frame can load.
            crate::windows::apply_under_page_all(app.handle(), &state.settings);
            crate::windows::restore_main(app.handle(), &state);
            if state.settings.restore_session {
                for (label, session) in state
                    .windows
                    .iter()
                    .filter(|(key, _)| key.starts_with("window-"))
                    .take(15)
                {
                    if let Err(error) =
                        crate::windows::create(app.handle(), label, session.geometry.as_ref())
                    {
                        // One saved window that cannot be built must not deny
                        // the rest of the restored session or the library.
                        crate::windows::report_window_creation_failure(
                            app.handle(),
                            label,
                            &error.to_string(),
                        );
                    }
                }
                if let Err(error) = crate::windows::restore_native_tabs(app.handle(), &state.windows) {
                    service.note_startup_warning(format!("Window groups could not be restored: {error}"));
                }
            }
            // Run CLI archive imports off the setup main thread so large files
            // cannot freeze the UI or trip the 10 s startup watchdog. Capture
            // the launch generation synchronously above and target "main".
            if !startup_paths.is_empty() {
                let startup_service = service.clone();
                let main_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let outcome = open_paths(
                        startup_service.clone(),
                        startup_paths,
                        startup_article,
                        false,
                        Some(startup_generation),
                    )
                    .await;
                    let unopened = note_startup_launch_outcome(&startup_service, outcome);
                    if let Some(window) = main_app.get_webview_window("main") {
                        let _ = window.emit("kiwix-open", ());
                        // The notice is in the main window: bring that window
                        // forward so a failed "Open with" is not left behind
                        // another restored window.
                        if unopened {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                });
            }
            for (options, cwd) in pending {
                spawn_secondary_paths(app.handle(), service.clone(), options, cwd);
            }
            // Accept only after the service and windows exist, so a forwarded
            // launch can be applied immediately; connections made during setup
            // wait in the listener backlog.
            #[cfg(unix)]
            if let Some(rendezvous) = rendezvous {
                rendezvous.spawn(app.handle().clone());
            }
            tauri::async_runtime::spawn(maintenance(service));
            eprintln!("KIWIX_DESKTOP_READY");
            Ok(())
        }))
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Some(state) = window.try_state::<Arc<Service>>() {
                    let _ = crate::windows::capture(window.app_handle(), &state.store);
                }
            }
            #[cfg(desktop)]
            if matches!(event, tauri::WindowEvent::Destroyed)
                && window.label() == crate::service::photo_capture::CAMERA_LABEL
            {
                if let Some(state) = window.try_state::<Arc<Service>>() {
                    state.camera_capture_window_destroyed();
                }
                return;
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                quit::window_destroyed(window.app_handle(), window.label());
                if let Some(state) = window.try_state::<Arc<Service>>() {
                    let service = state.inner().clone();
                    let label = window.label().to_string();
                    tauri::async_runtime::spawn(async move {
                        let _ = service
                            .dispatch("close_window", json!({"__windowLabel":label}))
                            .await;
                    });
                }
            }
        })
        .build(context)
        .map_err(|e| e.to_string())?;
    let quit_flags = QuitFlags::default();
    app.run(move |app, event| match event {
        tauri::RunEvent::ExitRequested { api, code, .. } if !quit_flags.authorized.load(Ordering::SeqCst) => {
            api.prevent_exit();
            if !quit_flags.exiting.swap(true, Ordering::SeqCst) {
                if let Some(state) = app.try_state::<Arc<Service>>() {
                    let service = state.inner().clone();
                    let app = app.clone();
                    let flags = quit_flags.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = quit::prepare(&app).await {
                            // prepare cancelled the attempt and the shells resumed.
                            // The relaunch of an installed update is decided by the
                            // person's answer, not dropped behind their back.
                            eprintln!("Application Quit cancelled: {error}");
                            ask_after_failed_quit(app, service, quit::QuitFailure::Flush, error, code, flags);
                            return;
                        }
                        if let Err(error) = service.shutdown().await {
                            eprintln!("Application Quit needs a retry: service retirement failed: {error}");
                            // Keep the acknowledged shells frozen. Their ordinary commands
                            // cannot run once the service has started retiring.
                            ask_after_failed_quit(app, service, quit::QuitFailure::Retirement, error, code, flags);
                            return;
                        }
                        flags.authorized.store(true, Ordering::SeqCst);
                        finish_authorized_quit(&app, code);
                    });
                } else {
                    quit_flags.exiting.store(false, Ordering::SeqCst);
                    eprintln!("Application Quit cancelled: library service unavailable");
                }
            } else {
                // Held back while an attempt or its dialog is in progress. The
                // dialog acts on it when it closes.
                quit_flags.requested_again.store(true, Ordering::SeqCst);
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Opened { urls } => {
            let mut paths = Vec::new();
            let mut links = Vec::new();
            let mut searches = Vec::new();
            for url in urls {
                match url.scheme() {
                    "file" => {
                        if let Ok(path) = url.to_file_path() {
                            paths.push(path.to_string_lossy().into_owned());
                        }
                    }
                    "zim" => links.push(url.to_string()),
                    "kiwix" => {
                        if let Ok(query) = launch_search::parse_url(url.as_str()) {
                            searches.push(query);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(state) = app.try_state::<Arc<Service>>() {
                let service = state.inner().clone();
                if !paths.is_empty() {
                    let service = service.clone();
                    #[cfg(target_os = "macos")]
                    let app = app.clone();
                    let generation = service.launch_generation();
                    tauri::async_runtime::spawn(async move {
                        // Open With grants the file but not its folder, which the
                        // sandbox build needs to read the archive.
                        #[cfg(target_os = "macos")]
                        request_folder_access(&app, &paths).await;
                        // Same report as a second launch (spawn_secondary_paths).
                        let outcome = open_paths(service.clone(), paths, None, true, Some(generation)).await;
                        report_launch_outcome(&app, &service, "Import archives", outcome);
                    });
                }
                if !links.is_empty() {
                    let service = service.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = open_zim_links(service, links).await {
                            eprintln!("Opening zim:// link: {error}");
                        }
                    });
                }
                if !searches.is_empty() {
                    tauri::async_runtime::spawn(async move {
                        for query in searches {
                            // A cold-start URL can arrive before the main window
                            // exists. Queue it for the default shell instead of
                            // dropping it.
                            if let Err(error) = open_search(service.clone(), query.clone(), true, None).await {
                                if let Err(queue_error) = open_search(service.clone(), query, false, None).await {
                                    eprintln!("Opening kiwix:// link: {error}; queueing it also failed: {queue_error}");
                                }
                            }
                        }
                    });
                }
            }
        }
        _ => {}
    });
    Ok(())
}
/// Install the process-wide rustls provider before any reqwest 0.13 client is built.
/// The updater and librqbit request `rustls-no-provider`; LocalSwarm and reqwest 0.12
/// already use ring, so one provider serves every TLS client in the process.
pub(crate) fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(desktop)]
pub fn run() {
    install_crypto_provider();
    let arguments: Vec<String> = std::env::args().collect();
    // Decided from the raw arguments, so an unparsable command line that asks
    // for `--headless` still reports on stderr only.
    let headless = arguments.iter().any(|argument| argument == "--headless");
    if let Err(error) = parse_options(arguments.into_iter()).and_then(run_application) {
        exit_after_startup_failure(&error, headless);
    }
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn run() {
    install_crypto_provider();
    #[cfg(target_os = "ios")]
    notifications::install_early();
    mobile::run();
}

#[cfg(all(test, desktop))]
mod launch_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn reqwest_013_client_builds_with_the_installed_ring_provider() {
        install_crypto_provider();
        reqwest13::Client::builder().build().expect("reqwest 0.13 client needs the installed provider");
    }

    #[test]
    fn shell_boot_script_carries_the_saved_theme_and_platform() {
        assert_eq!(
            shell_boot_script("dark", "macos"),
            "\n;window.__KIWIX_BOOT__ = Object.freeze({ theme: \"dark\", platform: \"macos\" });"
        );
        assert_eq!(
            shell_boot_script("system", "linux"),
            "\n;window.__KIWIX_BOOT__ = Object.freeze({ theme: \"system\", platform: \"linux\" });"
        );
    }

    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("kiwix-launch-test-{}", Uuid::new_v4())))
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn options(arguments: &[&str]) -> Options {
        parse_options(
            std::iter::once("kiwix".to_owned())
                .chain(arguments.iter().map(|arg| (*arg).to_owned())),
        )
        .unwrap()
    }
    fn fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/small.zim")
            .to_string_lossy()
            .into_owned()
    }
    /// Main document of `tests/fixtures/main_path.zim`: a space, a literal `?`,
    /// `#`, `%` and non-ASCII text in one libzim path.
    const MAIN_PATH_FIXTURE: &str = "main page 100% ?#\u{e9}.html";
    fn main_path_fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/main_path.zim")
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn archive_launch_locations_encode_the_main_path_exactly_once() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        open_paths(service.clone(), vec![main_path_fixture()], None, false, None)
            .await
            .unwrap();
        let snapshot = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        let launch = snapshot["launchLocations"][0].clone();
        let main_location = crate::server::encode_path(MAIN_PATH_FIXTURE);
        // The import payload's `mainPath` is already a location. Encoding it a
        // second time queued `main%2520page…`, which no archive contains, so the
        // first tab of every archive opened from the command line, Finder or a
        // secondary instance failed with "Entry not found" over a blank frame.
        assert_eq!(launch["path"], main_location);
        let opened = service
            .clone()
            .dispatch(
                "open_article",
                json!({
                    "bookId": launch["bookId"],
                    "path": launch["path"],
                    "parentOrigin": "tauri://localhost",
                }),
            )
            .await
            .unwrap();
        assert_eq!(opened["missing"], false);
        assert_eq!(opened["path"], main_location);
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn incompatible_secondary_launches_reject_before_import_or_profile_creation() {
        let profile = Profile::new();
        let unrequested = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let archive = fixture();
        let cwd = std::env::current_dir().unwrap();
        for arguments in [
            vec!["--private", archive.as_str()],
            vec!["--read-only", archive.as_str()],
            vec![
                "--data-dir",
                unrequested.0.to_str().unwrap(),
                archive.as_str(),
            ],
        ] {
            let error = open_secondary_paths(service.clone(), options(&arguments), &cwd)
                .await
                .unwrap_err();
            assert!(
                error.contains("No archives were opened or imported"),
                "{error}"
            );
            assert!(error.contains("Quit the running Kiwix"), "{error}");
            assert!(service.store.read().unwrap().books.is_empty());
            assert!(!unrequested.0.exists());
        }
        // A mode cutover after validation must still reject under the import's
        // existing privacy guard, before the archive can enter the new profile state.
        let generation = service.launch_generation();
        validate_secondary_launch(&service, &options(&[]), &cwd).unwrap();
        service
            .clone()
            .dispatch("save_settings", json!({"settings":{"privateMode":true}}))
            .await
            .unwrap();
        let error = open_paths(
            service.clone(),
            vec![archive],
            None,
            false,
            Some(generation),
        )
        .await
        .unwrap_err();
        assert!(error.contains("privacy transition"), "{error}");
        assert!(service.store.read().unwrap().books.is_empty());
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn unreadable_launch_archives_are_reported_and_readable_ones_still_open() {
        let profile = Profile::new();
        let scratch = Profile::new();
        std::fs::create_dir_all(&scratch.0).unwrap();
        let broken = scratch.0.join("Broken Book.zim");
        std::fs::write(&broken, b"not a ZIM archive").unwrap();
        let broken = broken.to_string_lossy().into_owned();
        let service = Service::new(profile.0.clone(), false, false).unwrap();

        // Nothing could be opened. This used to be `Ok(())` with the reason on
        // stderr only, so a double-clicked archive did nothing visible.
        let failures = open_paths(service.clone(), vec![broken.clone()], None, false, None)
            .await
            .unwrap();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].starts_with("Broken Book.zim: "), "{failures:?}");
        assert!(failures[0].len() > "Broken Book.zim: ".len(), "{failures:?}");
        assert!(service.store.read().unwrap().books.is_empty());

        // A second launch (and macOS "Open with") reports the same list in a dialog.
        let cwd = std::env::current_dir().unwrap();
        let secondary = open_secondary_paths(service.clone(), options(&[broken.as_str()]), &cwd).await;
        assert_eq!(secondary.as_ref().unwrap(), &failures);
        assert_eq!(launch_failure_detail(secondary), Some(failures[0].clone()));
        assert_eq!(launch_failure_detail(Ok(Vec::new())), None);
        assert_eq!(
            launch_failure_detail(Err("Cannot open archives while application Quit is pending".into())).as_deref(),
            Some("Cannot open archives while application Quit is pending")
        );

        // Mixed launch that cannot reach a window (this test has none): the
        // error still names the archive that could not be opened.
        let undelivered = open_paths(service.clone(), vec![broken.clone(), fixture()], None, true, None)
            .await
            .unwrap_err();
        assert_eq!(
            undelivered,
            format!("No native application is available to open the archive\n{}", failures[0])
        );
        assert_eq!(undelivered_launch_error("No window".into(), &[]), "No window");

        // Mixed launch: the readable archive opens, the other one is reported.
        let mixed = open_paths(service.clone(), vec![broken.clone(), fixture()], None, false, None)
            .await
            .unwrap();
        assert_eq!(mixed, failures);
        let snapshot = service.clone().dispatch("snapshot", json!({})).await.unwrap();
        assert_eq!(snapshot["launchLocations"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["books"].as_array().unwrap().len(), 1);
        assert!(snapshot["startupWarnings"].as_array().unwrap().is_empty());

        // At startup the list goes to the shell's startup notice.
        assert!(note_startup_launch_outcome(&service, Ok(mixed)));
        assert!(!note_startup_launch_outcome(&service, Ok(Vec::new())));
        let snapshot = service.clone().dispatch("snapshot", json!({})).await.unwrap();
        assert_eq!(
            snapshot["startupWarnings"],
            json!([format!("Could not open startup archive: {}", failures[0])])
        );
        service.shutdown().await.unwrap();
        drop(service);

        // Read-only mode opens registered archives and names the one it refused.
        let service = Service::new(profile.0.clone(), false, true).unwrap();
        let refused = open_paths(service.clone(), vec![fixture(), broken], None, false, None)
            .await
            .unwrap();
        assert_eq!(
            refused,
            vec!["Broken Book.zim: Read-only mode can open registered archives only".to_string()]
        );
        let snapshot = service.clone().dispatch("snapshot", json!({})).await.unwrap();
        assert_eq!(snapshot["launchLocations"].as_array().unwrap().len(), 1);
        service.shutdown().await.unwrap();
    }

    #[test]
    fn fatal_startup_errors_reach_a_dialog_and_exit_1_unless_headless() {
        let profile = Profile::new();
        std::fs::create_dir_all(&profile.0).unwrap();
        std::fs::write(profile.0.join("state.json"), b"{").unwrap();
        // The pre-builder failure for a damaged profile: `run_application`
        // returns this text from `hint.read_only.clone()?`.
        let error = store::startup_hint(&profile.0).read_only.unwrap_err();
        assert!(error.contains("state.json") && error.contains("The original is untouched."), "{error}");

        let windowed = startup_failure(&error, false);
        assert_eq!(windowed.exit_code, 1);
        let message = windowed.dialog.expect("a windowed launch explains itself in a dialog");
        // The store's text is shown whole, after a fixed sentence: the dialog
        // text can never begin like a command-line option.
        assert_eq!(message, format!("Kiwix could not start.\n\n{error}"));
        // A headless launch reports on stderr only and exits with the same status.
        assert_eq!(startup_failure(&error, true), StartupFailure { dialog: None, exit_code: 1 });
        // An argument error that itself starts with an option name.
        let usage = parse(&["--data-dir"]).err().unwrap();
        assert_eq!(usage, "--data-dir requires a path");
        assert!(startup_failure(&usage, false).dialog.unwrap().starts_with("Kiwix could not start."));
        assert_eq!(std::fs::read(profile.0.join("state.json")).unwrap(), b"{");
    }

    #[test]
    fn a_profile_saved_by_another_version_shows_the_stores_recovery_text() {
        let profile = Profile::new();
        std::fs::create_dir_all(&profile.0).unwrap();
        let saved = br#"{"version":3,"settings":{}}"#;
        std::fs::write(profile.0.join("state.json"), saved).unwrap();
        // The setup-hook failure: the store refuses the profile, and that
        // error must become the dialog, not a Tauri setup panic.
        let error = match Service::new_checked(profile.0.clone(), false, false, None) {
            Err(error @ store::StoreOpenError::Fatal(_)) => error.to_string(),
            Err(store::StoreOpenError::Busy(error)) => panic!("unexpectedly busy: {error}"),
            Ok(_) => panic!("a version 3 profile must not open"),
        };
        let failure = startup_failure(&error, false);
        assert_eq!(failure.exit_code, 1);
        let message = failure.dialog.unwrap();
        assert!(message.ends_with(&error), "{message}");
        for sentence in [
            "This profile was saved by a different version of Kiwix (version 3;",
            "Open it with that version, or choose another profile.",
            "Your saved file was not changed.",
        ] {
            assert!(message.contains(sentence), "{message}");
        }
        assert_eq!(std::fs::read(profile.0.join("state.json")).unwrap(), saved);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn quitting_without_saving_still_retires_the_service_when_the_profile_is_gone() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        // A reader listener is open: a socket that only retirement releases.
        open_paths(service.clone(), vec![fixture()], None, false, None)
            .await
            .unwrap();
        let snapshot = service.clone().dispatch("snapshot", json!({})).await.unwrap();
        let launch = snapshot["launchLocations"][0].clone();
        let opened = service
            .clone()
            .dispatch(
                "open_article",
                json!({
                    "bookId": launch["bookId"],
                    "path": launch["path"],
                    "parentOrigin": "tauri://localhost",
                }),
            )
            .await
            .unwrap();
        let reader = url::Url::parse(opened["baseUrl"].as_str().unwrap()).unwrap();
        let listener = ("127.0.0.1", reader.port().expect("reader listeners use an explicit port"));
        assert!(std::net::TcpStream::connect(listener).is_ok(), "the reader is listening");
        service
            .store
            .update(|state| {
                state.settings.zoom = 1.25;
                Ok(())
            })
            .unwrap();
        // A reading-position style change waits in memory for the next write.
        service
            .store
            .update_browsing(|state| {
                state.recent_searches = vec!["pending".into()];
                Ok(())
            })
            .unwrap();
        // The volume that holds the profile goes away.
        std::fs::remove_dir_all(&profile.0).unwrap();
        assert!(service.store.update(|_| Ok(())).is_err());
        // Nothing has retired the service yet.
        assert!(service.ensure_running().is_ok());

        let labels = quit::FailedQuitLabels {
            retry: "Retry".into(),
            quit_without_saving: "Quit Without Saving".into(),
            cancel: "Cancel".into(),
        };
        // Only the explicit button moves on.
        assert_eq!(
            quit::failed_quit_action(
                quit::QuitFailure::Flush,
                &tauri_plugin_dialog::MessageDialogResult::Custom("Quit Without Saving".into()),
                &labels,
            ),
            quit::FailedQuitAction::QuitWithoutSaving
        );
        // The forced path is the first and only retirement in this test. It
        // must return although the profile cannot be written, and it must
        // still have run the stages that release resources: the listener is
        // closed and the service refuses commands.
        retire_without_saving(&service).await;
        assert!(
            std::net::TcpStream::connect(listener).is_err(),
            "quitting without saving left the reader listener open"
        );
        assert!(service.ensure_running().is_err());
        assert!(service.clone().dispatch("snapshot", json!({})).await.is_err());
        // The same retirement as an ordinary Quit still reports what could not
        // be saved, which is why the person had to be asked.
        let error = service.shutdown().await.unwrap_err();
        assert!(error.contains("Writing the last reading position"), "{error}");
    }

    #[tokio::test]
    async fn secondary_launches_accept_matching_modes_profiles_and_omitted_flags() {
        let profile = Profile::new();
        let cwd = std::env::current_dir().unwrap();
        for (private, read_only) in [(false, false), (true, false), (false, true), (true, true)] {
            // A read-only profile must already exist.
            drop(Service::new(profile.0.clone(), false, false).unwrap());
            let service = Service::new(profile.0.clone(), private, read_only).unwrap();
            assert!(validate_secondary_launch(&service, &options(&[]), &cwd).is_ok());
            assert_eq!(
                validate_secondary_launch(&service, &options(&["--private"]), &cwd).is_ok(),
                private
            );
            assert_eq!(
                validate_secondary_launch(&service, &options(&["--read-only"]), &cwd).is_ok(),
                read_only
            );
            let equivalent = profile.0.join(".");
            let requested = options(&["--data-dir", equivalent.to_str().unwrap()]);
            assert!(validate_secondary_launch(&service, &requested, &cwd).is_ok());
            let relative = options(&[
                "--data-dir",
                profile.0.file_name().unwrap().to_str().unwrap(),
            ]);
            assert!(
                validate_secondary_launch(&service, &relative, profile.0.parent().unwrap()).is_ok()
            );
            service.shutdown().await.unwrap();
            drop(service);
        }
        let missing = profile.0.join("missing").join("..").join("other");
        assert_eq!(
            profile_identity(&missing, &cwd).unwrap(),
            profile.0.join("other")
        );
        assert!(!profile.0.join("missing").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn secondary_profile_identity_accepts_existing_symlink_alias() {
        let profile = Profile::new();
        let alias = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        std::os::unix::fs::symlink(&profile.0, &alias.0).unwrap();
        let requested = options(&["--data-dir", alias.0.to_str().unwrap()]);
        assert!(
            validate_secondary_launch(&service, &requested, &std::env::current_dir().unwrap())
                .is_ok()
        );
        service.shutdown().await.unwrap();
        std::fs::remove_file(&alias.0).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_read_only_policy_claims_without_writable_files_or_rendezvous() {
        use std::collections::BTreeMap;
        fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
            fn walk(root: &Path, directory: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
                for entry in std::fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    let metadata = std::fs::symlink_metadata(&path).unwrap();
                    if metadata.is_dir() {
                        out.insert(relative, None);
                        walk(root, &path, out);
                    } else {
                        out.insert(relative, Some(std::fs::read(&path).unwrap()));
                    }
                }
            }
            let mut out = BTreeMap::new();
            walk(root, root, &mut out);
            out
        }
        let profile = Profile::new();
        {
            let store = store::Store::new(profile.0.clone()).unwrap();
            store
                .update(|state| {
                    state.settings.read_only = true;
                    Ok(())
                })
                .unwrap();
        }
        let before = tree(&profile.0);
        // This is the startup decision `run_application` makes before claiming:
        // the saved desktop policy is read from state.json and must win even
        // though no --read-only flag was passed.
        let initialization_read_only = store::saved_desktop_read_only(&profile.0).unwrap();
        assert!(initialization_read_only);
        let lock = store::claim_profile_lock(&profile.0, initialization_read_only).unwrap();
        let rendezvous = if initialization_read_only {
            None
        } else {
            Some(instance::Rendezvous::bind(&profile.0).unwrap())
        };
        assert!(rendezvous.is_none());
        let reopened =
            store::Store::new_with_claimed_lock(profile.0.clone(), false, None, lock, None).unwrap();
        assert!(reopened.read_only().unwrap());
        drop(reopened);
        assert!(!instance::socket_dir(&profile.0).exists());
        assert_eq!(tree(&profile.0), before);
        // The headless/checked opener reads the same saved policy: with the
        // lock file missing it must fail instead of recreating it writable.
        let lock_path = profile.0.join("store.lock");
        std::fs::remove_file(&lock_path).unwrap();
        assert!(store::Store::new_with_mode_checked(profile.0.clone(), false, None).is_err());
        assert!(!lock_path.exists());
    }

    #[tokio::test]
    async fn launch_queue_survives_snapshots_and_only_target_window_can_acknowledge() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let article = "A/encoded%20article?query=a%26b%23c#part%202";
        open_paths(
            service.clone(),
            vec![fixture()],
            Some(article.into()),
            false,
            None,
        )
        .await
        .unwrap();
        let first = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        let launch = first["launchLocations"][0].clone();
        assert_eq!(launch["path"], article);
        assert_eq!(launch["targetWindow"], "main");
        assert!(launch["launchId"].as_str().is_some_and(|id| !id.is_empty()));
        // The queue, not an eagerly persisted duplicate tab, owns startup delivery.
        assert!(first["tabs"].as_array().unwrap().is_empty());
        assert!(service.store.read().unwrap().tabs.is_empty());
        let repeated = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        assert_eq!(repeated["launchLocations"], first["launchLocations"]);
        let other = service
            .clone()
            .dispatch("snapshot", json!({"__windowLabel":"window-new"}))
            .await
            .unwrap();
        assert!(other["launchLocations"].as_array().unwrap().is_empty());

        service
            .queue_launch_locations(
                "window-new",
                vec![store::TabLocation {
                    book_id: launch["bookId"].as_str().unwrap().into(),
                    path: article.into(),
                    title: "Queued".into(),
                    scroll: 0.0,
                }],
                1,
            )
            .await
            .unwrap();
        // This first snapshot stands in for a newly loaded window that missed the event.
        let new_window = service
            .clone()
            .dispatch("snapshot", json!({"__windowLabel":"window-new"}))
            .await
            .unwrap();
        let other_launch = new_window["launchLocations"][0].clone();
        assert_eq!(other_launch["targetWindow"], "window-new");
        assert_eq!(other_launch["path"], article);
        assert_ne!(other_launch["launchId"], launch["launchId"]);

        service
            .clone()
            .dispatch(
                "ack_launch_locations",
                json!({
                    "__windowLabel":"window-new", "privacyGeneration":1,
                    "ids":[launch["launchId"], "unknown-id"]
                }),
            )
            .await
            .unwrap();
        let remaining = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        assert_eq!(remaining["launchLocations"], first["launchLocations"]);
        assert!(service
            .clone()
            .dispatch("ack_launch_locations", json!({"ids":[launch["launchId"]]}))
            .await
            .is_err());
        service
            .clone()
            .dispatch(
                "ack_launch_locations",
                json!({
                    "privacyGeneration":1, "ids":[launch["launchId"], other_launch["launchId"]]
                }),
            )
            .await
            .unwrap();
        let main = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        assert!(main["launchLocations"].as_array().unwrap().is_empty());
        let other = service
            .clone()
            .dispatch("snapshot", json!({"__windowLabel":"window-new"}))
            .await
            .unwrap();
        assert_eq!(other["launchLocations"], new_window["launchLocations"]);

        service
            .clone()
            .dispatch("save_settings", json!({"settings":{"privateMode":true}}))
            .await
            .unwrap();
        let after_cutover = service
            .clone()
            .dispatch("snapshot", json!({"__windowLabel":"window-new"}))
            .await
            .unwrap();
        assert!(after_cutover["launchLocations"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(service
            .queue_launch_locations("main", vec![], 1)
            .await
            .is_err());
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn zim_links_queue_library_articles_and_reject_untrusted_forms() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        open_paths(service.clone(), vec![fixture()], None, false, None)
            .await
            .unwrap();
        let first = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        let book = first["books"][0].clone();
        let id = book["id"].as_str().unwrap().to_string();
        let main_path = book["mainPath"].as_str().unwrap().to_string();
        let generation = first["privacyGeneration"].as_u64().unwrap();
        let ids: Vec<_> = first["launchLocations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|launch| launch["launchId"].clone())
            .collect();
        service
            .clone()
            .dispatch(
                "ack_launch_locations",
                json!({"ids":ids,"privacyGeneration":generation}),
            )
            .await
            .unwrap();

        let queued = service
            .queue_zim_links(
                "main",
                vec![
                    // Upstream Apple stores UUIDs in uppercase; the host is
                    // normalized but the article path is preserved exactly once.
                    format!(
                        "zim://{}/A/Page%20One?query=x#part",
                        id.to_ascii_uppercase()
                    ),
                    format!("zim://{id}"),
                ],
                generation,
            )
            .await
            .unwrap();
        assert_eq!(queued, 2);
        let snapshot = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        let locations = snapshot["launchLocations"].as_array().unwrap();
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0]["bookId"], id.as_str());
        assert_eq!(locations[0]["path"], "A/Page%20One?query=x#part");
        assert_eq!(locations[0]["title"], book["title"]);
        // An empty article path resolves to the archive's own main document.
        assert_eq!(locations[1]["path"], main_path.as_str());

        let before = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        for link in [
            format!("zim://{id}/../etc/passwd"),
            format!("zim://{id}/A/%2e%2e/secret"),
            format!("zim://{id}:8080/A"),
            "zim://example.com/A".to_string(),
            format!("zim://{}/A", Uuid::new_v4()),
            "https://example.com/A".to_string(),
        ] {
            assert!(
                service
                    .queue_zim_links("main", vec![link.clone()], generation)
                    .await
                    .is_err(),
                "accepted {link}"
            );
        }
        // Rejection is all-or-nothing: no partial batch reaches the queue.
        let after = service
            .clone()
            .dispatch("snapshot", json!({}))
            .await
            .unwrap();
        assert_eq!(after["launchLocations"], before["launchLocations"]);
        service.shutdown().await.unwrap();
    }

    fn parse(arguments: &[&str]) -> Result<Options, String> {
        parse_options(
            std::iter::once("kiwix".to_owned())
                .chain(arguments.iter().map(|arg| (*arg).to_owned())),
        )
    }

    #[test]
    fn look_up_arguments_only_prefill_a_search() {
        assert_eq!(options(&["--search", "  Bahrain\u{200B} "]).search.as_deref(), Some("Bahrain"));
        assert_eq!(
            options(&["--open-url", "kiwix://search?q=Manama"]).search.as_deref(),
            Some("Manama")
        );
        let forwarded = parse_forwarded_options(
            ["kiwix", "--open-url", "kiwix://search?q=x"].map(String::from).into_iter(),
        )
        .unwrap();
        assert_eq!(forwarded.search.as_deref(), Some("x"));
        // The registered handler passes exactly one link; anything beside it,
        // or a link to another action, is refused rather than partly applied.
        for bad in [
            &["--open-url"][..],
            &["--open-url", "kiwix://search?q=a", "--data-dir", "elsewhere"],
            &["--open-url", "kiwix://search?q=a", "file.zim"],
            &["--private", "--open-url", "kiwix://search?q=a"],
            &["--open-url", "kiwix://library"],
            &["--open-url", "C:\\archive.zim"],
            &["--search"],
            &["--search", "\u{202E}\u{200B} "],
            &["--search", "a", "file.zim"],
            &["--search", "a", "--headless"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn desktop_entries_accept_a_bare_file_or_search_url() {
        assert_eq!(
            options(&["kiwix://search?q=Manama"]).search.as_deref(),
            Some("Manama")
        );
        // A file URL names a local path in the host's own form: Windows needs
        // a drive letter and yields backslashes.
        #[cfg(not(windows))]
        let (url, path) = ("file:///tmp/", "/tmp/");
        #[cfg(windows)]
        let (url, path) = ("file:///C:/tmp/", "C:\\tmp\\");
        assert_eq!(
            options(&[&format!("{url}My%20Archive.zim")]).paths,
            vec![format!("{path}My Archive.zim")]
        );
        assert_eq!(
            options(&[&format!("{url}One.zim"), &format!("{url}Two.zim")]).paths,
            vec![format!("{path}One.zim"), format!("{path}Two.zim")]
        );
        assert!(parse(&["kiwix://library"]).is_err());
        assert!(parse(&["kiwix://search?q=a&open=b"]).is_err());
    }

    #[tokio::test]
    async fn launch_search_keeps_only_the_latest_request_until_acknowledged() {
        let profile = Profile::new();
        let service = Service::new(profile.0.clone(), false, false).unwrap();
        let snapshot = |label: &'static str| {
            let service = service.clone();
            async move {
                service
                    .dispatch("snapshot", json!({"__windowLabel": label}))
                    .await
                    .unwrap()["launchSearch"]
                    .clone()
            }
        };
        assert!(snapshot("main").await.is_null());
        service.queue_launch_search("main", "first".into(), 1).await.unwrap();
        let first = snapshot("main").await;
        assert_eq!(first["query"], "first");
        service.queue_launch_search("main", "second".into(), 1).await.unwrap();
        let second = snapshot("main").await;
        assert_eq!(second["query"], "second");
        assert_ne!(second["launchId"], first["launchId"]);
        // Another window never sees it.
        assert!(snapshot("window-other").await.is_null());
        // A stale acknowledgement, or one from another window, keeps the newer request.
        service
            .clone()
            .dispatch("ack_launch_search", json!({"id": first["launchId"]}))
            .await
            .unwrap();
        service
            .clone()
            .dispatch(
                "ack_launch_search",
                json!({"__windowLabel": "window-other", "id": second["launchId"]}),
            )
            .await
            .unwrap();
        assert_eq!(snapshot("main").await["launchId"], second["launchId"]);
        service
            .clone()
            .dispatch("ack_launch_search", json!({"id": second["launchId"]}))
            .await
            .unwrap();
        assert!(snapshot("main").await.is_null());
        // A request from an older privacy generation is refused.
        assert!(service.queue_launch_search("main", "old".into(), 0).await.is_err());
    }
}

#[cfg(all(test, desktop))]
mod maintenance_cadence_tests {
    use super::maintenance_reconciles;

    #[test]
    fn availability_reconciles_on_the_first_cycle_and_every_fifth() {
        // A file deleted while the app was closed must flip on startup, not
        // after the first five-tick interval (Apple #1301, desktop #1493/#1560).
        assert!(maintenance_reconciles(1));
        assert!(!maintenance_reconciles(2));
        assert!(!maintenance_reconciles(3));
        assert!(!maintenance_reconciles(4));
        assert!(maintenance_reconciles(5));
        assert!(!maintenance_reconciles(6));
        assert!(!maintenance_reconciles(14));
        assert!(maintenance_reconciles(15));
    }
}

#[cfg(all(test, desktop))]
mod launch_target_tests {
    use super::launch_target_index;

    #[test]
    fn launches_pick_reader_windows_and_never_internal_ones() {
        // Labels arrive sorted, as desktop_launch_target sorts them.
        assert_eq!(launch_target_index(&[("main", false), ("window-a", true)]), Some(1));
        assert_eq!(launch_target_index(&[("main", false), ("window-a", false)]), Some(0));
        // The hidden PDF renderer sorts before `window-*` and must be skipped,
        // even if it reports focus.
        assert_eq!(launch_target_index(&[("pdf-render-1", false), ("window-a", false)]), Some(1));
        assert_eq!(launch_target_index(&[("pdf-render-1", true), ("window-a", false)]), Some(1));
        assert_eq!(launch_target_index(&[("pdf-render-1", false)]), None);
        assert_eq!(launch_target_index(&[]), None);
    }
}

#[cfg(all(test, desktop))]
mod navigation_policy_tests {
    use super::navigation_allowed;
    use tauri::Url;

    #[test]
    fn only_shell_reader_and_local_handoff_urls_may_navigate() {
        let is_reader = |origin: &str| origin == "http://abc-123.localhost:49860";
        for allowed in [
            "tauri://localhost/index.html",
            "about:blank",
            "about:srcdoc",
            "http://tauri.localhost/",
            "https://tauri.localhost/",
            "blob:http://tauri.localhost/export",
            "blob:http://abc-123.localhost:49860/export",
            "http://abc-123.localhost:49860/index",
        ] {
            assert!(
                navigation_allowed(&Url::parse(allowed).unwrap(), is_reader),
                "{allowed}"
            );
        }
        let development_server = Url::parse("http://127.0.0.1:1420/").unwrap();
        assert_eq!(
            navigation_allowed(&development_server, is_reader),
            cfg!(debug_assertions),
            "the development server is available only in debug builds"
        );
        for denied in [
            "https://example.com/",
            "http://tracker.example/?q=1",
            "http://127.0.0.1:8080/",
            "http://evil.localhost:1234/",
            "http://abc-123.localhost:9999/",
            "blob:https://example.com/export",
            "data:text/html,<script>fetch('https://example.com')</script>",
            "file:///etc/passwd",
            "ftp://example.com/",
        ] {
            assert!(
                !navigation_allowed(&Url::parse(denied).unwrap(), is_reader),
                "{denied}"
            );
        }
    }
}
