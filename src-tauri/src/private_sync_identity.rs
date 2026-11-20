// SPDX-License-Identifier: GPL-3.0-or-later
//! Native-only installation identity; never put this object in Store state or an IPC response.
//! The 64-byte private file is X25519 secret || public, with no portable/profile metadata.
//! Failed/interrupted publication may retain a private, randomly named pending file. Such
//! files are never identities and are not removed through potentially substituted paths.
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, Metadata, OpenOptions};
use snow::{params::DHChoice, resolvers::{CryptoResolver, DefaultResolver}};
use std::{io::{Read, Write}, path::{Path, PathBuf}};
use zeroize::Zeroizing;

const DIRECTORY: &str = "private-reading-sync-v1";
const KEY_FILE: &str = "identity.x25519";
const KEY_LEN: usize = 32;
const FILE_LEN: u64 = 64;
const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// Intentionally has neither Debug, Clone nor Serialize. Only the public key is UI data.
pub struct Identity {
    secret: Zeroizing<Vec<u8>>,
    public: [u8; KEY_LEN],
}
impl Identity {
    pub fn public_hex(&self) -> String { hex::encode(self.public) }
    pub fn public_key(&self) -> &[u8] { &self.public }
    /// For the native Noise builder only; callers must not log or serialize this slice.
    pub fn secret_key(&self) -> &[u8] { self.secret.as_slice() }
}

fn err(error: impl std::fmt::Display) -> String { format!("Private reading sync identity: {error}") }
fn same(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

struct Directory {
    path: PathBuf,
    dir: Dir,
    metadata: Metadata,
}
impl Directory {
    fn open(path: &Path) -> Result<Self, String> {
        let dir = crate::storage::open_directory_nofollow(path).map_err(err)?;
        let metadata = dir.dir_metadata().map_err(err)?;
        Ok(Self { path: path.into(), dir, metadata })
    }
    fn unchanged(&self) -> Result<(), String> {
        let current = Self::open(&self.path)?;
        if !same(&self.metadata, &current.metadata) {
            return Err(err("profile directory was substituted"));
        }
        Ok(())
    }
    fn sync(&self) -> Result<(), String> {
        #[cfg(unix)]
        crate::storage::sync_cap_directory(&self.dir).map_err(err)?;
        // Windows has no unprivileged directory-fsync equivalent. The file itself
        // is flushed before exclusive, handle-relative publication; this is not a
        // POSIX directory-entry power-loss durability guarantee on Windows.
        Ok(())
    }
    fn private_child(&self) -> Result<Self, String> {
        self.unchanged()?;
        check_profile(&self.dir)?;
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        { use cap_std::fs::DirBuilderExt; builder.mode(0o700); }
        let created = match self.dir.create_dir_with(DIRECTORY, &builder) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(err(error)),
        };
        let dir = self.dir.open_dir_nofollow(DIRECTORY).map_err(err)?;
        #[cfg(windows)]
        if created { windows_acl::restrict_directory(&dir)?; }
        check_directory(&dir)?;
        let metadata = dir.dir_metadata().map_err(err)?;
        let result = Self { path: self.path.join(DIRECTORY), dir, metadata };
        self.unchanged()?;
        result.unchanged()?;
        if created { self.sync()?; }
        Ok(result)
    }
    fn file_unchanged(&self, name: &str, expected: &Metadata) -> Result<(), String> {
        let current = self.dir.symlink_metadata(name).map_err(err)?;
        check_file_metadata(&current)?;
        if !same(expected, &current) || current.len() != expected.len()
            || current.modified().map_err(err)? != expected.modified().map_err(err)? {
            return Err(err("key file was substituted or modified"));
        }
        self.unchanged()
    }
}

