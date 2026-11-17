// SPDX-License-Identifier: GPL-3.0-or-later
//! Matched desktop SDK linkage; stock libzim/Xapian cannot implement retained FD search.
use sha2::{Digest, Sha256};
use std::{env, fs, io::Read, path::{Component, Path, PathBuf}};

pub struct DesktopSdk {
    root: PathBuf,
}

/// Resolve a configured SDK root against the repository root. Build scripts run
/// with `src-tauri/` as their working directory, so an unanchored relative value
/// (the documented `.native/desktop-retained/<target>` form) would otherwise
/// resolve below `src-tauri/` and never match. Absolute values are kept as given.
pub fn resolve_root(manifest_dir: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        manifest_dir.parent().unwrap_or(manifest_dir).join(value)
    }
}

fn hash_file(path: &Path) -> String {
    let mut file = fs::File::open(path)
        .unwrap_or_else(|error| panic!("Cannot read SDK file {}: {error}", path.display()));
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).expect("Read native SDK file");
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    format!("{:x}", hash.finalize())
}

fn sdk_file(root: &Path, relative: &str) -> PathBuf {
    let path = Path::new(relative);
    assert!(!relative.is_empty() && path.components().all(|part| matches!(part, Component::Normal(_))),
        "SDK manifest path must be relative and cannot traverse: {relative}");
    let path = root.join(path).canonicalize()
        .unwrap_or_else(|error| panic!("Missing native SDK file {relative}: {error}"));
    assert!(path.starts_with(root) && path.is_file(), "SDK file escapes root or is not a file: {relative}");
    path
}

