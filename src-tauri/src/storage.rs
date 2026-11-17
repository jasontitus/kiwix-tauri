// SPDX-License-Identifier: GPL-3.0-or-later
//! Native filesystem limits, shared by download, map and torrent preflights.
use std::path::{Path, PathBuf};

pub const FAT_MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// Open normalized absolute paths without following any caller-controlled component.
/// iOS cannot open `/` or other ancestors outside its sandbox: start at the
/// OS-provided, retained application container instead.
pub(crate) fn open_directory_nofollow(path: &Path) -> std::io::Result<cap_std::fs::Dir> {
    use cap_fs_ext::DirExt;
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::CurDir | Component::ParentDir)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Directory path must be normalized and absolute"));
    }
    #[cfg(target_os = "ios")]
    let (mut directory, relative) = {
        use std::os::unix::fs::MetadataExt;
        let root = IOS_CONTAINER.as_ref().map_err(|error| std::io::Error::new(error.kind(), format!("Opening native iOS container: {error}")))?;
        let current = std::fs::symlink_metadata(&root.path)?;
        let retained = root.directory.try_clone()?.into_std_file().metadata()?;
        if !current.is_dir() || current.dev() != retained.dev() || current.ino() != retained.ino() {
            return Err(std::io::Error::other("Native iOS container identity changed"));
        }
        let relative = path.strip_prefix(&root.path).map_err(|_| std::io::Error::new(
            std::io::ErrorKind::PermissionDenied, format!("Directory {} is outside native iOS container {}", path.display(), root.path.display())))?;
        (root.directory.try_clone()?, relative)
    };
    #[cfg(target_os = "macos")]
    if crate::sandbox::is_sandboxed() {
        return open_sandboxed_directory(path);
    }
    #[cfg(not(target_os = "ios"))]
    let (mut directory, relative) = {
        let mut root = PathBuf::new();
        for part in path.components() {
            if matches!(part, Component::Prefix(_) | Component::RootDir) { root.push(part.as_os_str()); } else { break; }
        }
        let relative = path.strip_prefix(&root).map_err(std::io::Error::other)?;
        (cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())?, relative)
    };
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Directory path contains a non-normal component"));
        };
        directory = directory.open_dir_nofollow(name)?;
    }
    Ok(directory)
}

/// The App Sandbox denies opening ancestors of a granted folder (`/Users/<name>`
/// among them), so the component walk above cannot start at `/`. Open the
/// directory itself instead, without following a final link, and confirm that
/// no earlier component is a link either: when the kernel's path for the
/// descriptor is spelled differently from the request (Unicode form, case,
/// `/System/Volumes/Data`), every ancestor is checked with `lstat`, which reads
/// metadata only, and the request must still name the opened directory.
#[cfg(target_os = "macos")]
fn open_sandboxed_directory(path: &Path) -> std::io::Result<cap_std::fs::Dir> {
    use std::os::{fd::AsRawFd, unix::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}}};
    // Normalize trailing separators/dots before O_NOFOLLOW: Darwin otherwise
    // follows an alias in both "alias/" and "alias/." spellings.
    let path = path.components().as_path();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes, including the NUL, into the buffer.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let resolved = std::ffi::CStr::from_bytes_until_nul(&buffer)
        .map_err(|_| std::io::Error::other("Resolved directory path is not terminated"))?;
    if !same_path_spelling(resolved.to_bytes(), path.as_os_str().as_bytes()) {
        let linked = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Directory {} resolves through a symbolic link", path.display()),
        );
        for ancestor in path.ancestors().skip(1) {
            // A refusal keeps its OS error so callers can ask for access.
            if std::fs::symlink_metadata(ancestor)?.file_type().is_symlink() {
                return Err(linked);
            }
        }
        let opened = file.metadata()?;
        let named = std::fs::metadata(path)?;
        if opened.dev() != named.dev() || opened.ino() != named.ino() {
            return Err(linked);
        }
    }
    Ok(cap_std::fs::Dir::from_std_file(file))
}

