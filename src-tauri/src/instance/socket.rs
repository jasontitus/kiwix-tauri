// SPDX-License-Identifier: GPL-3.0-or-later
//! Both desktop listeners use the profile lock and a private runtime directory.
//! Capture a retired socket before removing it, and publish new sockets exclusively.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{
    Dir, DirBuilder, DirBuilderExt, FileTypeExt, MetadataExt, OpenOptions, OpenOptionsExt,
    Permissions, PermissionsExt,
};
use std::{
    io,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
};

fn socket_id(dir: &Dir, name: &Path) -> io::Result<(u64, u64)> {
    let metadata = dir.symlink_metadata(name)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != crate::fs_ext::effective_uid()
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "Another entry occupies the local socket path",
        ));
    }
    Ok((metadata.dev(), metadata.ino()))
}

pub(crate) struct SocketEntry {
    dir: Dir,
    name: PathBuf,
    id: (u64, u64),
}

fn remove_socket(dir: &Dir, name: &Path, id: (u64, u64)) -> io::Result<()> {
    if socket_id(dir, name)? != id {
        return Err(io::Error::other("Local socket changed before removal"));
    }
    let capture = PathBuf::from(format!(".retired-{}", uuid::Uuid::new_v4()));
    crate::publication::rename_entry_noreplace(dir, name, &capture)?;
    if socket_id(dir, &capture).ok() == Some(id) {
        return dir.remove_file(capture);
    }
    crate::publication::rename_entry_noreplace(dir, &capture, name).map_err(|err| {
        io::Error::other(format!(
            "Changed socket entry preserved at {}: {err}",
            capture.display()
        ))
    })?;
    Err(io::Error::other("Local socket changed during removal"))
}

impl Drop for SocketEntry {
    fn drop(&mut self) {
        if let Err(error) = remove_socket(&self.dir, &self.name, self.id) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("Kiwix: cannot retire {}: {error}", self.name.display());
            }
        }
    }
}

pub(crate) fn bind(path: &Path) -> io::Result<(UnixListener, SocketEntry)> {
    // The public path must also fit clients' sockaddr_un, even when the
    // kernel route used for publication below is shorter.
    socket2::SockAddr::unix(path)?;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing socket directory"))?;
    let parent = directory
        .parent()
        .ok_or_else(|| io::Error::other("Missing profile directory"))?;
    // The selected profile (or OS temporary root) may itself be a user-chosen alias.
    // Only its runtime child is application-owned.
    let parent = crate::storage::open_directory_nofollow(&parent.canonicalize()?)?;
    let child = directory
        .file_name()
        .ok_or_else(|| io::Error::other("Missing socket directory name"))?;
    match parent.create_dir_with(child, DirBuilder::new().mode(0o700)) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    let dir = parent.open_dir_nofollow(child)?;
    let metadata = dir.dir_metadata()?;
    if metadata.uid() != crate::fs_ext::effective_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Local socket directory belongs to another user",
        ));
    }
    if metadata.mode() & 0o777 != 0o700 {
        let readable = dir.open_with(
            ".",
            OpenOptions::new()
                .read(true)
                .follow(FollowSymlinks::No)
                .custom_flags(libc::O_DIRECTORY),
        )?;
        readable.set_permissions(Permissions::from_mode(0o700))?;
    }

    let name = PathBuf::from(
        path.file_name()
            .ok_or_else(|| io::Error::other("Missing socket name"))?,
    );
    // bind(2) and connect(2) have no dirfd argument. Use the kernel's path
    // to this retained directory, even when the public profile path moves.
    #[cfg(target_os = "macos")]
    let route = PathBuf::from(format!("/.vol/{}/{}", metadata.dev(), metadata.ino()));
    #[cfg(target_os = "linux")]
    let route = {
        use std::os::fd::AsRawFd;
        PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()))
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let route = directory.to_path_buf();
    let check_route = || {
        let named = std::fs::symlink_metadata(directory)?;
        use std::os::unix::fs::MetadataExt as _;
        if !named.is_dir() || named.dev() != metadata.dev() || named.ino() != metadata.ino() {
            return Err(io::Error::other(
                "Local socket directory moved during startup",
            ));
        }
        Ok(())
    };
    check_route()?;
    match socket_id(&dir, &name) {
        Ok(id) => {
            // Only ECONNREFUSED proves a stale listener. Timeouts, permission
            // errors and live peers must keep the existing entry.
            let probe = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
            probe.set_nonblocking(true)?;
            match probe.connect(&socket2::SockAddr::unix(route.join(&name))?) {
                Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "Another listener uses the local socket",
                    ))
                }
            }
            remove_socket(&dir, &name, id)?;
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let temp = PathBuf::from(format!(".starting-{}", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(route.join(&temp))?;
    let id = socket_id(&dir, &temp)?;
    let mut entry = SocketEntry {
        dir,
        name: temp,
        id,
    };
    #[cfg(target_os = "macos")]
    entry
        .dir
        .set_symlink_permissions(&entry.name, Permissions::from_mode(0o600))?;
    #[cfg(target_os = "linux")]
    {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt as _, PermissionsExt as _},
        };
        // Linux does not implement chmod on symlinks and older kernels have
        // no fchmodat2. O_PATH pins the socket inode without connecting.
        use rustix::fs::{openat, Mode, OFlags};
        let leaf = std::fs::File::from(openat(
            &entry.dir,
            &entry.name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let metadata = leaf.metadata()?;
        if (metadata.dev(), metadata.ino()) != entry.id {
            return Err(io::Error::other(
                "Local socket changed before setting permissions",
            ));
        }
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", leaf.as_raw_fd()),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    crate::publication::rename_entry_noreplace(&entry.dir, &entry.name, &name)?;
    entry.name = name;
    if socket_id(&entry.dir, &entry.name)? != entry.id {
        return Err(io::Error::other("Local socket changed during publication"));
    }
    check_route()?;
    Ok((listener, entry))
}
