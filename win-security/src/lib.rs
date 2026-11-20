//! Windows identity, security-descriptor and named-pipe checks shared by the
//! app's agent connector, its private reading-sync identity and the
//! `kiwix-mcp` relay.
//!
//! Callers pass borrowed handles (`&impl AsRawHandle`) so no crate here has to
//! know about tokio, cap-std or the app package. Every buffer this module
//! allocates through Win32 is freed exactly once on every path: tokens and
//! processes are owned by `std::os::windows::io::OwnedHandle`, `LocalFree`
//! allocations by `LocalAllocation`, and the SID inside `UserSid` points into
//! owned, never-reallocated storage.
#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use windows_sys::core::{BOOL, PWSTR};
use windows_sys::Win32::Foundation::{LocalFree, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo,
    SE_FILE_OBJECT, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetTokenInformation, IsValidSid,
    IsWellKnownSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{ReOpenFile, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};
use windows_sys::Win32::System::Pipes::{
    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_NOWAIT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Which security check failed. `Error::code` carries the native code when one
/// exists; `Display` prints the operating system's text for those and the
/// fixed sentence for policy refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Token,
    TokenTooLarge,
    SidText,
    Descriptor,
    Dacl,
    Owner,
    AceType,
    AceBounds,
    AceSid,
    Process,
    Image,
    Pipe,
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    code: Option<u32>,
    message: &'static str,
}

impl Error {
    fn fixed(kind: ErrorKind, message: &'static str) -> Self {
        Self { kind, code: None, message }
    }

    fn last(kind: ErrorKind, message: &'static str) -> Self {
        Self { kind, code: std::io::Error::last_os_error().raw_os_error().map(|code| code as u32), message }
    }

    fn status(kind: ErrorKind, code: u32, message: &'static str) -> Self {
        Self { kind, code: Some(code), message }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The native error code, when the failure came from a Win32 call.
    pub fn code(&self) -> Option<u32> {
        self.code
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code {
            Some(code) => write!(formatter, "{}", std::io::Error::from_raw_os_error(code as i32)),
            None => formatter.write_str(self.message),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Owns the `TOKEN_USER` buffer and therefore the SID embedded in it.
///
/// Not `Clone`: the heap allocation is never moved or mutated after
/// construction, so the SID pointer returned by `sid` stays valid for as long
/// as the value lives.
pub struct UserSid {
    storage: Vec<usize>,
}

impl UserSid {
    /// The current process's token user. `GetCurrentProcess` is only used as a
    /// read-only pseudo-handle and is never closed.
    pub fn current() -> Result<Self> {
        Self::open_token(None)
    }

    /// The token user of a caller-owned process handle. Never closes `process`.
    pub fn from_process(process: &impl AsRawHandle) -> Result<Self> {
        Self::open_token(Some(process.as_raw_handle()))
    }

    fn open_token(process: Option<HANDLE>) -> Result<Self> {
        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: `process` is either the caller's borrowed live handle or the
        // process-lifetime pseudo-handle from `GetCurrentProcess`; the token
        // written into `raw` is adopted by `OwnedHandle` below and closed
        // exactly once. The size probe writes only `required`; the fill writes
        // at most `required` bytes into `storage`, which is `usize`-aligned,
        // holds at least `required` zeroed bytes and is not reallocated while
        // the live token is queried.
        unsafe {
            let process = match process {
                Some(process) => process,
                None => GetCurrentProcess(),
            };
            if OpenProcessToken(process, TOKEN_QUERY, &mut raw) == 0 {
                return Err(Error::last(ErrorKind::Token, "Cannot read a process identity"));
            }
            let token = OwnedHandle::from_raw_handle(raw);
            let mut required = 0u32;
            let _ = GetTokenInformation(token.as_raw_handle(), TokenUser, std::ptr::null_mut(), 0, &mut required);
            if required == 0 || required > 64 * 1024 {
                return Err(Error::fixed(ErrorKind::TokenTooLarge, "could not determine the Windows user SID"));
            }
            let mut storage = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
            if GetTokenInformation(token.as_raw_handle(), TokenUser, storage.as_mut_ptr().cast(), required, &mut required)
                == 0
            {
                return Err(Error::last(ErrorKind::Token, "Cannot read a process identity"));
            }
            Ok(Self { storage })
        }
    }

    /// SAFETY contract for callers: `storage` owns a `TOKEN_USER` written by
    /// `GetTokenInformation`; it is `usize`-aligned and never reallocated, so
    /// the SID points into live owned memory.
    fn sid(&self) -> PSID {
        // SAFETY: see the method contract; `self.storage` is at least as large
        // as the `TOKEN_USER` the API wrote and outlives this borrow.
        unsafe { (*(self.storage.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }

    /// The SID as `S-1-…` text, copied before the native allocation is freed.
    pub fn as_string(&self) -> Result<String> {
        let mut text: PWSTR = std::ptr::null_mut();
        // SAFETY: `sid` points into `self.storage`, which outlives this call;
        // the API allocates a NUL-terminated string in the live local `text`.
        // `_allocation` owns that allocation until after the copy, and the scan
        // reads only up to the NUL the API guarantees.
        unsafe {
            if ConvertSidToStringSidW(self.sid(), &mut text) == 0 {
                return Err(Error::last(ErrorKind::SidText, "Cannot read the current user"));
            }
            let _allocation = LocalAllocation(text.cast());
            let mut length = 0usize;
            while *text.add(length) != 0 {
                length += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(text, length))
                .map_err(|_| Error::fixed(ErrorKind::SidText, "Cannot read the current user"))
        }
    }

    /// `EqualSid` over two live buffers. No SID pointer outlives its owner.
    pub fn equals(&self, other: &Self) -> bool {
        // SAFETY: both SIDs point into `Vec` storage owned by `self` and
        // `other`, which outlive the comparison and are not modified during it.
        unsafe { EqualSid(self.sid(), other.sid()) != 0 }
    }
}

/// A process handle opened with `PROCESS_QUERY_LIMITED_INFORMATION`.
pub struct ProcessHandle(OwnedHandle);

impl ProcessHandle {
    pub fn open_query(pid: u32) -> Result<Self> {
        // SAFETY: `pid` is an OS-provided value; the returned handle is
        // adopted by `OwnedHandle` exactly once and closed by std.
        unsafe {
            let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if raw.is_null() {
                return Err(Error::last(ErrorKind::Process, "Cannot open the process"));
            }
            Ok(Self(OwnedHandle::from_raw_handle(raw)))
        }
    }

    /// The process token's user, read through the borrowed handle.
    pub fn user(&self) -> Result<UserSid> {
        UserSid::from_process(&self.0)
    }

    /// The process image path (`PROCESS_NAME_WIN32`).
    pub fn image_path(&self) -> Result<PathBuf> {
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` is a live initialized `u16` array and `length` its
        // capacity in characters; the call writes at most that many and
        // updates `length` to the prefix written.
        unsafe {
            if QueryFullProcessImageNameW(self.0.as_raw_handle(), PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut length)
                == 0
            {
                return Err(Error::last(ErrorKind::Image, "Cannot identify the Kiwix process"));
            }
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize])))
    }
}

/// Which end of a named pipe to ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipePeer {
    Client,
    Server,
}

/// The client or server process ID of a connected pipe instance.
pub fn peer_process_id(pipe: &impl AsRawHandle, peer: PipePeer) -> Result<u32> {
    let mut pid = 0u32;
    // SAFETY: `pipe` is a live connected handle borrowed by the caller and
    // `pid` is a writable live local; no handle is created or closed.
    let ok = unsafe {
        match peer {
            PipePeer::Client => GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid),
            PipePeer::Server => GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid),
        }
    };
    if ok == 0 || pid == 0 {
        return Err(Error::last(ErrorKind::Pipe, "Cannot identify the pipe peer"));
    }
    Ok(pid)
}

/// Whether a kernel object's owner equals `user`. Any API failure is `false`,
/// so callers fail closed.
pub fn handle_owned_by(handle: &impl AsRawHandle, user: &UserSid) -> bool {
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `handle` is a live borrowed handle; `owner` and `descriptor` are
    // writable live locals. `_allocation` keeps the returned descriptor alive
    // until after `EqualSid`, and `user` outlives the comparison.
    unsafe {
        let status = GetSecurityInfo(
            handle.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        );
        let _allocation = LocalAllocation(descriptor);
        status == 0 && !owner.is_null() && EqualSid(owner, user.sid()) != 0
    }
}

/// Wait for a pipe to become available. The result is ignored, as the caller's
/// next open decides what to do.
pub fn wait_named_pipe(name: &str, milliseconds: u32) {
    let name: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `name` is a NUL-terminated UTF-16 buffer that stays alive across
    // the call; the return value is intentionally unused.
    unsafe {
        let _ = WaitNamedPipeW(name.as_ptr(), milliseconds);
    }
}

/// Switch a connected pipe to non-blocking mode.
pub fn set_nowait(pipe: &impl AsRawHandle) -> Result<()> {
    // SAFETY: `pipe` is a live borrowed handle, the mode is a constant and the
    // optional timeout/collect-data pointers are null, which the API allows.
    if unsafe {
        SetNamedPipeHandleState(pipe.as_raw_handle(), &PIPE_NOWAIT, std::ptr::null(), std::ptr::null())
    } == 0
    {
        return Err(Error::last(ErrorKind::Pipe, "Cannot configure the Kiwix connection"));
    }
    Ok(())
}

/// Owns one pointer that must be released with `LocalFree`.
struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: `self.0` is either null (ignored) or an allocation that
        // `ConvertStringSecurityDescriptorToSecurityDescriptorW`,
        // `ConvertSidToStringSidW` or `GetSecurityInfo` requires `LocalFree`
        // for. It is freed exactly once here and never used afterwards.
        if !self.0.is_null() {
            unsafe {
                let _ = LocalFree(self.0);
            }
        }
    }
}

/// Owns a `LocalAlloc`-backed security descriptor. Immutable and never cloned.
/// `raw` is borrowed and valid only until the value is dropped.
pub struct SecurityDescriptor {
    allocation: LocalAllocation,
}

impl SecurityDescriptor {
    /// Parse an SDDL string into a self-contained descriptor.
    pub fn from_sddl(sddl: &str) -> Result<Self> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut raw: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and alive across the call; the API
        // allocates a self-contained descriptor and stores it in the live
        // local `raw`, owned by `Self` from here on.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut raw, std::ptr::null_mut())
                == 0
            {
                return Err(Error::last(ErrorKind::Descriptor, "Cannot build a Windows security descriptor"));
            }
        }
        Ok(Self { allocation: LocalAllocation(raw) })
    }

    /// Pipe DACL: owned by the current user, full access for that user only,
    /// with a protected DACL. Relays check the owner.
    pub fn current_user_only(user: &UserSid) -> Result<Self> {
        let sid = user.as_string()?;
        Self::from_sddl(&format!("O:{sid}D:P(A;;GA;;;{sid})"))
    }

    /// File DACL: full access for the current user only, with a protected
    /// DACL. Directories inherit the entry; files do not.
    pub fn private_acl(user: &UserSid, directory: bool) -> Result<Self> {
        let sid = user.as_string()?;
        let inheritance = if directory { "OICI" } else { "" };
        Self::from_sddl(&format!("D:P(A;{inheritance};FA;;;{sid})"))
    }

    /// Borrowed native descriptor. Valid only while `self` is alive.
    pub fn raw(&self) -> *mut c_void {
        self.allocation.0
    }
}

// SAFETY: the allocation is process-wide, immutable after construction, has no
// thread affinity, and `LocalFree` may run on any thread. No `&mut` path
// exists; the only reader is a synchronous `CreateNamedPipeW` copy.
unsafe impl Send for SecurityDescriptor {}
// SAFETY: shared reads of the same immutable descriptor are sound for the same
// reason.
unsafe impl Sync for SecurityDescriptor {}

/// Apply a descriptor's DACL to a file or kernel object. With `owner`, first
/// tries to set owner + protected DACL; a failure (for example a handle
/// without `WRITE_OWNER`) falls back to the protected DACL alone, and the
/// caller's later check rejects an unexpected owner.
pub fn set_dacl(handle: &impl AsRawHandle, descriptor: &SecurityDescriptor, owner: Option<&UserSid>) -> Result<()> {
    let mut present: BOOL = 0;
    let mut defaulted: BOOL = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `descriptor` is a live owned allocation; `present`, `defaulted`
    // and `dacl` are writable live locals and the DACL pointer stays inside
    // that allocation. `handle` is a live borrowed handle and `owner` outlives
    // the calls, so every pointer passed to `SetSecurityInfo` is valid.
    unsafe {
        if GetSecurityDescriptorDacl(descriptor.raw(), &mut present, &mut dacl, &mut defaulted) == 0 {
            return Err(Error::last(ErrorKind::Dacl, "Windows private DACL is unavailable"));
        }
        if present == 0 || dacl.is_null() {
            return Err(Error::fixed(ErrorKind::Dacl, "Windows private DACL is unavailable"));
        }
        let handle = handle.as_raw_handle();
        if let Some(owner) = owner {
            let status = SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                owner.sid(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            );
            if status == 0 {
                return Ok(());
            }
        }
        let status = SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        );
        if status != 0 {
            return Err(Error::status(ErrorKind::Dacl, status, "Windows private DACL is unavailable"));
        }
    }
    Ok(())
}

/// Build the current user's private DACL, apply it, then verify it.
pub fn restrict_handle(handle: &impl AsRawHandle, directory: bool) -> Result<()> {
    let user = UserSid::current()?;
    let descriptor = SecurityDescriptor::private_acl(&user, directory)?;
    set_dacl(handle, &descriptor, Some(&user))?;
    check_private_handle(handle, false, directory)
}

/// Restrict a file to the current user. A handle without `WRITE_DAC` is
/// retried on the same object (never a name) with a broader access mask; the
/// first error is reported if the retry also fails.
pub fn restrict_file(file: &impl AsRawHandle) -> Result<()> {
    match restrict_handle(file, false) {
        Ok(()) => Ok(()),
        Err(first) => {
            let owner = reopen_for_owner(file)?;
            restrict_handle(&owner, false).map_err(|_| first)
        }
    }
}

/// Verify the DACL policy. A profile directory may also grant the two standard
/// OS administrators; every other object must be current-user-only full
/// control with the expected inheritance flags.
pub fn check_private_handle(handle: &impl AsRawHandle, profile: bool, directory: bool) -> Result<()> {
    let user = UserSid::current()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `handle` is a live borrowed handle; `owner`, `dacl` and
    // `descriptor` are writable live locals and the API allocates the
    // descriptor recorded there. `_allocation` keeps that allocation alive
    // while `owner`, `dacl` and every `GetAce` result are read, and `user`
    // outlives the comparisons.
    unsafe {
        let status = GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        );
        if status != 0 {
            return Err(Error::status(ErrorKind::Owner, status, "Cannot read the Windows security descriptor"));
        }
        let _allocation = LocalAllocation(descriptor);
        if owner.is_null() || EqualSid(owner, user.sid()) == 0 || dacl.is_null() || (*dacl).AceCount == 0 {
            return Err(Error::fixed(
                ErrorKind::Owner,
                "Windows identity storage must be owned by this user with a restrictive DACL",
            ));
        }
        if !profile {
            let mut control = 0u16;
            let mut revision = 0u32;
            if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
                return Err(Error::last(ErrorKind::Dacl, "Cannot read the Windows DACL control flags"));
            }
            if control & SE_DACL_PROTECTED == 0 || (*dacl).AceCount != 1 {
                return Err(Error::fixed(
                    ErrorKind::Dacl,
                    "Windows identity storage must have a protected, single-user DACL",
                ));
            }
        }
        for index in 0..u32::from((*dacl).AceCount) {
            let mut raw = std::ptr::null_mut();
            if GetAce(dacl, index, &mut raw) == 0 {
                return Err(Error::last(ErrorKind::AceBounds, "Windows identity storage has an invalid ACE"));
            }
            if raw.is_null() {
                return Err(Error::fixed(ErrorKind::AceBounds, "Windows identity storage has an invalid ACE"));
            }
            let header = &*raw.cast::<ACE_HEADER>();
            let acl_start = dacl as usize;
            let acl_end = acl_start
                .checked_add(usize::from((*dacl).AclSize))
                .ok_or_else(|| Error::fixed(ErrorKind::AceBounds, "Windows identity ACL size is invalid"))?;
            let ace_start = raw as usize;
            let ace_end = ace_start
                .checked_add(usize::from(header.AceSize))
                .ok_or_else(|| Error::fixed(ErrorKind::AceBounds, "Windows identity ACE size is invalid"))?;
            if ace_start < acl_start || ace_end > acl_end {
                return Err(Error::fixed(ErrorKind::AceBounds, "Windows identity ACE exceeds its ACL"));
            }
            // Reject callback/object/deny ACEs rather than interpreting arbitrary ACLs.
            if header.AceType != 0 {
                return Err(Error::fixed(ErrorKind::AceType, "Windows identity storage has an unexpected ACE type"));
            }
            let sid_offset = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
            if usize::from(header.AceSize) < sid_offset + 8 {
                return Err(Error::fixed(ErrorKind::AceSid, "Windows identity ACE is too short for a SID"));
            }
            let sid_bytes = raw.cast::<u8>().add(sid_offset);
            let subauthorities = usize::from(*sid_bytes.add(1));
            let sid_length = 8usize
                .checked_add(
                    subauthorities
                        .checked_mul(4)
                        .ok_or_else(|| Error::fixed(ErrorKind::AceSid, "Windows identity SID is too large"))?,
                )
                .ok_or_else(|| Error::fixed(ErrorKind::AceSid, "Windows identity SID is too large"))?;
            if sid_offset.checked_add(sid_length).is_none_or(|end| end > usize::from(header.AceSize)) {
                return Err(Error::fixed(ErrorKind::AceSid, "Windows identity SID exceeds its ACE"));
            }
            let ace = &*raw.cast::<ACCESS_ALLOWED_ACE>();
            let sid: PSID = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
            if IsValidSid(sid) == 0 {
                return Err(Error::fixed(ErrorKind::AceSid, "Windows identity ACE contains an invalid SID"));
            }
            let own = EqualSid(sid, user.sid()) != 0;
            if profile {
                // Standard private Windows profiles also grant the two OS administrators.
                if !own
                    && IsWellKnownSid(sid, WinLocalSystemSid) == 0
                    && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0
                {
                    return Err(Error::fixed(
                        ErrorKind::Owner,
                        "Windows profile grants access to another user or group",
                    ));
                }
            } else if !own || ace.Mask != 0x001f_01ff || ace.Header.AceFlags != if directory { 3 } else { 0 } {
                return Err(Error::fixed(
                    ErrorKind::Owner,
                    "Windows identity permissions are not current-user-only full control",
                ));
            }
        }
    }
    Ok(())
}

/// Reopen the same object (never a name that delete-sharing can replace) with
/// owner-write access, adopting the returned handle exactly once.
fn reopen_for_owner(file: &impl AsRawHandle) -> Result<OwnedHandle> {
    // SAFETY: `file` owns a live handle; the returned handle is checked for
    // null, then adopted exactly once by `OwnedHandle` and closed by std.
    unsafe {
        let raw = ReOpenFile(
            file.as_raw_handle(),
            0x000e_0000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        );
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(Error::last(ErrorKind::Owner, "Cannot reopen the identity file for its owner"));
        }
        Ok(OwnedHandle::from_raw_handle(raw))
    }
}