/// Exact match, ignoring trailing slashes.
#[cfg(any(target_os = "macos", test))]
fn same_path_spelling(resolved: &[u8], requested: &[u8]) -> bool {
    fn trim(mut bytes: &[u8]) -> &[u8] {
        while bytes.len() > 1 && bytes.last() == Some(&b'/') { bytes = &bytes[..bytes.len() - 1]; }
        bytes
    }
    trim(resolved) == trim(requested)
}

#[cfg(test)]
#[test]
fn sandboxed_directory_spelling_ignores_trailing_slashes_only() {
    assert!(same_path_spelling(b"/Users/ann/zims", b"/Users/ann/zims/"));
    assert!(same_path_spelling(b"/", b"/"));
    // Anything else falls back to the per-component link check.
    assert!(!same_path_spelling(b"/Users/Ann/ZIMs", b"/Users/ann/zims"));
    assert!(!same_path_spelling(b"/private/var/zims", b"/var/zims"));
}

#[cfg(target_os = "macos")]
#[test]
fn sandboxed_directory_rejects_symlinks_with_trailing_separators() {
    use std::os::unix::fs::symlink;
    let base = std::env::temp_dir().join(format!("kiwix-sandbox-path-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&base).unwrap();
    let base = base.canonicalize().unwrap();
    let real = base.join("real");
    let alias = base.join("alias");
    std::fs::create_dir(&real).unwrap();
    symlink(&real, &alias).unwrap();
    let linked = [alias.clone(), alias.join(""), alias.join("."), alias.join("././")];
    for path in linked {
        assert!(open_sandboxed_directory(&path).is_err(), "followed {}", path.display());
    }
    for path in [&real, &real.join(""), &real.join("."), &real.join("././")] {
        let directory = open_sandboxed_directory(path).unwrap();
        directory.write("proof", b"real directory").unwrap();
        assert_eq!(std::fs::read(real.join("proof")).unwrap(), b"real directory");
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[cfg(target_os = "ios")]
struct IosContainer {
    path: PathBuf,
    directory: cap_std::fs::Dir,
}
#[cfg(target_os = "ios")]
static IOS_CONTAINER: std::sync::LazyLock<std::io::Result<IosContainer>> = std::sync::LazyLock::new(open_ios_container);

/// Bootstrap only from Foundation's native container, before profile initialization.
#[cfg(target_os = "ios")]
fn open_ios_container() -> std::io::Result<IosContainer> {
    use std::{ffi::CStr, os::{fd::FromRawFd, unix::ffi::OsStrExt}};
    unsafe extern "C" {
        fn kiwix_mobile_open_app_root(path: *mut std::ffi::c_char, capacity: usize) -> i32;
    }
    let mut path = [0 as std::ffi::c_char; libc::PATH_MAX as usize];
    let fd = unsafe { kiwix_mobile_open_app_root(path.as_mut_ptr(), path.len()) };
    if fd < 0 { return Err(std::io::Error::last_os_error()); }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let path = CStr::from_bytes_until_nul(unsafe {
        std::slice::from_raw_parts(path.as_ptr().cast::<u8>(), path.len())
    }).map_err(|_| std::io::Error::other("Native iOS container path is not terminated"))?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
    if !path.is_absolute() || !file.metadata()?.is_dir() {
        return Err(std::io::Error::other("Invalid native iOS container capability"));
    }
    Ok(IosContainer { path, directory: cap_std::fs::Dir::from_std_file(file) })
}

/// Synchronize the retained directory itself, never a path reopened through
/// ambient authority. Linux/Android cap-std directory capabilities use O_PATH;
/// cloning that descriptor does not make it valid for fsync.
#[cfg(unix)]
pub fn sync_cap_directory(directory: &cap_std::fs::Dir) -> std::io::Result<()> {
    use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};

    let before = directory.dir_metadata()?;
    if !before.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Directory capability does not refer to a directory"));
    }
    let mut options = OpenOptions::new();
    // Do not call open_dir/reopen_dir: those deliberately request O_PATH on
    // Linux. A plain read-only open of "." retains the original capability.
    options.read(true).follow(FollowSymlinks::No)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let readable = directory.open_with(".", &options)?;
    let opened = readable.metadata()?;
    if !opened.is_dir() || opened.dev() != before.dev() || opened.ino() != before.ino() {
        return Err(std::io::Error::other("Readable directory descriptor changed capability identity"));
    }
    readable.sync_all()?;
    let after = directory.dir_metadata()?;
    if !after.is_dir() || after.dev() != opened.dev() || after.ino() != opened.ino() {
        return Err(std::io::Error::other("Directory capability changed identity during synchronization"));
    }
    Ok(())
}

/// Rename the open file or directory `handle` to the leaf `name` inside the
/// directory capability `directory`, never replacing an existing entry.
///
/// Win32 `SetFileInformationByHandle(FileRenameInfo)` rejects a non-NULL
/// `RootDirectory` with ERROR_INVALID_PARAMETER (87) on current Windows, so a
/// capability-relative rename must use the native
/// `NtSetInformationFile(FileRenameInformation)` with the same structure.
/// Failures are converted with `RtlNtStatusToDosError`, the conversion the
/// Win32 wrapper applies, so callers keep seeing the usual `raw_os_error()`
/// values (for example ERROR_ALREADY_EXISTS when the target exists).
#[cfg(windows)]
pub(crate) fn rename_into_directory_noreplace(
    handle: std::os::windows::io::RawHandle,
    directory: std::os::windows::io::RawHandle,
    name: &std::ffi::OsStr,
) -> std::io::Result<()> {
    use std::{
        ffi::c_void,
        mem::{offset_of, size_of},
        os::windows::ffi::OsStrExt,
    };
    // FILE_RENAME_INFORMATION: the BOOLEAN ReplaceIfExists shares its slot
    // with the ULONG Flags union member; zero means "do not replace".
    #[repr(C)]
    struct RenameInformation {
        replace_if_exists: u32,
        root_directory: *mut c_void,
        file_name_length: u32,
        file_name: [u16; 1],
    }
    // IO_STATUS_BLOCK: union { NTSTATUS Status; PVOID Pointer; }, ULONG_PTR.
    #[repr(C)]
    struct IoStatusBlock {
        status: isize,
        information: usize,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSetInformationFile(
            file: *mut c_void,
            io_status: *mut IoStatusBlock,
            information: *const c_void,
            length: u32,
            class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    const FILE_RENAME_INFORMATION: u32 = 10;
    const STATUS_PENDING: i32 = 0x103;

    let wide: Vec<u16> = name.encode_wide().collect();
    // A leaf name only: no NUL, separators, drive/stream colon, or dot entries.
    if wide.is_empty()
        || wide.iter().any(|unit| matches!(unit, 0 | 47 | 58 | 92))
        || wide == [46]
        || wide == [46, 46]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Capability rename target must be a single filename",
        ));
    }
    let too_long = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "Rename filename is too long");
    let name_bytes = wide.len().checked_mul(2).ok_or_else(too_long)?;
    let name_length = u32::try_from(name_bytes).map_err(|_| too_long())?;
    let length = offset_of!(RenameInformation, file_name)
        .checked_add(name_bytes)
        .map(|length| length.max(size_of::<RenameInformation>()))
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(too_long)?;
    // usize backing gives pointer alignment; zero-filling covers ABI padding.
    // The variable-length UTF-16 name carries no terminating NUL.
    let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    let information = buffer.as_mut_ptr().cast::<RenameInformation>();
    let mut io_status = IoStatusBlock { status: 0, information: 0 };
    // SAFETY: `buffer` is large enough for the header plus `name_bytes`, is
    // pointer-aligned and outlives the call; both handles are borrowed from
    // owners that outlive this synchronous request.
    let status = unsafe {
        std::ptr::addr_of_mut!((*information).replace_if_exists).write(0);
        std::ptr::addr_of_mut!((*information).root_directory).write(directory);
        std::ptr::addr_of_mut!((*information).file_name_length).write(name_length);
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            std::ptr::addr_of_mut!((*information).file_name).cast::<u16>(),
            wide.len(),
        );
        NtSetInformationFile(handle, &mut io_status, information.cast(), length, FILE_RENAME_INFORMATION)
    };
    if status >= 0 && status != STATUS_PENDING {
        Ok(())
    } else {
        // SAFETY: a pure status-code conversion with no pointer arguments.
        let code = unsafe { RtlNtStatusToDosError(status) };
        Err(std::io::Error::from_raw_os_error(code as i32))
    }
}