impl DesktopSdk {
    pub fn load() -> Self {
        let target = env::var("TARGET").expect("Cargo target is required");
        let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
        for key in ["KIWIX_DESKTOP_SDK_ROOT", "LIBZIM_SDK_ROOT", "LIBZIM_INCLUDE_DIR", "LIBZIM_LIB_DIR"] {
            println!("cargo:rerun-if-env-changed={key}");
        }
        let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
        let root = env::var_os("KIWIX_DESKTOP_SDK_ROOT")
            .or_else(|| env::var_os("LIBZIM_SDK_ROOT"))
            .map(PathBuf::from)
            .map(|value| resolve_root(&manifest_dir, &value))
            .unwrap_or_else(|| manifest_dir.join("../.native/desktop-retained").join(&target));
        let root = root.canonicalize().unwrap_or_else(|error| panic!(
            "Matched patched desktop SDK for {target} is required at {}: {error}. Run scripts/build-desktop-sdk.py on native Linux/macOS, or provision a schema-2 MSVC SDK; set KIWIX_DESKTOP_SDK_ROOT (relative values are resolved from the repository root). Stock pkg-config libzim is not supported.", root.display()));
        match (env::var_os("LIBZIM_INCLUDE_DIR"), env::var_os("LIBZIM_LIB_DIR")) {
            (Some(include), Some(lib)) => {
                assert_eq!(PathBuf::from(include).canonicalize().expect("SDK include override"), root.join("include").canonicalize().expect("SDK include directory"), "LIBZIM_INCLUDE_DIR must belong to the validated SDK");
                assert_eq!(PathBuf::from(lib).canonicalize().expect("SDK library override"), root.join("lib").canonicalize().expect("SDK library directory"), "LIBZIM_LIB_DIR must belong to the validated SDK");
            }
            (None, None) => {}
            _ => panic!("Set both LIBZIM_INCLUDE_DIR and LIBZIM_LIB_DIR, or neither; KIWIX_DESKTOP_SDK_ROOT always identifies the verified SDK"),
        }
        let manifest = root.join("kiwix-sdk.json");
        println!("cargo:rerun-if-changed={}", manifest.display());
        let metadata: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)
            .unwrap_or_else(|error| panic!("Required desktop SDK manifest {}: {error}", manifest.display())))
            .expect("Valid desktop SDK manifest JSON");
        if let Some(version) = metadata["version"].as_str() {
            println!("cargo:rustc-env=KIWIX_LIBZIM_VERSION={version}");
        }
        if let Some(version) = metadata["xapian"]["version"].as_str() {
            println!("cargo:rustc-env=KIWIX_XAPIAN_VERSION={version}");
        }
        assert_eq!(metadata["schema"].as_u64(), Some(2), "Desktop SDK schema 2 with current patch provenance is required");
        assert_eq!(metadata["target"].as_str(), Some(target.as_str()), "Desktop SDK target mismatch");
        assert_eq!(metadata["version"].as_str(), Some("9.8.2"), "Desktop SDK libzim version mismatch");
        assert_eq!(metadata["libzim"]["commit"].as_str(), Some("26ec526f74e8342a40da5ab14e364546988e0e1a"), "Desktop SDK libzim source pin mismatch");
        assert_eq!(metadata["libzim"]["search"].as_bool(), Some(true), "Desktop SDK must implement full-text search");
        let files = metadata["files"].as_object().expect("Desktop SDK SHA256 file inventory is required");
        assert!(!files.is_empty(), "Desktop SDK file inventory cannot be empty");
        for (relative, expected) in files {
            let path = sdk_file(&root, relative);
            println!("cargo:rerun-if-changed={}", root.join(relative).display());
            let expected = expected.as_str().expect("SDK file SHA256 string");
            assert_eq!(hash_file(&path), expected, "Desktop SDK checksum mismatch: {relative}");
        }
        let required = [
            ("kiwix-split-ranges-v1", "zim", "libzim-split-ranges.patch"),
            ("kiwix-retained-fd-v1", "zim", "libzim-retained-fd.patch"),
            ("kiwix-xapian-fd-offset-v1", "xapian", "xapian-fd-offset.patch"),
        ];
        let records = metadata["sourcePatches"].as_array().expect("Desktop SDK source patch provenance is required");
        // Windows retains native path/index access, not the POSIX FD factory.
        // Verify the offset patch too if the external Windows SDK declares it.
        let requires_offset = !windows || records.iter().any(|record| record["marker"].as_str() == Some(required[2].0));
        let required_count = if requires_offset { 3 } else { 2 };
        assert_eq!(records.len(), required_count, "Unexpected desktop SDK source patch set");
        let markers = metadata["patches"].as_array().expect("Desktop SDK patch markers are required");
        assert_eq!(markers.len(), required_count, "Unexpected desktop SDK patch marker set");
        for (marker, component, filename) in required.into_iter().take(required_count) {
            assert_eq!(markers.iter().filter(|value| value.as_str() == Some(marker)).count(), 1,
                "SDK must declare patch marker {marker} exactly once");
            let mut matching = records.iter().filter(|record| record["marker"].as_str() == Some(marker));
            let record = matching.next().unwrap_or_else(|| panic!("Missing desktop SDK patch {marker}"));
            assert!(matching.next().is_none(), "Duplicate desktop SDK patch {marker}");
            assert_eq!(record["component"].as_str(), Some(component), "SDK patch component mismatch");
            let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
                .join("native").join(filename);
            println!("cargo:rerun-if-changed={}", source.display());
            let expected = hash_file(&source);
            assert_eq!(record["sha256"].as_str(), Some(expected.as_str()), "Desktop SDK patch is stale: {marker}");
            let relative = format!("share/provenance/{filename}");
            assert_eq!(record["path"].as_str(), Some(relative.as_str()), "Unexpected SDK patch provenance path");
            assert_eq!(files.get(&relative).and_then(|value| value.as_str()), Some(expected.as_str()),
                "SDK inventory must cover installed patch provenance: {marker}");
        }
        for relative in ["include/zim/archive.h", "include/zim/search.h"] {
            assert!(files.contains_key(relative), "SDK manifest must cover {relative}");
        }
        let library = if windows { "lib/zim.lib" } else if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            "lib/libzim.dylib"
        } else { "lib/libzim.so" };
        assert!(files.contains_key(library), "SDK manifest must cover {library}");
        if !windows {
            assert_eq!(metadata["xapian"]["version"].as_str(), Some("1.4.32"), "Desktop SDK needs matched Xapian 1.4.32");
            assert_eq!(metadata["xapian"]["linkage"].as_str(), Some("shared"), "Desktop Xapian must be dynamic");
            let xapian = if library.ends_with("dylib") { "lib/libxapian.dylib" } else { "lib/libxapian.so" };
            assert!(files.contains_key(xapian), "SDK manifest must cover matched dynamic Xapian");
        } else {
            let bin = root.join("bin");
            let mut count = 0;
            for entry in fs::read_dir(&bin).expect("Windows SDK runtime DLL directory") {
                let entry = entry.expect("Windows SDK runtime entry");
                if entry.path().extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("dll")) {
                    let relative = format!("bin/{}", entry.file_name().to_str().expect("SDK DLL filename"));
                    assert!(files.contains_key(&relative), "Unverified Windows runtime DLL: {relative}");
                    count += 1;
                }
            }
            assert!(count > 0, "Windows SDK must provide the runtime DLL closure");
        }
        Self { root }
    }

    pub fn configure(&self, build: &mut cc::Build) {
        build.include(tool_path(&self.root.join("include")));
    }

    pub fn emit_links(&self) {
        println!("cargo:rustc-link-search=native={}", tool_path(&self.root.join("lib")).display());
        println!("cargo:rustc-link-lib=dylib=zim");
        if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            // Keep development executables bound to this exact matched SDK as well.
            // Release bundlers subsequently rewrite/copy the dynamic dependency graph.
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", self.root.join("lib").display());
        }
    }
}

