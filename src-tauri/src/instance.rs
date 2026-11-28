// SPDX-License-Identifier: GPL-3.0-or-later
//! Profile-scoped rendezvous between processes that share one data directory.
//!
//! `tauri-plugin-single-instance` keys on the bundle identifier, so a dev build
//! and a bundled build can both start against the same profile. Ownership is
//! really the profile's `store.lock`; this module gives whoever holds it a
//! private endpoint next to the lock so a later process can ask the owner to
//! come forward instead of aborting in Tauri's setup hook.
//!
//! Endpoint: `<profile>/instance/rendezvous.sock`, a Unix domain socket in a
//! directory owned by the user and forced to 0700, with the socket at 0600.
//! Each connection carries one newline-delimited JSON request and one reply.
//! Peer credentials must match the profile owner's uid; the directory and file
//! permissions are the first line of authentication, `getpeereid`/`SO_PEERCRED`
//! the second. Only the `activate` action exists. Its launch payload goes
//! through the same `parse_options` and `validate_secondary_launch` path as any
//! other launch, and payloads that would end the owner (`--help`, `--version`)
//! are refused.
//!
//! The `instance` directory is runtime IPC, not user state: it holds no durable
//! data, and a stale socket file is replaced by the next process that holds the
//! profile lock. The lock itself remains the single source of ownership, so a
//! crashed owner releases the profile without this endpoint's help.

#![forbid(unsafe_code)]

#[cfg(unix)]
pub(crate) mod socket;

use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

/// The only protocol revision this build speaks.
#[cfg(unix)]
const PROTOCOL_VERSION: u32 = 1;
/// Bound on one request line, including its newline.
#[cfg(unix)]
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Bound on forwarded arguments; real launches carry one executable and files.
#[cfg(unix)]
const MAX_ARGV: usize = 64;
/// How long a second instance waits for the owner's acknowledgement. The owner
/// may still be finishing startup, so this is deliberately generous.
#[cfg(unix)]
const ACTIVATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(unix)]
const SOCKET_DIR: &str = "instance";
#[cfg(unix)]
const SOCKET_FILE: &str = "rendezvous.sock";

/// The directory that holds the rendezvous socket.
#[cfg(unix)]
pub(crate) fn socket_dir(data_dir: &Path) -> PathBuf {
    if crate::sandbox::is_sandboxed() {
        // The container profile path alone exceeds the local socket limit
        // below. The container's own temporary directory is private to this
        // app and much shorter; name the profile by a digest of its resolved
        // path so separate profiles keep separate sockets and two spellings of
        // one profile share one.
        use sha2::Digest;
        let resolved = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
        let digest = sha2::Sha256::digest(resolved.as_os_str().as_encoded_bytes());
        let name: String = digest[..6].iter().map(|byte| format!("{byte:02x}")).collect();
        return std::env::temp_dir().join(format!("kw{name}"));
    }
    data_dir.join(SOCKET_DIR)
}

/// The rendezvous socket for one profile, next to `store.lock` (in the
/// sandbox, in the short per-profile temporary directory above).
#[cfg(unix)]
pub(crate) fn socket_path(data_dir: &Path) -> PathBuf {
    socket_dir(data_dir).join(if crate::sandbox::is_sandboxed() { "r" } else { SOCKET_FILE })
}

/// What a second instance does when the profile lock is already held.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BusyDecision {
    /// The owner acknowledged and is coming forward; exit quietly.
    Activated,
    /// No reachable owner: explain the conflict and exit without touching the profile.
    Dialog(String),
}

/// The recovery order is deliberate: prefer bringing the owner forward, and
/// only explain the conflict when that is impossible.
#[cfg(unix)]
pub(crate) fn busy_decision(activation: Result<(), String>) -> BusyDecision {
    match activation {
        Ok(()) => BusyDecision::Activated,
        Err(error) => BusyDecision::Dialog(error),
    }
}