/// Windows: delete the empty directory `name` in `parent` only if the entry is
/// a directory that is not a symlink or junction, and `verified` holds on the
/// very handle that deletes it. Callers must first release their own
/// capabilities to it: cap-std directory handles never share delete.
///
/// The handle shares READ|WRITE but not DELETE, so nobody can rename or replace
/// the entry between the identity check and the deletion. Deletion uses POSIX
/// semantics, so the name disappears immediately even while another process
/// (Explorer, the indexer, antivirus) holds the folder open with delete
/// sharing; the legacy disposition would leave it delete-pending and make
/// removing the parent fail. Volumes without POSIX deletion (e.g. FAT) fall
/// back to the legacy disposition. A non-empty directory is refused with
/// ERROR_DIR_NOT_EMPTY, so unknown entries are never swept.
#[cfg(windows)]
pub(crate) fn remove_empty_directory_verified(
    parent: &cap_std::fs::Dir,
    name: &Path,
    verified: impl FnOnce(&cap_std::fs::Metadata) -> bool,
    replaced: &str,
) -> std::io::Result<()> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(file: *mut c_void, class: i32, information: *const c_void, size: u32) -> i32;
    }
    const DELETE: u32 = 0x0001_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ_WRITE: u32 = 1 | 2;
    const FILE_DISPOSITION_INFO: i32 = 4;
    const FILE_DISPOSITION_INFO_EX: i32 = 21;
    const FILE_DISPOSITION_FLAG_DELETE: u32 = 0x1;
    const FILE_DISPOSITION_FLAG_POSIX_SEMANTICS: u32 = 0x2;
    let mut options = OpenOptions::new();
    options
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .follow(FollowSymlinks::No)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ_WRITE);
    let directory = parent.open_with(name, &options)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.is_symlink() || !verified(&metadata) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, replaced.to_string()));
    }
    let handle = directory.as_raw_handle();
    let flags: u32 = FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS;
    // SAFETY: `handle` is owned by `directory`, which outlives both synchronous
    // calls; each buffer matches its class (FILE_DISPOSITION_INFO_EX is a ULONG
    // of flags, FILE_DISPOSITION_INFO a BOOLEAN) and outlives the call.
    if unsafe { SetFileInformationByHandle(handle, FILE_DISPOSITION_INFO_EX, std::ptr::from_ref(&flags).cast(), 4) } == 0 {
        let error = std::io::Error::last_os_error();
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER,
        // ERROR_INVALID_LEVEL: the volume lacks POSIX deletion.
        if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124)) {
            return Err(error);
        }
        let delete: u8 = 1;
        // SAFETY: as above.
        if unsafe { SetFileInformationByHandle(handle, FILE_DISPOSITION_INFO, std::ptr::from_ref(&delete).cast(), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    drop(directory);
    Ok(())
}

/// Windows: NTFS refuses to rename a directory while any file inside it is
/// open, whatever its share mode, and a third party (antivirus scanning
/// just-written files, the search indexer, a sync client) often holds one for
/// a moment. Retry such refusals (ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION)
/// a few times with backoff, about 1.5 s in total. Each `attempt` must redo
/// its own identity checks; the last error is returned unchanged.
#[cfg(windows)]
pub(crate) fn retry_transient_sharing<T>(mut attempt: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut delay = std::time::Duration::from_millis(50);
    for _ in 0..5 {
        match attempt() {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            other => return other,
        }
    }
    attempt()
}

/// A path for people to read: Windows verbatim forms (`\\?\C:\…`,
/// `\\?\UNC\server\share\…`) are shown in their ordinary spelling. Display
/// only; never use the result to open, compare or store a path.
pub(crate) fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text,
    }
}

