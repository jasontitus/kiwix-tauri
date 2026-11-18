//! The relay's side of the same-user channel to the running app.
//!
//! Framing: a big-endian u32 length, then that many bytes of JSON. The relay
//! sends `{"hello":1,"dataDir":…}` first, then `{"id","tool","arguments"}`
//! requests; the app answers each with `{"id","ok"}` or `{"id","error"}`.
//!
//! The pipe name is predictable and lives in a namespace shared by every
//! account, and while Kiwix is closed or the connector is off nothing owns it.
//! So the relay authenticates the server before sending anything:
//! - the pipe object's owner must be this user (an owner cannot be set to
//!   another account without a restore privilege, unlike a recycled PID);
//! - the server process must run as this user and be the
//!   `kiwix-workbench.exe` installed next to this relay;
//! - and it connects at identification level only, so a server cannot act as
//!   the relay's user.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_FRAME: usize = 4 * 1024 * 1024;
pub const NOT_RUNNING: &str = "Kiwix is not running, or its agent connector is off. \
Open Kiwix and turn on Settings → Agent connector, then try again.";

/// Why a call failed: the channel broke (worth one reconnect), or the app
/// answered with an error for the agent.
#[derive(Debug)]
pub enum CallError {
    Lost,
    App(String),
}

pub fn write_frame(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"));
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    writer.write_all(&frame)?;
    writer.flush()
}

pub fn read_frame(reader: &mut impl Read) -> std::io::Result<Value> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// The pipe name suffix for a non-default profile: a short hash of its
/// canonical path. The app computes the same value from its own profile.
pub fn profile_suffix(profile: &Path) -> Result<String, String> {
    let canonical = std::fs::canonicalize(profile)
        .map_err(|_| "Cannot identify the Kiwix profile folder. Check --data-dir and try again.".to_string())?;
    let digest = chksum_hash_sha2_256::hash(canonical.to_string_lossy().to_lowercase().as_bytes());
    Ok(digest.to_hex_lowercase()[..16].to_owned())
}

/// Which profile's app this relay talks to: `--data-dir`, else the portable
/// profile beside this relay, else the default profile (`None`). A
/// `--data-dir` naming the default profile is the default profile, which the
/// app serves under the plain per-user name.
pub fn target_profile(data_dir: Option<&Path>) -> Result<Option<PathBuf>, String> {
    if let Some(path) = data_dir {
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        // Preserve an explicit --data-dir even when it names a default profile:
        // the host, macOS container and Flatpak defaults are distinct apps.
        #[cfg(unix)]
        return Ok(Some(path));
        #[cfg(not(unix))]
        {
            let is_default = directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench")
                .is_some_and(|dirs| same_file::is_same_file(&path, dirs.data_dir()).unwrap_or(false));
            return Ok((!is_default).then_some(path));
        }
    }
    // A relay inside Kiwix.app belongs to the sandboxed release app, which
    // ignores a portable marker beside the bundle. Do not select a profile the
    // app will never serve.
    #[cfg(all(unix, target_os = "macos"))]
    if unix::relay_is_bundled() {
        return Ok(None);
    }
    #[cfg(unix)]
    let root = unix::distribution_root();
    #[cfg(not(unix))]
    let root = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(root.join(".portable").is_file().then(|| root.join("data")))
}

#[cfg(windows)]
type Channel = windows::Pipe;
#[cfg(unix)]
type Channel = std::os::unix::net::UnixStream;

pub struct Connection {
    channel: Channel,
    next_id: u64,
}

impl Connection {
    pub fn open(profile: Option<&Path>) -> Result<Self, String> {
        #[cfg(windows)]
        let mut channel = windows::connect(profile.map(profile_suffix).transpose()?.as_deref())?;
        #[cfg(unix)]
        let mut channel = unix::connect(profile)?;
        #[cfg(unix)]
        {
            // Longer than both the app's permit wait and its native work budget.
            // An app that is suspended or wedged must not block the relay forever.
            let timeout = Some(std::time::Duration::from_secs(30));
            channel.set_read_timeout(timeout).and_then(|()| channel.set_write_timeout(timeout))
                .map_err(|_| "Cannot configure the Kiwix connection".to_string())?;
        }
        let hello = json!({"hello": 1, "dataDir": profile.map(|path| path.to_string_lossy().into_owned())});
        write_frame(&mut channel, &hello).map_err(|_| NOT_RUNNING.to_string())?;
        let reply = read_frame(&mut channel).map_err(|_| NOT_RUNNING.to_string())?;
        if let Some(error) = reply.get("error").and_then(Value::as_str) {
            return Err(error.chars().take(500).collect());
        }
        if reply["ok"]["version"].as_str().is_none_or(str::is_empty) {
            return Err("Unexpected greeting from Kiwix. Check the installed connector version.".into());
        }
        Ok(Self { channel, next_id: 1 })
    }