/// The message shown when a second instance cannot reach the owner. It names
/// the exact profile, says another instance is running, and explains the
/// supported way to use a different library. It never suggests deleting the
/// lock file, which would let two processes corrupt one profile.
pub(crate) fn busy_dialog_message(data_dir: &Path, detail: &str) -> String {
    let detail = if detail.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nDetails: {detail}")
    };
    // Name the tool this platform actually has for ending a process.
    let process_tool = if cfg!(target_os = "macos") {
        "Activity Monitor"
    } else if cfg!(windows) {
        "Task Manager"
    } else {
        "your system monitor"
    };
    format!(
        "Another Kiwix instance is already running with this data directory:\n\n{}\n\nThat instance keeps the profile; this launch changed nothing. To use a separate library, launch Kiwix with --data-dir PATH (for example a new folder). If no Kiwix window is visible, quit the running instance from its window or {process_tool}; do not delete store.lock.{detail}",
        data_dir.display()
    )
}

/// Show the busy-profile explanation using a native dialog. The pre-Tauri path
/// has no `AppHandle`, so this goes through the platform's dialog helper.
///
/// The explanation is always written to stderr first. Whether a helper really
/// showed its dialog cannot be known for certain (zenity also exits 1 when it
/// cannot open a stale display), so the dialog is an addition, never the only
/// place the message goes.
#[cfg(unix)]
pub(crate) fn show_profile_busy(data_dir: &Path, detail: &str) {
    let message = busy_dialog_message(data_dir, detail);
    let title = "Kiwix — data directory in use";
    eprintln!("{title}: {message}");
    show_native_message(NativeMessageKind::Warning, title, &message);
}

/// Which icon a pre-Tauri native message carries.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeMessageKind {
    /// The launch changed nothing and another instance carries on.
    Warning,
    /// Kiwix cannot start.
    Error,
}

/// Show one message with an OK button before any Tauri runtime exists, and
/// wait for it to be dismissed. Returns whether a dialog was shown; the caller
/// owns the stderr fallback.
///
/// The text reaches the helper program as argument values, never through a
/// shell: a profile path in it cannot be read as a command or an option.
#[cfg(target_os = "macos")]
pub(crate) fn show_native_message(kind: NativeMessageKind, title: &str, message: &str) -> bool {
    let icon = match kind {
        NativeMessageKind::Warning => "caution",
        NativeMessageKind::Error => "stop",
    };
    let script = format!(
        "display dialog {} with title {} buttons {{\"OK\"}} default button \"OK\" with icon {icon}",
        apple_script_string(message),
        apple_script_string(title),
    );
    std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => {}
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// Whether a graphical session exists to show a dialog in. Without one the
/// helpers can only fail, so a terminal, service or SSH launch must not wait
/// on them.
#[cfg(all(unix, not(target_os = "macos")))]
fn has_display(display: Option<&std::ffi::OsStr>, wayland_display: Option<&std::ffi::OsStr>) -> bool {
    [display, wayland_display]
        .into_iter()
        .flatten()
        .any(|value| !value.is_empty())
}

/// The dialog helpers to try, in order, with their complete argument lists.
/// Every piece of caller text is the value of the option before it, and
/// `--no-markup` keeps zenity from parsing `&` or `<` in a path as Pango markup
/// (which blanks the whole message).
#[cfg(all(unix, not(target_os = "macos")))]
fn native_message_commands<'a>(
    kind: NativeMessageKind,
    title: &'a str,
    message: &'a str,
) -> [(&'static str, Vec<&'a str>); 2] {
    let (zenity_kind, kdialog_kind) = match kind {
        NativeMessageKind::Warning => ("--warning", "--msgbox"),
        NativeMessageKind::Error => ("--error", "--error"),
    };
    [
        ("zenity", vec![zenity_kind, "--no-wrap", "--no-markup", "--title", title, "--text", message]),
        ("kdialog", vec!["--title", title, kdialog_kind, message]),
    ]
}

/// Whether a helper that ran and ended with `code` showed its dialog.
///
/// Closing a dialog with Escape or its close button ends zenity and kdialog
/// with status 1. Reading that as "not shown" opened the same message a second
/// time in the next helper. Only zenity's 255, its status for a command line
/// it cannot use, means nothing appeared. kdialog has no status of its own for
/// that case (Qt's parser also exits 1), so every status it returns counts as
/// shown; it is the last helper, and its options here are long-standing. A
/// helper ended by a signal (`None`), as Qt does without a display, showed
/// nothing.
#[cfg(all(unix, not(target_os = "macos")))]
fn helper_showed_dialog(program: &str, code: Option<i32>) -> bool {
    match code {
        None => false,
        Some(255) => program != "zenity",
        Some(_) => true,
    }
}