/// Windows refuses to move or delete a file another program (or another part
/// of this app) holds open, reporting a sharing violation (32) or access
/// denied (5). Say what to do instead of only surfacing the raw OS error.
#[cfg(windows)]
pub(crate) fn explain_in_use(error: String) -> String {
    if error.contains("(os error 32)") || error.contains("(os error 5)") {
        format!("This archive may be in use by another program or task; close it (or stop sharing, checking or indexing it) and try again. Details: {error}")
    } else {
        error
    }
}

/// Reject only a *known* FAT/FAT32 single-file limit. exFAT is not FAT32.
/// An as-yet uncreated download directory is checked through its existing ancestor.
/// This function never creates directories or probes a volume with test writes.
pub fn check_file_size(directory: &Path, size: u64) -> Result<(), String> {
    if size <= FAT_MAX_FILE_SIZE {
        return Ok(());
    }
    let directory = existing_directory(directory)?;
    if fat_filesystem(&directory)? {
        return Err(format!(
            "{} uses FAT/FAT32, which cannot store one file larger than 4 GiB − 1 byte. Choose an exFAT, NTFS, APFS or other large-file-capable download directory.",
            directory.display()
        ));
    }
    Ok(())
}

/// Free and total bytes for the volume containing the nearest existing ancestor
/// of `path`. Missing directories are inspected through that ancestor without
/// creating or modifying anything. `None` means the platform or sandbox does
/// not report capacity; callers must present that honestly, not as zero.
pub fn volume_space(path: &Path) -> Option<(u64, u64)> {
    let existing = path.ancestors().find(|candidate| candidate.is_dir())?;
    let free = crate::fs_ext::available_space(existing).ok()?;
    let total = crate::fs_ext::total_space(existing).ok()?;
    Some((free, total))
}