#[cfg(unix)]
fn check_profile(dir: &Dir) -> Result<(), String> {
    let metadata = dir.dir_metadata().map_err(err)?;
    if cap_std::fs::MetadataExt::uid(&metadata) != crate::fs_ext::effective_uid()
        || cap_std::fs::MetadataExt::mode(&metadata) & 0o022 != 0 {
        return Err(err("profile must be owned by this user and not writable by other users"));
    }
    Ok(())
}
#[cfg(unix)]
fn check_directory(dir: &Dir) -> Result<(), String> {
    let metadata = dir.dir_metadata().map_err(err)?;
    if !metadata.is_dir() || cap_std::fs::MetadataExt::uid(&metadata) != crate::fs_ext::effective_uid()
        || cap_std::fs::MetadataExt::mode(&metadata) & 0o7777 != 0o700 {
        return Err(err("identity directory must be owned by this user with permissions 0700"));
    }
    Ok(())
}
fn check_file_metadata(metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1 || metadata.len() != FILE_LEN {
        return Err(err("identity must be a regular, single-link, exactly 64-byte file"));
    }
    #[cfg(unix)]
    if cap_std::fs::MetadataExt::uid(metadata) != crate::fs_ext::effective_uid()
        || cap_std::fs::MetadataExt::mode(metadata) & 0o7777 != 0o600 {
        return Err(err("identity file must be owned by this user with permissions 0600"));
    }
    Ok(())
}
#[cfg(windows)]
fn check_profile(dir: &Dir) -> Result<(), String> { windows_acl::check_directory(dir, true) }
#[cfg(windows)]
fn check_directory(dir: &Dir) -> Result<(), String> { windows_acl::check_directory(dir, false) }
#[cfg(not(any(unix, windows)))]
fn check_profile(_: &Dir) -> Result<(), String> { Err(err("this platform has no supported private-file permission implementation")) }
#[cfg(not(any(unix, windows)))]
fn check_directory(_: &Dir) -> Result<(), String> { Err(err("this platform has no supported private-file permission implementation")) }

fn existing(directory: &Directory) -> Result<Option<Identity>, String> {
    let before = match directory.dir.symlink_metadata(KEY_FILE) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(err(error)),
        Ok(metadata) => metadata,
    };
    check_file_metadata(&before)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC); }
    #[cfg(windows)]
    { use cap_std::fs::OpenOptionsExt; options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4); }
    let mut file = directory.dir.open_with(KEY_FILE, &options).map_err(err)?;
    let opened = file.metadata().map_err(err)?;
    check_file_metadata(&opened)?;
    if !same(&before, &opened) { return Err(err("identity changed while opening")); }
    #[cfg(windows)]
    windows_acl::check_file(&file)?;
    let mut identity = Identity { secret: Zeroizing::new(vec![0; KEY_LEN]), public: [0; KEY_LEN] };
    file.read_exact(&mut identity.secret).map_err(err)?;
    file.read_exact(&mut identity.public).map_err(err)?;
    let mut extra = Zeroizing::new([0u8; 1]);
    if file.read(&mut extra[..]).map_err(err)? != 0 { return Err(err("identity grew while reading")); }
    let after = file.metadata().map_err(err)?;
    check_file_metadata(&after)?;
    if !same(&opened, &after) || after.modified().map_err(err)? != opened.modified().map_err(err)? {
        return Err(err("identity changed while reading"));
    }
    directory.file_unchanged(KEY_FILE, &opened)?;
    // Derive through the same maintained Noise implementation, rejecting mismatched
    // private/public files instead of advertising an identity we cannot authenticate.
    let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).ok_or_else(|| err("X25519 is unavailable"))?;
    dh.set(identity.secret_key());
    let matches = dh.pubkey() == identity.public_key();
    dh.set(&[0; KEY_LEN]);
    if !matches { return Err(err("identity public key does not match its secret")); }
    check_directory(&directory.dir)?;
    Ok(Some(identity))
}