    pub fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        write_frame(&mut self.channel, &json!({"id": id, "tool": tool, "arguments": arguments}))
            .map_err(|_| CallError::Lost)?;
        let mut reply = read_frame(&mut self.channel).map_err(|_| CallError::Lost)?;
        if reply.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(CallError::Lost);
        }
        if let Some(error) = reply.get("error").and_then(Value::as_str) {
            return Err(CallError::App(error.chars().take(2000).collect()));
        }
        reply.get_mut("ok").map(Value::take).ok_or_else(|| CallError::App("Kiwix returned an unexpected result".into()))
    }
}

#[cfg(windows)]
mod windows {
    use super::NOT_RUNNING;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, HANDLE};
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, TokenUser, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_NOWAIT};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// `SecurityIdentification << 16`; std adds `SECURITY_SQOS_PRESENT`.
    const SECURITY_IDENTIFICATION: u32 = 1 << 16;
    const REFUSED: &str = "Refused a connection that did not come from Kiwix. Restart Kiwix and try again.";

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The process token's user, as an owned TOKEN_USER buffer.
    fn token_user(process: HANDLE) -> Result<Vec<u64>, String> {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
                return Err("Cannot read a process identity".into());
            }
            let token = Handle(token);
            let mut length = 0u32;
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut length);
            if length == 0 || length > 4096 {
                return Err("Cannot read a process identity".into());
            }
            let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
            if GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), length, &mut length) == 0 {
                return Err("Cannot read a process identity".into());
            }
            Ok(buffer)
        }
    }

    fn sid(buffer: &[u64]) -> PSID {
        unsafe { (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    pub fn current_user_sid() -> Result<String, String> {
        let user = token_user(unsafe { GetCurrentProcess() })?;
        unsafe {
            let mut text: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(sid(&user), &mut text) == 0 {
                return Err("Cannot read the current user".into());
            }
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let value = std::ffi::OsString::from_wide(std::slice::from_raw_parts(text, length));
            LocalFree(text.cast());
            Ok(value.to_string_lossy().into_owned())
        }
    }

    pub fn pipe_name(suffix: Option<&str>) -> Result<String, String> {
        let base = format!(r"\\.\pipe\kiwix-mcp-{}", current_user_sid()?);
        Ok(match suffix {
            Some(suffix) => format!("{base}-{suffix}"),
            None => base,
        })
    }

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn image_path(process: HANDLE) -> Result<PathBuf, String> {
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut length) } == 0 {
            return Err("Cannot identify the Kiwix process".into());
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize])))
    }

    /// The pipe object's owner must be this user.
    fn owned_by(pipe: HANDLE, own: &[u64]) -> bool {
        unsafe {
            let mut owner: PSID = std::ptr::null_mut();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let status = GetSecurityInfo(
                pipe,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            );
            let owned = status == 0 && !owner.is_null() && EqualSid(owner, sid(own)) != 0;
            if !descriptor.is_null() {
                LocalFree(descriptor);
            }
            owned
        }
    }

    /// Refuse any server that is not this user's Kiwix from this installation.
    fn verify_server(pipe: &std::fs::File) -> Result<(), String> {
        let handle = pipe.as_raw_handle() as HANDLE;
        let own = token_user(unsafe { GetCurrentProcess() })?;
        if !owned_by(handle, &own) {
            return Err(REFUSED.into());
        }
        let mut pid = 0u32;
        if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 || pid == 0 {
            return Err(REFUSED.into());
        }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(REFUSED.into());
        }
        let process = Handle(process);
        let server = token_user(process.0).map_err(|_| REFUSED.to_string())?;
        if unsafe { EqualSid(sid(&server), sid(&own)) } == 0 {
            return Err(REFUSED.into());
        }
        let expected = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("kiwix-workbench.exe");
        if !same_file::is_same_file(image_path(process.0).map_err(|_| REFUSED.to_string())?, expected).unwrap_or(false) {
            return Err(REFUSED.into());
        }
        Ok(())
    }

    pub struct Pipe(std::fs::File);

    // This stdio relay has one synchronous caller. Retry only while the
    // local byte pipe is empty (read) or full (write), with a finite wait.
    fn wait_for_progress(mut attempt: impl FnMut() -> io::Result<Option<usize>>) -> io::Result<usize> {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(count) = attempt()? { return Ok(count); }
            if Instant::now() >= deadline { return Err(io::ErrorKind::TimedOut.into()); }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            wait_for_progress(|| match self.0.read(buf) {
                Err(err) if err.raw_os_error() == Some(ERROR_NO_DATA as i32) => Ok(None),
                result => result.map(Some),
            })
        }
    }

    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            wait_for_progress(|| self.0.write(buf).map(|n| (n != 0 || buf.is_empty()).then_some(n)))
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    pub fn connect(suffix: Option<&str>) -> Result<Pipe, String> {
        let name = pipe_name(suffix)?;
        for _ in 0..3 {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(SECURITY_IDENTIFICATION)
                .open(&name);
            match opened {
                Ok(pipe) => {
                    verify_server(&pipe)?;
                    if unsafe { SetNamedPipeHandleState(pipe.as_raw_handle().cast(), &PIPE_NOWAIT,
                        std::ptr::null(), std::ptr::null()) } == 0 {
                        return Err("Cannot configure the Kiwix connection".into());
                    }
                    return Ok(Pipe(pipe));
                }
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    unsafe { WaitNamedPipeW(wide(&name).as_ptr(), 2000) };
                }
                Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {
                    return Err(NOT_RUNNING.into());
                }
                Err(_) => return Err(NOT_RUNNING.into()),
            }
        }
        Err("Kiwix is busy. Try again.".into())
    }
}