fn existing_directory(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    for candidate in absolute.ancestors() {
        match std::fs::metadata(candidate) {
            Ok(metadata) if metadata.is_dir() => return Ok(candidate.to_path_buf()),
            Ok(_) => return Err(format!("Not a download directory: {}", candidate.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!(
                    "Inspecting download filesystem {}: {e}",
                    candidate.display()
                ))
            }
        }
    }
    Err("Download directory has no accessible filesystem ancestor".into())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn fat_filesystem(directory: &Path) -> Result<bool, String> {
    use std::os::unix::ffi::OsStrExt;
    let path =
        std::ffi::CString::new(directory.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: statfs initializes info on success and path is NUL-terminated.
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(format!(
            "Inspecting download filesystem: {}",
            std::io::Error::last_os_error()
        ));
    }
    let info = unsafe { info.assume_init() };
    let name = unsafe { std::ffi::CStr::from_ptr(info.f_fstypename.as_ptr()) };
    Ok(matches!(name.to_bytes(), b"msdos" | b"fat" | b"vfat"))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn fat_filesystem(directory: &Path) -> Result<bool, String> {
    use std::os::unix::ffi::OsStrExt;
    let path =
        std::ffi::CString::new(directory.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(format!(
            "Inspecting download filesystem: {}",
            std::io::Error::last_os_error()
        ));
    }
    // MSDOS_SUPER_MAGIC covers the vfat driver; exFAT has a distinct magic.
    Ok(unsafe { info.assume_init() }.f_type == 0x4d44)
}

#[cfg(windows)]
fn fat_filesystem(directory: &Path) -> Result<bool, String> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, length: u32) -> i32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_length: u32,
            serial: *mut u32,
            component: *mut u32,
            flags: *mut u32,
            filesystem: *mut u16,
            filesystem_length: u32,
        ) -> i32;
    }
    let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = [0u16; 32768];
    let mut filesystem = [0u16; 64];
    if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0
        || unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        } == 0
    {
        return Err(format!(
            "Inspecting download filesystem: {}",
            std::io::Error::last_os_error()
        ));
    }
    let end = filesystem
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(filesystem.len());
    Ok(fat_windows_name(&filesystem[..end]))
}