/// Show one message with an OK button before any Tauri runtime exists, and
/// wait for it to be dismissed. Returns whether a dialog was shown; the caller
/// owns the stderr fallback.
///
/// The text reaches the helper program as argument values, never through a
/// shell: a profile path in it cannot be read as a command or an option.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn show_native_message(kind: NativeMessageKind, title: &str, message: &str) -> bool {
    if !has_display(
        std::env::var_os("DISPLAY").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
    ) {
        return false;
    }
    for (program, arguments) in native_message_commands(kind, title, message) {
        // A helper that is not installed cannot be started: try the next.
        let shown = std::process::Command::new(program)
            .args(arguments)
            .status()
            .is_ok_and(|status| helper_showed_dialog(program, status.code()));
        if shown {
            return true;
        }
    }
    false
}

#[cfg(unix)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    action: String,
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    cwd: String,
}

#[cfg(unix)]
impl Request {
    /// The only command this endpoint accepts. Everything else — another
    /// action, a different protocol, an argument that would end the owner, or a
    /// payload whose relative paths have no anchor — is refused before any
    /// window or archive code runs.
    fn into_launch_payload(self) -> Result<(Vec<String>, PathBuf), String> {
        if self.version != PROTOCOL_VERSION {
            return Err("Unsupported instance protocol version".into());
        }
        if self.action != "activate" {
            return Err("Unsupported instance action".into());
        }
        if self.argv.len() > MAX_ARGV {
            return Err("Too many launch arguments".into());
        }
        if self
            .argv
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version"))
        {
            return Err("Unsupported launch payload".into());
        }
        let cwd = PathBuf::from(&self.cwd);
        if !cwd.is_absolute() {
            return Err("Launch payload has no absolute working directory".into());
        }
        Ok((self.argv, cwd))
    }
}

#[cfg(unix)]
#[derive(serde::Serialize)]
struct Reply<'a> {
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedReply {
    status: String,
    #[serde(default)]
    message: Option<String>,
}

/// The owning instance's listening endpoint.
#[cfg(unix)]
pub(crate) struct Rendezvous {
    listener: std::os::unix::net::UnixListener,
    _entry: socket::SocketEntry,
    path: PathBuf,
}

#[cfg(unix)]
impl Rendezvous {
    /// Bind the profile's endpoint. Must only be called after the caller has
    /// claimed `store.lock`; that ownership is what makes replacing a stale
    /// socket safe.
    pub(crate) fn bind(data_dir: &Path) -> std::io::Result<Self> {
        let path = socket_path(data_dir);
        // sockaddr_un.sun_path is 104 bytes on macOS and 108 on Linux. Stay
        // under the smaller limit; a deeper profile keeps the dialog fallback.
        if std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).len() > 100 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Profile path is too long for a local socket ({}); second instances will explain the conflict instead",
                    path.display()
                ),
            ));
        }
        let (listener, entry) = socket::bind(&path)?;
        Ok(Self { listener, path, _entry: entry })
    }

    /// Start accepting after the app is fully initialized. Connections made
    /// while setup was still running wait in the listener backlog.
    pub(crate) fn spawn(self, app: tauri::AppHandle) {
        eprintln!("KIWIX_INSTANCE_READY {}", self.path.display());
        let spawned = std::thread::Builder::new()
            .name("kiwix-instance".into())
            .spawn(move || {
                let _entry = self._entry;
                for stream in self.listener.incoming() {
                    match stream {
                        Ok(stream) => handle_connection(&app, stream),
                        Err(error) => eprintln!("Kiwix instance: accept failed: {error}"),
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("Kiwix instance: listener thread unavailable: {error}");
        }
    }
}

/// Ask the instance that owns this profile to come forward, forwarding this
/// process's launch arguments through the same validation a normal launch uses.
#[cfg(unix)]
pub(crate) fn request_activation(
    data_dir: &Path,
    argv: Vec<String>,
    cwd: &Path,
) -> Result<(), String> {
    use std::io::{BufReader, Read, Write};

    let path = socket_path(data_dir);
    let mut stream = connect_with_grace(&path).map_err(|error| {
        format!("No instance is listening at {}: {error}", path.display())
    })?;
    if !peer_is_owner(&stream) {
        return Err("The activation endpoint belongs to another account".into());
    }
    let _ = stream.set_read_timeout(Some(ACTIVATION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(ACTIVATION_TIMEOUT));
    let request = Request {
        version: PROTOCOL_VERSION,
        action: "activate".into(),
        argv,
        cwd: cwd.to_string_lossy().into_owned(),
    };
    let mut line = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stream
        .write_all(&line)
        .map_err(|error| format!("Cannot send activation to {}: {error}", path.display()))?;
    let _ = stream.flush();
    let mut reply = String::new();
    BufReader::new(stream.take(4096))
        .read_to_string(&mut reply)
        .map_err(|error| format!("No activation reply from {}: {error}", path.display()))?;
    let reply: OwnedReply = serde_json::from_str(reply.trim())
        .map_err(|error| format!("Invalid activation reply from {}: {error}", path.display()))?;
    if reply.status == "ok" {
        Ok(())
    } else {
        Err(reply
            .message
            .unwrap_or_else(|| "The owning instance rejected the activation request".into()))
    }
}

/// A simultaneous launch can lose the lock a moment before the owner binds the
/// endpoint, and a starting owner accepts only after its windows exist. Retry
/// only errors that mean "not there yet" (including a live listener whose
/// accept queue is momentarily full); a real permission or path failure is
/// reported immediately.
#[cfg(unix)]
fn connect_with_grace(path: &Path) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::unix::net::UnixStream;

    let address = socket2::SockAddr::unix(path)?;
    let mut last = None;
    for _ in 0..20 {
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        match socket.connect_timeout(&address, std::time::Duration::from_millis(100)) {
            Ok(()) => return Ok(UnixStream::from(std::os::fd::OwnedFd::from(socket))),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::TimedOut
                ) =>
            {
                last = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "No profile rendezvous")
    }))
}