#[cfg(unix)]
mod unix {
    //! The app serves a Unix socket at `<profile>/instance/mcp.sock`, or
    //! `mcp-<profile hash>.sock` for a non-default profile. In the macOS App
    //! Sandbox the socket lives under the container's short temporary path
    //! instead. The relay verifies that the server runs as this user before
    //! sending anything.

    use super::{profile_suffix, NOT_RUNNING};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    fn default_profiles() -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(dirs) = directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench") {
            out.push(dirs.data_dir().to_path_buf());
        }
        #[cfg(target_os = "linux")]
        if let Some(home) = std::env::var_os("HOME") {
            let flatpak = PathBuf::from(home).join(".var/app/org.kiwix.workbench/data/kiwixworkbench");
            if !out.iter().any(|profile| profile == &flatpak) {
                out.push(flatpak);
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(home) = std::env::var_os("HOME") {
            let container = PathBuf::from(home)
                .join("Library/Containers/org.kiwix.workbench/Data/Library/Application Support/org.kiwix.KiwixWorkbench");
            if !out.iter().any(|profile| profile == &container) {
                out.push(container);
            }
        }
        out
    }

    pub(super) fn is_default_profile(profile: &Path) -> bool {
        default_profiles().iter().any(|candidate| same_file::is_same_file(candidate, profile).unwrap_or(false))
    }

    #[cfg(target_os = "macos")]
    fn is_sandbox_default(profile: &Path) -> bool {
        let Some(home) = std::env::var_os("HOME") else { return false };
        let container = PathBuf::from(home)
            .join("Library/Containers/org.kiwix.workbench/Data/Library/Application Support/org.kiwix.KiwixWorkbench");
        same_file::is_same_file(profile, container).unwrap_or(false)
    }

    /// The directory the distributed app was installed in, matching the app's
    /// portable-profile detection: beside the AppImage, or beside the `.app`.
    pub(super) fn distribution_root() -> PathBuf {
        #[cfg(target_os = "linux")]
        if let Some(image) = std::env::var_os("APPIMAGE") {
            let image = PathBuf::from(image);
            if image.is_absolute() {
                if let Some(parent) = image.parent() {
                    return parent.to_path_buf();
                }
            }
        }
        let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let parent = executable.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        #[cfg(target_os = "macos")]
        if parent.file_name().is_some_and(|name| name == "MacOS") {
            if let Some(contents) = parent.parent().filter(|path| path.file_name().is_some_and(|name| name == "Contents")) {
                if let Some(bundle) = contents.parent().filter(|path| path.extension().is_some_and(|extension| extension == "app")) {
                    return bundle.parent().map(Path::to_path_buf).unwrap_or(parent);
                }
            }
        }
        parent
    }

    #[cfg(target_os = "macos")]
    pub(super) fn relay_is_bundled() -> bool {
        std::env::current_exe().is_ok_and(|executable| {
            executable.to_string_lossy().contains(".app/Contents/MacOS/")
        })
    }

    #[cfg(target_os = "macos")]
    fn sandbox_candidate(name: &str, out: &mut Vec<PathBuf>) {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(
                PathBuf::from(home)
                    .join("Library/Containers/org.kiwix.workbench/Data/tmp/kiwix-mcp")
                    .join(name),
            );
        }
    }

    /// Every socket the app could have created for this target profile.
    pub(super) fn candidates(profile: Option<&Path>) -> Vec<PathBuf> {
        let mut out = Vec::new();
        match profile {
            Some(profile) => {
                if is_default_profile(profile) {
                    out.push(profile.join("instance").join("mcp.sock"));
                    #[cfg(target_os = "macos")]
                    if is_sandbox_default(profile) {
                        sandbox_candidate("mcp.sock", &mut out);
                    }
                } else {
                    let Ok(suffix) = profile_suffix(profile) else { return out };
                    let name = format!("mcp-{suffix}.sock");
                    out.push(profile.join("instance").join(&name));
                    #[cfg(target_os = "macos")]
                    sandbox_candidate(&name, &mut out);
                }
            }
            None => {
                // A relay inside Kiwix.app belongs to the sandboxed release app.
                // Never fall through to a separately running host build: the
                // hello has no dataDir, so the wrong app would accept.
                #[cfg(target_os = "macos")]
                if relay_is_bundled() {
                    sandbox_candidate("mcp.sock", &mut out);
                    return out;
                }
                for profile in default_profiles() {
                    out.push(profile.join("instance").join("mcp.sock"));
                }
                #[cfg(target_os = "macos")]
                sandbox_candidate("mcp.sock", &mut out);
            }
        }
        out
    }

    #[cfg(target_os = "linux")]
    fn peer_is_same_user(stream: &UnixStream) -> bool {
        rustix::net::sockopt::socket_peercred(stream)
            .is_ok_and(|credentials| credentials.uid == rustix::process::geteuid())
    }

    #[cfg(target_os = "macos")]
    fn peer_is_same_user(stream: &UnixStream) -> bool {
        nix::unistd::getpeereid(stream)
            .is_ok_and(|(uid, _)| uid == nix::unistd::geteuid())
    }

    #[cfg(test)]
    #[test]
    fn peer_lookup_accepts_connected_sockets_and_denies_a_failed_lookup() {
        let (left, right) = UnixStream::pair().unwrap();
        assert!(peer_is_same_user(&left));
        assert!(peer_is_same_user(&right));
        let directory: std::os::fd::OwnedFd = std::fs::File::open(".").unwrap().into();
        assert!(!peer_is_same_user(&UnixStream::from(directory)));
    }

    pub(super) fn connect(profile: Option<&Path>) -> Result<UnixStream, String> {
        for path in candidates(profile) {
            if !path.exists() {
                continue;
            }
            let Ok(address) = socket2::SockAddr::unix(&path) else { continue };
            let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)
                .map_err(|_| "Cannot open a local Kiwix connection".to_string())?;
            if socket.connect_timeout(&address, std::time::Duration::from_secs(2)).is_err() { continue; }
            let stream = UnixStream::from(std::os::fd::OwnedFd::from(socket));
            if peer_is_same_user(&stream) {
                return Ok(stream);
            }
        }
        Err(NOT_RUNNING.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_reject_oversize() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &json!({"id": 1, "ok": {"x": "é"}})).unwrap();
        assert_eq!(read_frame(&mut buffer.as_slice()).unwrap()["ok"]["x"], "é");
        let mut huge = ((MAX_FRAME + 1) as u32).to_be_bytes().to_vec();
        huge.extend_from_slice(b"{}");
        assert!(read_frame(&mut huge.as_slice()).is_err());
        assert!(read_frame(&mut [0u8, 0].as_slice()).is_err());
    }

    #[test]
    fn profile_suffix_is_stable_and_resolves_directory_aliases() {
        let dir = std::env::temp_dir();
        let one = profile_suffix(&dir).unwrap();
        assert_eq!(one.len(), 16);
        assert_eq!(one, profile_suffix(&dir.join(".")).unwrap());
        assert!(profile_suffix(Path::new("Z:/definitely/missing/profile")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn pipe_name_is_per_user_and_per_profile() {
        let name = windows::pipe_name(None).unwrap();
        assert!(name.starts_with(r"\\.\pipe\kiwix-mcp-S-1-"), "{name}");
        assert_eq!(windows::pipe_name(Some("ab")).unwrap(), format!("{name}-ab"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_candidates_follow_the_profile() {
        let dir = std::env::temp_dir();
        let suffix = profile_suffix(&dir).unwrap();
        let named = unix::candidates(Some(&dir));
        assert!(named.iter().any(|path| path.ends_with(format!("mcp-{suffix}.sock"))), "{named:?}");
        let defaults = unix::candidates(None);
        assert!(defaults.iter().any(|path| path.ends_with("mcp.sock")), "{defaults:?}");
        #[cfg(target_os = "macos")]
        assert!(defaults.iter().any(|path| path.to_string_lossy().contains("Library/Containers/org.kiwix.workbench/Data/tmp/kiwix-mcp")), "{defaults:?}");
    }
}