#[cfg(any(windows, test))]
fn fat_windows_name(name: &[u16]) -> bool {
    // Native Windows filesystem identifiers are uppercase ASCII. In particular
    // \"exFAT\" must not be classified by a substring match on \"FAT\".
    name == [70, 65, 84] || name == [70, 65, 84, 51, 50]
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android", windows)))]
fn fat_filesystem(_: &Path) -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn directory_capability_rejects_symlinks_and_parent_escape() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("kiwix-directory-capability-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::create_dir(root.join("owned")).unwrap();
        std::fs::write(root.join("owned/value"), b"retained bytes").unwrap();
        symlink(root.join("owned"), root.join("alias")).unwrap();
        let directory = open_directory_nofollow(&root.join("owned")).unwrap();
        assert_eq!(directory.read("value").unwrap(), b"retained bytes");
        assert!(open_directory_nofollow(&root.join("alias")).is_err());
        assert!(open_directory_nofollow(&root.join("owned/../owned")).is_err());
        std::fs::rename(root.join("owned"), root.join("moved")).unwrap();
        std::fs::create_dir(root.join("owned")).unwrap();
        std::fs::write(root.join("owned/value"), b"replacement bytes").unwrap();
        assert_eq!(directory.read("value").unwrap(), b"retained bytes");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fat_boundary_does_not_probe_or_modify_nonexistent_directory() {
        assert!(check_file_size(
            Path::new("/nonexistent/kiwix-size-check"),
            FAT_MAX_FILE_SIZE
        )
        .is_ok());
        assert_eq!(FAT_MAX_FILE_SIZE, u32::MAX as u64);
    }
    #[test]
    fn ancestor_lookup_is_read_only() {
        let root = std::env::temp_dir();
        let missing = root
            .join(format!("kiwix-no-create-{}", uuid::Uuid::new_v4()))
            .join("downloads");
        assert_eq!(existing_directory(&missing).unwrap(), root);
        assert!(!missing.exists());
    }
    #[test]
    fn volume_space_reports_capacity_through_an_existing_ancestor() {
        let root = std::env::temp_dir();
        let missing = root
            .join(format!("kiwix-space-{}", uuid::Uuid::new_v4()))
            .join("downloads");
        let (free, total) = volume_space(&missing).expect("temporary volume capacity");
        assert!(total > 0);
        assert!(free <= total);
        assert!(!missing.exists());
    }
    #[test]
    fn exfat_is_not_subject_to_the_fat32_limit() {
        assert!(fat_windows_name(&[70, 65, 84]));
        assert!(fat_windows_name(&[70, 65, 84, 51, 50]));
        assert!(!fat_windows_name(&[101, 120, 70, 65, 84]));
        assert!(!fat_windows_name(&[69, 88, 70, 65, 84]));
        assert!(!fat_windows_name(&[78, 84, 70, 83]));
    }

    /// Regression for os error 87: the Win32 wrapper rejected every
    /// capability-relative rename, so no download could be published.
    #[cfg(windows)]
    #[test]
    fn capability_relative_rename_moves_files_and_directories_without_replacing() {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        use cap_std::fs::{OpenOptions, OpenOptionsExt};
        use std::os::windows::io::AsRawHandle;
        let base = std::env::temp_dir().join(format!("kiwix-cap-rename-{}", uuid::Uuid::new_v4()));
        // Declared before the capabilities, so it drops after them, even on a failed assertion.
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let _cleanup = Cleanup(base.clone());
        std::fs::create_dir_all(base.join("dest")).unwrap();
        std::fs::write(base.join("staged.part"), b"archive").unwrap();
        std::fs::write(base.join("dest").join("occupied.zim"), b"keep").unwrap();
        std::fs::create_dir(base.join("folder")).unwrap();
        let root = cap_std::fs::Dir::open_ambient_dir(&base, cap_std::ambient_authority()).unwrap();
        let dest = root.open_dir("dest").unwrap();
        let open = |name: &str, directory: bool| {
            let mut options = OpenOptions::new();
            options.access_mode(0x0001_0000 | 0x0000_0080).follow(FollowSymlinks::No)
                .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 });
            root.open_with(name, &options).unwrap()
        };

        // An existing target is refused and left untouched.
        let staged = open("staged.part", false);
        let refused = rename_into_directory_noreplace(
            staged.as_raw_handle(), dest.as_raw_handle(), std::ffi::OsStr::new("occupied.zim")).unwrap_err();
        assert_eq!(refused.raw_os_error(), Some(183), "{refused}"); // ERROR_ALREADY_EXISTS
        assert_eq!(std::fs::read(base.join("dest").join("occupied.zim")).unwrap(), b"keep");
        // Non-leaf names never reach the kernel.
        for bad in ["", ".", "..", "a\\b", "a/b", "c:x", "a:stream"] {
            let error = rename_into_directory_noreplace(
                staged.as_raw_handle(), dest.as_raw_handle(), std::ffi::OsStr::new(bad)).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{bad:?}");
        }
        // Cross-directory and same-directory renames succeed.
        rename_into_directory_noreplace(
            staged.as_raw_handle(), dest.as_raw_handle(), std::ffi::OsStr::new("published.zim")).unwrap();
        drop(staged);
        assert_eq!(std::fs::read(base.join("dest").join("published.zim")).unwrap(), b"archive");
        assert!(!base.join("staged.part").exists());
        let folder = open("folder", true);
        rename_into_directory_noreplace(
            folder.as_raw_handle(), root.as_raw_handle(), std::ffi::OsStr::new("renamed")).unwrap();
        drop(folder);
        assert!(base.join("renamed").is_dir() && !base.join("folder").exists());
    }

    /// Regression: the legacy disposition left a removed folder delete-pending
    /// while Explorer/the indexer held it, so the parent could not be removed.
    #[cfg(windows)]
    #[test]
    fn verified_empty_directory_removal_is_immediate_identity_bound_and_empty_only() {
        use cap_fs_ext::MetadataExt as _;
        use std::os::windows::fs::OpenOptionsExt;
        let base = std::env::temp_dir().join(format!("kiwix-dir-remove-{}", uuid::Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let _cleanup = Cleanup(base.clone());
        std::fs::create_dir_all(base.join("parent").join("owned")).unwrap();
        std::fs::create_dir_all(base.join("parent").join("busy")).unwrap();
        std::fs::write(base.join("parent").join("busy").join("keep"), b"x").unwrap();
        let parent = cap_std::fs::Dir::open_ambient_dir(base.join("parent"), cap_std::ambient_authority()).unwrap();
        let identity = |name: &str| {
            let metadata = parent.symlink_metadata(name).unwrap();
            (metadata.dev(), metadata.ino())
        };
        let owned = identity("owned");
        // A foreign identity is refused and the entry preserved.
        let refused = remove_empty_directory_verified(&parent, Path::new("owned"), |_| false, "replaced").unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        assert!(base.join("parent").join("owned").is_dir());
        // A non-empty directory is never swept.
        let busy = identity("busy");
        let error = remove_empty_directory_verified(
            &parent, Path::new("busy"), |m| (m.dev(), m.ino()) == busy, "replaced").unwrap_err();
        assert_eq!(error.raw_os_error(), Some(145), "{error}"); // ERROR_DIR_NOT_EMPTY
        assert!(base.join("parent").join("busy").join("keep").is_file());
        // An observer holding the folder with share-all (like Explorer) does
        // not leave the name behind.
        let observer = std::fs::OpenOptions::new()
            .access_mode(0x0000_0080) // FILE_READ_ATTRIBUTES
            .share_mode(1 | 2 | 4)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
            .open(base.join("parent").join("owned"))
            .unwrap();
        remove_empty_directory_verified(
            &parent, Path::new("owned"), |m| (m.dev(), m.ino()) == owned, "replaced").unwrap();
        // exists() is also false for a delete-pending name (access denied), so
        // re-creating the name is the real check that it was removed at once.
        assert!(!base.join("parent").join("owned").exists());
        std::fs::create_dir(base.join("parent").join("owned"))
            .expect("the removed name must be free immediately, not delete-pending");
        drop(observer);
    }
}