/// Called by a second instance that lost the profile lock: activate the owner
/// when possible, otherwise explain the conflict. Either way the caller exits 0
/// and the owner's profile is untouched.
#[cfg(unix)]
pub(crate) fn report_busy_profile(data_dir: &Path, argv: Vec<String>, cwd: PathBuf) {
    match busy_decision(request_activation(data_dir, argv, &cwd)) {
        BusyDecision::Activated => eprintln!(
            "Kiwix: another instance owns {}; asked it to come forward.",
            data_dir.display()
        ),
        BusyDecision::Dialog(detail) => {
            eprintln!(
                "Kiwix: another instance owns {} and could not be reached: {detail}",
                data_dir.display()
            );
            show_profile_busy(data_dir, &detail);
        }
    }
}

#[cfg(unix)]
fn handle_connection(app: &tauri::AppHandle, stream: std::os::unix::net::UnixStream) {
    let _ = stream.set_read_timeout(Some(ACTIVATION_TIMEOUT));
    let _ = stream.set_write_timeout(Some(ACTIVATION_TIMEOUT));
    if !peer_is_owner(&stream) {
        eprintln!("Kiwix instance: rejected an activation request from another user");
        reply(stream, Err("Unauthorized peer"));
        return;
    }
    let request = match read_request(&stream) {
        Ok(request) => request,
        Err(error) => {
            reply(stream, Err(&error));
            return;
        }
    };
    let (argv, cwd) = match request.into_launch_payload() {
        Ok(payload) => payload,
        Err(error) => {
            reply(stream, Err(&error));
            return;
        }
    };
    eprintln!("Kiwix instance: activation request received");
    crate::activate_instance(app);
    crate::forward_launch(app, argv, cwd);
    reply(stream, Ok(()));
}

#[cfg(unix)]
fn read_request(stream: &std::os::unix::net::UnixStream) -> Result<Request, String> {
    use std::io::{BufRead, BufReader, Read};

    let mut buffer = Vec::new();
    BufReader::new(stream.take((MAX_REQUEST_BYTES + 1) as u64))
        .read_until(b'\n', &mut buffer)
        .map_err(|error| format!("Cannot read request: {error}"))?;
    if buffer.len() > MAX_REQUEST_BYTES {
        return Err("Request exceeds the instance protocol limit".into());
    }
    while matches!(buffer.last(), Some(b'\n' | b'\r')) {
        buffer.pop();
    }
    if buffer.is_empty() {
        return Err("Empty request".into());
    }
    serde_json::from_slice(&buffer).map_err(|error| format!("Invalid request: {error}"))
}