/// Load only from this installation's native profile; never from portable reading data.
/// Existing malformed/unsafe entries are errors and are never repaired or overwritten.
pub fn load_or_create(data_dir: &Path) -> Result<Identity, String> {
    load_or_create_before_publish(data_dir, || {})
}
fn load_or_create_before_publish(data_dir: &Path, before_publish: impl FnOnce()) -> Result<Identity, String> {
    // Windows creates an empty inherited-private directory before protecting its
    // DACL. Serialize this initialization within a process; another process seeing
    // an incomplete DACL transition fails closed rather than accepting permissions.
    #[cfg(windows)]
    static INITIALIZATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    #[cfg(windows)]
    let _initialization = INITIALIZATION.lock().map_err(|_| err("Windows identity initialization lock is poisoned"))?;
    let profile = Directory::open(data_dir)?;
    let directory = profile.private_child()?;
    if let Some(identity) = existing(&directory)? {
        profile.unchanged()?;
        return Ok(identity);
    }
    let params = NOISE.parse().map_err(err)?;
    let pair = snow::Builder::new(params).generate_keypair().map_err(err)?;
    let secret = Zeroizing::new(pair.private);
    let public: [u8; KEY_LEN] = pair.public.try_into().map_err(|_| err("Noise generated an invalid public key length"))?;
    if secret.len() != KEY_LEN { return Err(err("Noise generated an invalid secret key length")); }
    let identity = Identity { secret, public };
    profile.unchanged()?;
    directory.unchanged()?;
    check_directory(&directory.dir)?;
    let pending = format!(".identity-{}.pending", uuid::Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    { use cap_std::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC); }
    #[cfg(windows)]
    { use cap_std::fs::OpenOptionsExt; options.access_mode(0xc006_0000).custom_flags(0x0020_0000).share_mode(1 | 4); }
    let mut file = directory.dir.open_with(&pending, &options).map_err(err)?;
    #[cfg(windows)]
    windows_acl::restrict_file(&file)?;
    crate::mobile_backup::exclude_payload(&file, &directory.path.join(&pending))?;
    file.write_all(identity.secret_key()).map_err(err)?;
    file.write_all(identity.public_key()).map_err(err)?;
    file.sync_all().map_err(err)?;
    check_file_metadata(&file.metadata().map_err(err)?)?;
    let verified = crate::publication::VerifiedFile::new(file.into_std(), FILE_LEN).map_err(err)?;
    before_publish();
    profile.unchanged()?;
    directory.unchanged()?;
    check_directory(&directory.dir)?;
    match crate::publication::publish_verified_noreplace_in(
        &directory.dir, Path::new(&pending), Path::new(KEY_FILE), &verified,
    ) {
        Ok(()) => {
            directory.sync()?;
            directory.file_unchanged(KEY_FILE, &Metadata::from_file(verified.file()).map_err(err)?)?;
            profile.unchanged()?;
            Ok(identity)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // A simultaneous creator wins; load its complete identity, never overwrite.
            let winner = existing(&directory)?.ok_or_else(|| err("concurrent identity creation disappeared"))?;
            directory.sync()?;
            profile.unchanged()?;
            Ok(winner)
        }
        Err(error) => Err(err(error)),
    }
}

// Cache data may inherit the standard private Windows profile ACL. Only the
// file owner, LocalSystem and Builtin Administrators may hold allow entries.
#[cfg(windows)]
pub(crate) fn check_cache_directory_access(dir: &Dir) -> Result<(), String> {
    windows_acl::check_directory(dir, true)
}
#[cfg(windows)]
pub(crate) fn check_cache_file_access(file: &cap_std::fs::File) -> Result<(), String> {
    windows_acl::check(file, true, false)
}