/// `canonicalize` returns verbatim `\\?\` paths on Windows, which MSVC `cl.exe`
/// cannot use for `-I` (C1083) and which are not portable linker search paths.
/// Hand tools the equivalent ordinary form; verification above keeps using
/// the canonical root so containment checks compare like with like.
pub fn tool_path(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else { return path.to_path_buf() };
    // Without the prefix Win32 normalization applies: a component ending in
    // '.' or ' ' would be trimmed, and long paths may not open at all, so the
    // tool could see a different directory than the one verified above.
    let win32_safe = |rest: &str| {
        rest.len() < 260 && rest.split('\\').all(|component| !component.ends_with(['.', ' ']))
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        if win32_safe(rest) {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        return path.to_path_buf();
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
            && win32_safe(rest)
        {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_windows_paths_become_tool_paths() {
        assert_eq!(tool_path(Path::new(r"\\?\E:\sdk\include")), Path::new(r"E:\sdk\include"));
        assert_eq!(tool_path(Path::new(r"\\?\UNC\server\share\sdk")), Path::new(r"\\server\share\sdk"));
        // Other verbatim forms and ordinary paths are left unchanged.
        assert_eq!(tool_path(Path::new(r"\\?\Volume{1234}\sdk")), Path::new(r"\\?\Volume{1234}\sdk"));
        assert_eq!(tool_path(Path::new(r"\\?\E:")), Path::new(r"\\?\E:"));
        // Win32 would trim these components or reject the length: keep verbatim.
        assert_eq!(tool_path(Path::new(r"\\?\E:\sdk.\include")), Path::new(r"\\?\E:\sdk.\include"));
        assert_eq!(tool_path(Path::new(r"\\?\E:\sdk \include")), Path::new(r"\\?\E:\sdk \include"));
        let long = format!(r"\\?\E:\{}\include", "a".repeat(300));
        assert_eq!(tool_path(Path::new(&long)), Path::new(&long));
        assert_eq!(tool_path(Path::new("/opt/kiwix-sdk/include")), Path::new("/opt/kiwix-sdk/include"));
    }

    #[test]
    fn relative_sdk_root_is_anchored_at_the_repository_root() {
        let manifest = Path::new("/repository/src-tauri");
        assert_eq!(
            resolve_root(manifest, Path::new(".native/desktop-retained/aarch64-apple-darwin")),
            Path::new("/repository/.native/desktop-retained/aarch64-apple-darwin"),
            "a repo-relative SDK root must resolve from the repository root, not src-tauri"
        );
        assert_eq!(
            resolve_root(manifest, Path::new(".native/desktop-retained-v2/aarch64-apple-darwin")),
            Path::new("/repository/.native/desktop-retained-v2/aarch64-apple-darwin")
        );
    }

    #[test]
    fn absolute_sdk_root_is_kept_verbatim() {
        let manifest = Path::new("/repository/src-tauri");
        let absolute = if cfg!(windows) { Path::new(r"C:\kiwix-sdk") } else { Path::new("/opt/kiwix-sdk") };
        assert_eq!(resolve_root(manifest, absolute), absolute);
    }
}