#[cfg(unix)]
fn reply(mut stream: std::os::unix::net::UnixStream, result: Result<(), &str>) {
    use std::io::Write;

    let body = match result {
        Ok(()) => Reply { status: "ok", message: None },
        Err(message) => Reply { status: "error", message: Some(message) },
    };
    let mut line = serde_json::to_vec(&body)
        .unwrap_or_else(|_| br#"{"status":"error"}"#.to_vec());
    line.push(b'\n');
    let _ = stream.write_all(&line);
    let _ = stream.flush();
}

/// Socket credentials are checked against the profile owner's uid. Filesystem
/// permissions already restrict the endpoint to this user; this rejects a peer
/// that reached the socket some other way.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn peer_is_owner(stream: &std::os::unix::net::UnixStream) -> bool {
    nix::unistd::getpeereid(stream)
        .is_ok_and(|(uid, _)| uid.as_raw() == crate::fs_ext::effective_uid())
}

#[cfg(target_os = "linux")]
fn peer_is_owner(stream: &std::os::unix::net::UnixStream) -> bool {
    use std::os::fd::AsFd;

    rustix::net::sockopt::socket_peercred(stream.as_fd())
        .is_ok_and(|credentials| credentials.uid.as_raw() == crate::fs_ext::effective_uid())
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
fn peer_is_owner(_stream: &std::os::unix::net::UnixStream) -> bool {
    // Without a peer-credential API this platform never authenticates a
    // request, so it must never act on one.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn busy_profile_tries_activation_then_falls_back_to_the_dialog() {
        assert_eq!(busy_decision(Ok(())), BusyDecision::Activated);
        assert_eq!(
            busy_decision(Err("No such file or directory".into())),
            BusyDecision::Dialog("No such file or directory".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn instance_requests_are_limited_to_activate_with_a_validated_launch() {
        let request = |action: &str, argv: &[&str], cwd: &str| Request {
            version: PROTOCOL_VERSION,
            action: action.into(),
            argv: argv.iter().map(|argument| argument.to_string()).collect(),
            cwd: cwd.into(),
        };
        assert!(request("activate", &["kiwix", "/tmp/a.zim"], "/tmp")
            .into_launch_payload()
            .is_ok());
        assert!(request("activate", &[], "/tmp").into_launch_payload().is_ok());
        assert!(request("quit", &[], "/tmp").into_launch_payload().is_err());
        assert!(request("activate", &["kiwix", "--version"], "/tmp")
            .into_launch_payload()
            .is_err());
        assert!(request("activate", &["kiwix", "--help"], "/tmp")
            .into_launch_payload()
            .is_err());
        assert!(request("activate", &[], "relative").into_launch_payload().is_err());
        assert!(Request {
            version: PROTOCOL_VERSION + 1,
            ..request("activate", &[], "/tmp")
        }
        .into_launch_payload()
        .is_err());
    }

    #[test]
    fn busy_message_names_the_profile_and_how_to_choose_another() {
        let profile = Path::new("/tmp/kiwix-profile");
        let message = busy_dialog_message(profile, "No instance is listening");
        assert!(message.contains("/tmp/kiwix-profile"));
        assert!(message.contains("Another Kiwix instance is already running"));
        assert!(message.contains("--data-dir"));
        assert!(message.contains("do not delete store.lock"));
        assert!(!message.to_lowercase().contains("delete its lock file"));
        // Activity Monitor is a macOS application; Linux and Windows users
        // were being sent to a tool they do not have.
        assert_eq!(message.contains("Activity Monitor"), cfg!(target_os = "macos"), "{message}");
        assert_eq!(message.contains("Task Manager"), cfg!(windows), "{message}");
        assert_eq!(
            message.contains("your system monitor"),
            cfg!(not(any(target_os = "macos", windows))),
            "{message}"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn native_message_needs_a_display_and_passes_text_only_as_argument_values() {
        use std::ffi::OsStr;

        // A headless launch must not wait on a dialog helper.
        assert!(!has_display(None, None));
        assert!(!has_display(Some(OsStr::new("")), Some(OsStr::new(""))));
        assert!(has_display(Some(OsStr::new(":0")), None));
        assert!(has_display(None, Some(OsStr::new("wayland-0"))));

        // Text that looks like an option, markup or a shell command is still
        // exactly one argument, directly after the option that takes it.
        let title = "Kiwix — cannot start";
        let message = "--version; $(rm -rf ~) <b>&amp;</b>\n/home/jane doe/My Library/state.json";
        for kind in [NativeMessageKind::Warning, NativeMessageKind::Error] {
            for (program, arguments) in native_message_commands(kind, title, message) {
                let text = arguments.iter().position(|argument| *argument == message).unwrap();
                let caption = arguments.iter().position(|argument| *argument == title).unwrap();
                let (text_option, markup) = match program {
                    "zenity" => ("--text", true),
                    _ if kind == NativeMessageKind::Error => ("--error", false),
                    _ => ("--msgbox", false),
                };
                assert_eq!(arguments[text - 1], text_option, "{program}: {arguments:?}");
                assert_eq!(arguments[caption - 1], "--title", "{program}: {arguments:?}");
                assert_eq!(arguments.contains(&"--no-markup"), markup, "{program}: {arguments:?}");
                assert_eq!(arguments.iter().filter(|argument| argument.contains("jane doe")).count(), 1);
            }
        }
        assert_eq!(
            native_message_commands(NativeMessageKind::Error, title, message)[0].1[0],
            "--error"
        );
        assert_eq!(
            native_message_commands(NativeMessageKind::Warning, title, message)[0].1[0],
            "--warning"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_dismissed_dialog_counts_as_shown_so_the_message_is_not_repeated() {
        for program in ["zenity", "kdialog"] {
            // OK, then Escape or the close button, then zenity's timeout.
            for code in [0, 1, 5] {
                assert!(helper_showed_dialog(program, Some(code)), "{program} {code}");
            }
            // Ended by a signal: nothing was shown.
            assert!(!helper_showed_dialog(program, None), "{program}");
        }
        // zenity could not use its command line: the next helper must run.
        assert!(!helper_showed_dialog("zenity", Some(255)));
        assert!(helper_showed_dialog("kdialog", Some(255)));
    }

    #[cfg(unix)]
    #[test]
    fn rendezvous_endpoint_sits_beside_the_lock_and_is_private() {
        use std::os::unix::fs::PermissionsExt;

        // A short path keeps the socket under sockaddr_un's limit.
        let root = Path::new("/tmp").join(format!(
            "kiwix-inst-{}-{}",
            std::process::id(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            socket_path(&root),
            root.join(SOCKET_DIR).join(SOCKET_FILE)
        );
        assert_eq!(socket_path(&root).parent().unwrap().parent().unwrap(), root);

        let rendezvous = Rendezvous::bind(&root).unwrap();
        let socket = socket_path(&root);
        let directory_mode = std::fs::metadata(socket_dir(&root))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let socket_mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(socket_mode, 0o600);
        let outgoing = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        let (incoming, _) = rendezvous.listener.accept().unwrap();
        let supports_peer_credentials = cfg!(any(
            target_os = "macos", target_os = "ios", target_os = "linux", target_os = "freebsd",
            target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"
        ));
        assert_eq!(peer_is_owner(&outgoing), supports_peer_credentials);
        assert_eq!(peer_is_owner(&incoming), supports_peer_credentials);
        let file: std::os::fd::OwnedFd = std::fs::File::open(&root).unwrap().into();
        assert!(!peer_is_owner(&std::os::unix::net::UnixStream::from(file)));
        drop((outgoing, incoming));
        assert!(Rendezvous::bind(&root).is_err(), "must preserve a live listener");
        drop(rendezvous);

        assert!(!socket.exists());
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        // The lock owner crashed: the leftover socket must be
        // replaced, not treated as a live listener.
        let rebound = Rendezvous::bind(&root).unwrap();
        // A replacement made by the user must survive the earlier owner's cleanup.
        std::fs::rename(&socket, root.join("old-socket")).unwrap();
        std::fs::write(&socket, b"user file").unwrap();
        drop(rebound);
        assert!(Rendezvous::bind(&root).is_err());
        assert_eq!(std::fs::read(&socket).unwrap(), b"user file");
        std::fs::remove_file(&socket).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), &socket).unwrap();
        assert!(Rendezvous::bind(&root).is_err());
        assert!(socket.is_symlink());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