#[cfg(windows)]
mod windows_acl {
    use super::{err, Dir, OpenOptions};
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{File, OpenOptionsExt};
    use std::{ffi::c_void, os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle}};
    use windows::{
        core::{BOOL, PCWSTR, PWSTR},
        Win32::{
            Foundation::{HANDLE, HLOCAL, LocalFree},
            Security::{
                Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                    GetSecurityInfo, SetSecurityInfo, SE_FILE_OBJECT},
                EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
                GetTokenInformation, IsValidSid, IsWellKnownSid, ACL, ACE_HEADER, ACCESS_ALLOWED_ACE, PSID,
                PSECURITY_DESCRIPTOR, TOKEN_USER, TokenUser, TOKEN_QUERY,
                DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
                PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
                WinLocalSystemSid, WinBuiltinAdministratorsSid,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
            Storage::FileSystem::{ReOpenFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
        },
    };

    struct Allocation(*mut c_void);
    impl Drop for Allocation {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a non-null allocation that each constructor's API
            // requires `LocalFree` for: ConvertStringSecurityDescriptorToSecurityDescriptorW,
            // GetSecurityInfo and ConvertSidToStringSidW. It is freed exactly once
            // here and never used afterwards.
            if !self.0.is_null() { unsafe { let _ = LocalFree(Some(HLOCAL(self.0))); } }
        }
    }
    struct User { storage: Vec<usize> }
    impl User {
        fn current() -> Result<Self, String> {
            let mut raw = HANDLE::default();
            // SAFETY: `GetCurrentProcess` returns a process-lifetime pseudo-handle that must not be
            // closed and is only read; the token handle is written to the live local `raw`.
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw).map_err(err)?; }
            // SAFETY: `raw` is the handle just opened and is not copied elsewhere; `OwnedHandle`
            // owns it and closes it exactly once.
            let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
            let mut required = 0;
            // SAFETY: a null buffer with zero length only asks for the required size; the token
            // handle is alive and `required` is a writable live local.
            unsafe { let _ = GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, None, 0, &mut required); }
            if required == 0 || required > 64 * 1024 { return Err(err("could not determine the Windows user SID")); }
            let mut storage = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
            // SAFETY: `storage` holds at least `required` zeroed bytes, is `usize`-aligned, and
            // stays put while the live token is queried; the API writes at most `required` bytes.
            unsafe {
                GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser,
                    Some(storage.as_mut_ptr().cast()), required, &mut required).map_err(err)?;
            }
            Ok(Self { storage })
        }
        // SAFETY: `storage` owns a `TOKEN_USER` written by `GetTokenInformation` above; it is
        // `usize`-aligned and never reallocated, so the SID points into live owned memory.
        fn sid(&self) -> PSID { unsafe { (*(self.storage.as_ptr().cast::<TOKEN_USER>())).User.Sid } }
        fn text(&self) -> Result<String, String> {
            let mut text = PWSTR::null();
            // SAFETY: `sid` points into `self.storage`, which outlives this call; the API allocates
            // a NUL-terminated string and returns it through the live local `text`.
            unsafe { ConvertSidToStringSidW(self.sid(), &mut text).map_err(err)?; }
            let _allocation = Allocation(text.0.cast());
            // SAFETY: `_allocation` owns the NUL-terminated string returned above and keeps it
            // alive until after this read, which does not modify it.
            unsafe { text.to_string().map_err(err) }
        }
    }
    fn directory_handle(dir: &Dir, writable: bool, owner: bool) -> Result<File, String> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No)
            .access_mode(0x0002_0000 | if writable { 0x0004_0000 } else { 0 }
                | if owner { 0x0008_0000 } else { 0 })
            .custom_flags(0x0220_0000).share_mode(1 | 2 | 4);
        dir.open_with(".", &options).map_err(err)
    }
    fn restrict(file: &File, directory: bool) -> Result<(), String> {
        let user = User::current()?;
        let inheritance = if directory { "OICI" } else { "" };
        let sddl: Vec<u16> = format!("D:P(A;{inheritance};FA;;;{})", user.text()?)
            .encode_utf16().chain(Some(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` is NUL-terminated and alive across the call; the API allocates a
        // self-contained descriptor and stores it in the live local `descriptor`.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut descriptor, None).map_err(err)?;
        }
        let _allocation = Allocation(descriptor.0);
        let mut present = BOOL::default();
        let mut defaulted = BOOL::default();
        let mut dacl = std::ptr::null_mut();
        // SAFETY: `descriptor` is the live owned allocation above; `dacl` points into it and
        // `present`/`defaulted` are writable locals, so the DACL stays valid for `SetSecurityInfo`.
        unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted).map_err(err)?;
            if !present.as_bool() || dacl.is_null() { return Err(err("Windows private DACL is unavailable")); }
            let owner_result = SetSecurityInfo(HANDLE(file.as_raw_handle()), SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                Some(user.sid()), None, Some(dacl), None);
            if owner_result.is_err() {
                // The handle may lack WRITE_OWNER. The DACL is still required;
                // `check` below rejects an owner that is not this user.
                SetSecurityInfo(HANDLE(file.as_raw_handle()), SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None, None, Some(dacl), None).ok().map_err(err)?;
            }
        }
        check(file, false, directory)
    }
    pub(super) fn restrict_directory(dir: &Dir) -> Result<(), String> {
        let handle = directory_handle(dir, true, false)?;
        match restrict(&handle, true) {
            Ok(()) => Ok(()),
            Err(first) => {
                let owner = directory_handle(dir, true, true)?;
                restrict(&owner, true).map_err(|_| first)
            }
        }
    }
    pub(super) fn restrict_file(file: &File) -> Result<(), String> {
        match restrict(file, false) {
            Ok(()) => Ok(()),
            Err(first) => {
                // Reopen the same object, never a name that delete-sharing can
                // replace between the original open and the retry.
                // SAFETY: `file` owns a live handle; the returned handle is
                // adopted exactly once by `File::from_raw_handle`.
                let owner = unsafe {
                    ReOpenFile(HANDLE(file.as_raw_handle()), 0x000e_0000,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        FILE_FLAGS_AND_ATTRIBUTES(0))
                }.map_err(err)?;
                // SAFETY: ReOpenFile returned a new owned handle, adopted once.
                let owner = unsafe { File::from_raw_handle(owner.0) };
                restrict(&owner, false).map_err(|_| first)
            }
        }
    }
    pub(super) fn check_directory(dir: &Dir, profile: bool) -> Result<(), String> {
        check(&directory_handle(dir, false, false)?, profile, true)
    }
    pub(super) fn check_file(file: &File) -> Result<(), String> { check(file, false, false) }

    pub(super) fn check(file: &File, profile: bool, directory: bool) -> Result<(), String> {
        let user = User::current()?;
        let mut owner = PSID::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `file` is a live handle owned by the caller; `owner`, `dacl` and `descriptor`
        // are writable live locals, and the API allocates the descriptor recorded there.
        unsafe {
            GetSecurityInfo(HANDLE(file.as_raw_handle()), SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut owner), None, Some(&mut dacl), None, Some(&mut descriptor)).ok().map_err(err)?;
        }
        let _allocation = Allocation(descriptor.0);
        // SAFETY: `_allocation` keeps the descriptor allocation alive; `owner`, `dacl` and every
        // `GetAce` result point inside it and are only read, and `user` outlives the comparisons.
        unsafe {
            if owner.0.is_null() || EqualSid(owner, user.sid()).is_err() || dacl.is_null() || (*dacl).AceCount == 0 {
                return Err(err("Windows identity storage must be owned by this user with a restrictive DACL"));
            }
            if !profile {
                let mut control = 0u16;
                let mut revision = 0;
                GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).map_err(err)?;
                if control & SE_DACL_PROTECTED.0 == 0 || (*dacl).AceCount != 1 {
                    return Err(err("Windows identity storage must have a protected, single-user DACL"));
                }
            }
            for index in 0..u32::from((*dacl).AceCount) {
                let mut raw = std::ptr::null_mut();
                GetAce(dacl, index, &mut raw).map_err(err)?;
                if raw.is_null() { return Err(err("Windows identity storage has an invalid ACE")); }
                let header = &*raw.cast::<ACE_HEADER>();
                let acl_start = dacl as usize;
                let acl_end = acl_start.checked_add(usize::from((*dacl).AclSize))
                    .ok_or_else(|| err("Windows identity ACL size is invalid"))?;
                let ace_start = raw as usize;
                let ace_end = ace_start.checked_add(usize::from(header.AceSize))
                    .ok_or_else(|| err("Windows identity ACE size is invalid"))?;
                if ace_start < acl_start || ace_end > acl_end {
                    return Err(err("Windows identity ACE exceeds its ACL"));
                }
                // Reject callback/object/deny ACEs rather than interpreting arbitrary ACLs.
                if header.AceType != 0 { return Err(err("Windows identity storage has an unexpected ACE type")); }
                let sid_offset = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
                if usize::from(header.AceSize) < sid_offset + 8 {
                    return Err(err("Windows identity ACE is too short for a SID"));
                }
                let sid_bytes = raw.cast::<u8>().add(sid_offset);
                let subauthorities = usize::from(*sid_bytes.add(1));
                let sid_length = 8usize.checked_add(subauthorities.checked_mul(4)
                    .ok_or_else(|| err("Windows identity SID is too large"))?)
                    .ok_or_else(|| err("Windows identity SID is too large"))?;
                if sid_offset.checked_add(sid_length).is_none_or(|end| end > usize::from(header.AceSize)) {
                    return Err(err("Windows identity SID exceeds its ACE"));
                }
                let ace = &*raw.cast::<ACCESS_ALLOWED_ACE>();
                let sid = PSID(std::ptr::addr_of!(ace.SidStart).cast_mut().cast());
                if !IsValidSid(sid).as_bool() {
                    return Err(err("Windows identity ACE contains an invalid SID"));
                }
                let own = EqualSid(sid, user.sid()).is_ok();
                if profile {
                    // Standard private Windows profiles also grant the two OS administrators.
                    if !own && !IsWellKnownSid(sid, WinLocalSystemSid).as_bool()
                        && !IsWellKnownSid(sid, WinBuiltinAdministratorsSid).as_bool() {
                        return Err(err("Windows profile grants access to another user or group"));
                    }
                } else if !own || ace.Mask != 0x001f_01ff
                    || ace.Header.AceFlags != if directory { 3 } else { 0 } {
                    return Err(err("Windows identity permissions are not current-user-only full control"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::{Arc, Barrier}};

    struct OwnedProfile { root: PathBuf, profile: PathBuf }
    impl OwnedProfile {
        fn new() -> Self {
            // Canonicalize only the OS temp root, never the profile passed to the API:
            // macOS commonly exposes the former through the /var -> /private/var link.
            let base = std::env::temp_dir().canonicalize().unwrap();
            let root = base.join(format!("kiwix-private-identity-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let profile = root.join("profile");
            fs::create_dir(&profile).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
                fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self { root, profile }
        }
        fn key(&self) -> PathBuf { self.profile.join(DIRECTORY).join(KEY_FILE) }
    }
    impl Drop for OwnedProfile {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
    }

    #[test]
    fn generated_identity_persists_and_has_only_public_hex_exposure() {
        let profile = OwnedProfile::new();
        let first = load_or_create(&profile.profile).unwrap();
        assert_eq!(first.public_key().len(), KEY_LEN);
        assert_eq!(first.secret_key().len(), KEY_LEN);
        let public = first.public_hex();
        assert_eq!(public.len(), 64);
        assert!(public.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        let second = load_or_create(&profile.profile).unwrap();
        assert_eq!(public, second.public_hex());
        assert!(first.secret_key() == second.secret_key());
        assert_eq!(fs::metadata(profile.key()).unwrap().len(), FILE_LEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(profile.key()).unwrap().permissions().mode() & 0o7777, 0o600);
            assert_eq!(fs::metadata(profile.profile.join(DIRECTORY)).unwrap().permissions().mode() & 0o7777, 0o700);
        }
    }

    #[test]
    fn distinct_installations_have_distinct_keys() {
        let left = OwnedProfile::new();
        let right = OwnedProfile::new();
        assert_ne!(load_or_create(&left.profile).unwrap().public_hex(),
            load_or_create(&right.profile).unwrap().public_hex());
    }

    #[test]
    fn simultaneous_creators_converge_without_overwrite() {
        let profile = OwnedProfile::new();
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4).map(|_| {
            let path = profile.profile.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                load_or_create(&path).unwrap().public_hex()
            })
        }).collect();
        let public = load_or_create(&profile.profile).unwrap().public_hex();
        for handle in handles { assert_eq!(handle.join().unwrap(), public); }
    }

    #[test]
    fn unexpected_file_lengths_are_preserved_not_repaired() {
        for length in [0, 31, 32, 63, 65, 256] {
            let profile = OwnedProfile::new();
            load_or_create(&profile.profile).unwrap();
            let bytes = vec![0x5a; length];
            fs::write(profile.key(), &bytes).unwrap();
            assert!(load_or_create(&profile.profile).is_err());
            assert!(fs::read(profile.key()).unwrap() == bytes);
        }
    }

    #[test]
    fn inconsistent_keypair_is_rejected_without_replacement() {
        let profile = OwnedProfile::new();
        load_or_create(&profile.profile).unwrap();
        let mut bytes = Zeroizing::new(fs::read(profile.key()).unwrap());
        bytes[KEY_LEN] ^= 0x80;
        fs::write(profile.key(), bytes.as_slice()).unwrap();
        assert!(load_or_create(&profile.profile).is_err());
        assert!(fs::read(profile.key()).unwrap().as_slice() == bytes.as_slice());
    }

    #[test]
    fn invalid_entry_appearing_before_publish_is_not_clobbered() {
        let profile = OwnedProfile::new();
        let result = load_or_create_before_publish(&profile.profile, || {
            // This is a different creator's file, not a valid identity. Publication
            // must preserve it and validation must fail rather than replace it.
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            options.open(profile.key()).unwrap().write_all(b"unrelated owned sentinel").unwrap();
        });
        assert!(result.is_err());
        assert_eq!(fs::read(profile.key()).unwrap(), b"unrelated owned sentinel");
    }

    #[cfg(unix)]
    #[test]
    fn permissive_file_directory_and_profile_are_rejected_without_chmod() {
        use std::os::unix::fs::PermissionsExt;
        for (relative, mode) in [(PathBuf::from(DIRECTORY).join(KEY_FILE), 0o644),
            (PathBuf::from(DIRECTORY), 0o755), (PathBuf::new(), 0o777)] {
            let profile = OwnedProfile::new();
            load_or_create(&profile.profile).unwrap();
            let path = profile.profile.join(relative);
            let original = Zeroizing::new(fs::read(profile.key()).unwrap());
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(load_or_create(&profile.profile).is_err());
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o7777, mode);
            assert!(fs::read(profile.key()).unwrap().as_slice() == original.as_slice());
        }
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_identity_is_rejected_without_touching_either_name() {
        let profile = OwnedProfile::new();
        load_or_create(&profile.profile).unwrap();
        let linked = profile.root.join("other-key");
        fs::hard_link(profile.key(), &linked).unwrap();
        let original = Zeroizing::new(fs::read(&linked).unwrap());
        assert!(load_or_create(&profile.profile).is_err());
        assert!(fs::read(profile.key()).unwrap().as_slice() == original.as_slice());
        assert!(fs::read(&linked).unwrap().as_slice() == original.as_slice());
    }

    #[cfg(unix)]
    #[test]
    fn file_symlink_including_dangling_link_is_never_followed() {
        use std::os::unix::fs::symlink;
        for dangling in [false, true] {
            let profile = OwnedProfile::new();
            load_or_create(&profile.profile).unwrap();
            fs::remove_file(profile.key()).unwrap();
            let victim = profile.root.join("unrelated");
            if !dangling { fs::write(&victim, b"untouched").unwrap(); }
            symlink(&victim, profile.key()).unwrap();
            assert!(load_or_create(&profile.profile).is_err());
            assert!(fs::symlink_metadata(profile.key()).unwrap().file_type().is_symlink());
            if dangling { assert!(!victim.exists()); }
            else { assert_eq!(fs::read(&victim).unwrap(), b"untouched"); }
        }
    }

    #[cfg(unix)]
    #[test]
    fn directory_and_ancestor_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let profile = OwnedProfile::new();
        let victim = profile.root.join("unrelated");
        fs::create_dir(&victim).unwrap();
        fs::write(victim.join("sentinel"), b"untouched").unwrap();
        symlink(&victim, profile.profile.join(DIRECTORY)).unwrap();
        assert!(load_or_create(&profile.profile).is_err());
        assert!(!victim.join(KEY_FILE).exists());
        fs::remove_file(profile.profile.join(DIRECTORY)).unwrap();
        let alias = profile.root.join("profile-alias");
        symlink(&profile.profile, &alias).unwrap();
        assert!(load_or_create(&alias).is_err());
        assert!(!profile.profile.join(DIRECTORY).exists());
        assert_eq!(fs::read(victim.join("sentinel")).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn parent_substitution_before_commit_does_not_write_through_symlink() {
        use std::os::unix::fs::symlink;
        let profile = OwnedProfile::new();
        let victim = profile.root.join("unrelated");
        fs::create_dir(&victim).unwrap();
        fs::write(victim.join("sentinel"), b"untouched").unwrap();
        let result = load_or_create_before_publish(&profile.profile, || {
            fs::rename(profile.profile.join(DIRECTORY), profile.root.join("retained-owned-directory")).unwrap();
            symlink(&victim, profile.profile.join(DIRECTORY)).unwrap();
        });
        assert!(result.is_err());
        assert!(!victim.join(KEY_FILE).exists());
        assert!(!profile.root.join("retained-owned-directory").join(KEY_FILE).exists());
        assert_eq!(fs::read(victim.join("sentinel")).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn non_symlink_profile_substitution_before_commit_is_detected() {
        use std::os::unix::fs::PermissionsExt;
        let profile = OwnedProfile::new();
        let result = load_or_create_before_publish(&profile.profile, || {
            fs::rename(&profile.profile, profile.root.join("retained-profile")).unwrap();
            fs::create_dir(&profile.profile).unwrap();
            fs::set_permissions(&profile.profile, fs::Permissions::from_mode(0o700)).unwrap();
        });
        assert!(result.is_err());
        assert!(!profile.profile.join(DIRECTORY).exists());
        assert!(!profile.root.join("retained-profile").join(DIRECTORY).join(KEY_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_appearing_at_publication_is_not_followed_or_replaced() {
        use std::os::unix::fs::symlink;
        let profile = OwnedProfile::new();
        let victim = profile.root.join("unrelated");
        fs::write(&victim, b"untouched").unwrap();
        let result = load_or_create_before_publish(&profile.profile, || {
            symlink(&victim, profile.key()).unwrap();
        });
        assert!(result.is_err());
        assert!(fs::symlink_metadata(profile.key()).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&victim).unwrap(), b"untouched");
    }

    #[cfg(windows)]
    #[test]
    fn windows_broad_dacl_is_rejected_without_repair_or_clobber() {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::{HANDLE, HLOCAL, LocalFree},
            Security::{
                Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SetSecurityInfo, SE_FILE_OBJECT},
                ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
                GetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
            },
        };
        for relative in [PathBuf::new(), PathBuf::from(DIRECTORY), PathBuf::from(DIRECTORY).join(KEY_FILE)] {
            let profile = OwnedProfile::new();
            load_or_create(&profile.profile).unwrap();
            let original = Zeroizing::new(fs::read(profile.key()).unwrap());
            let file = fs::OpenOptions::new().read(true).access_mode(0x0006_0000)
                .custom_flags(0x0220_0000).open(profile.profile.join(relative)).unwrap();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let mut present = windows::core::BOOL::default();
            let mut defaulted = windows::core::BOOL::default();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            // SAFETY: the descriptor is allocated by `ConvertString...` and freed exactly once in
            // both branches below; `present`, `defaulted` and `dacl` are writable locals, the DACL
            // pointer stays inside the live allocation, and `file` is a live handle.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    windows::core::w!("D:P(A;;FA;;;WD)"), 1, &mut descriptor, None).unwrap();
                let parsed = GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted);
                if let Err(error) = parsed {
                    let _ = LocalFree(Some(HLOCAL(descriptor.0)));
                    panic!("Reading test security descriptor: {error}");
                }
                let changed = SetSecurityInfo(HANDLE(file.as_raw_handle()), SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None, None, Some(dacl), None);
                let _ = LocalFree(Some(HLOCAL(descriptor.0)));
                changed.ok().unwrap();
            }
            drop(file);
            assert!(load_or_create(&profile.profile).is_err());
            assert!(fs::read(profile.key()).unwrap().as_slice() == original.as_slice());
        }
    }
}

