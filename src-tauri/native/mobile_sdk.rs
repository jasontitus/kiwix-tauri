// SPDX-License-Identifier: GPL-3.0-or-later
//! Target-specific static SDK linkage. A host SDK must never satisfy a mobile build.
use sha2::{Digest, Sha256};
use std::{env, fs, io::Read, path::{Path, PathBuf}};

pub struct MobileSdk {
    root: PathBuf,
    metadata: serde_json::Value,
}

impl MobileSdk {
    pub fn load() -> Option<Self> {
        let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo target OS is required");
        if !matches!(target_os.as_str(), "ios" | "android") {
            return None;
        }
        println!("cargo:rerun-if-env-changed=TAURI_KIWIX_MOBILE_SDK_ROOT");
        let target = env::var("TARGET").expect("Cargo target is required");
        let root = env::var_os("TAURI_KIWIX_MOBILE_SDK_ROOT").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
                .join("../.native/mobile").join(&target)
        });
        let manifest = root.join("kiwix-sdk.json");
        println!("cargo:rerun-if-changed={}", manifest.display());
        let bytes = fs::read(&manifest).unwrap_or_else(|error| panic!(
            "Patched native SDK for {target} is required at {}: {error}. Run scripts/build-ios-sdk.py or scripts/build-android-sdk.py; never substitute a macOS SDK.", manifest.display()
        ));
        let metadata: serde_json::Value = serde_json::from_slice(&bytes).expect("Valid mobile SDK manifest JSON");
        if let Some(version) = metadata["version"].as_str() {
            println!("cargo:rustc-env=KIWIX_LIBZIM_VERSION={version}");
        }
        if let Some(version) = metadata["xapian"]["version"].as_str() {
            println!("cargo:rustc-env=KIWIX_XAPIAN_VERSION={version}");
        }
        assert_eq!(metadata["schema"].as_u64(), Some(2), "Mobile SDK schema 2 with descriptor-preserving source patches is required");
        assert_eq!(metadata["target"].as_str(), Some(target.as_str()), "Mobile SDK target mismatch");
        assert_eq!(metadata["libzim"]["commit"].as_str(), Some("26ec526f74e8342a40da5ab14e364546988e0e1a"), "Mobile SDK libzim pin mismatch");
        if target == "armv7-linux-androideabi" {
            assert_eq!(metadata["androidAbi"].as_str(), Some("armeabi-v7a"), "ARMv7 SDK ABI mismatch");
            assert_eq!(metadata["api"].as_u64(), Some(24), "ARMv7 SDK must retain the application's Android API 24 floor");
        }
        let required = [
            ("kiwix-split-ranges-v1", "libzim-split-ranges.patch"),
            ("kiwix-retained-fd-v1", "libzim-retained-fd.patch"),
            ("kiwix-xapian-fd-offset-v1", "xapian-fd-offset.patch"),
        ];
        let platform_patch = (target == "armv7-linux-androideabi")
            .then_some(("kiwix-lp32-raw-blob-v1", "libzim-lp32-raw-blob.patch"));
        let android_patch = (target_os == "android")
            .then_some(("kiwix-zstd-cache-accounting-v1", "libzim-zstd-cache-accounting.patch"));
        let retirement_patch = (target_os == "android")
            .then_some(("kiwix-decoded-cluster-retirement-v1", "libzim-decoded-cluster-retirement.patch"));
        let records = metadata["sourcePatches"].as_array().expect("Mobile SDK source patch provenance is required");
        assert_eq!(records.len(), required.len() + usize::from(platform_patch.is_some()) + usize::from(android_patch.is_some()) + usize::from(retirement_patch.is_some()), "Unexpected mobile SDK source patch set");
        for (marker, filename) in required.into_iter().chain(platform_patch).chain(android_patch).chain(retirement_patch) {
            let source = PathBuf::from("native").join(filename);
            println!("cargo:rerun-if-changed={}", source.display());
            assert!(metadata["patches"].as_array().is_some_and(|patches|
                patches.iter().any(|patch| patch.as_str() == Some(marker))),
                "Mobile SDK lacks required patch {marker}");
            let mut matching = records.iter().filter(|record| record["marker"].as_str() == Some(marker));
            let record = matching.next().unwrap_or_else(|| panic!("SDK lacks patch provenance: {marker}"));
            assert!(matching.next().is_none(), "SDK must declare patch {marker} exactly once");
            let hash = format!("{:x}", Sha256::digest(fs::read(&source).expect("Bundled native correction")));
            assert_eq!(record["sha256"].as_str(), Some(hash.as_str()), "Mobile SDK patch mismatch: {marker}");
            let relative = format!("share/provenance/{filename}");
            assert_eq!(record["path"].as_str(), Some(relative.as_str()), "Unexpected SDK patch provenance path");
            let installed = root.join(relative);
            println!("cargo:rerun-if-changed={}", installed.display());
            assert_eq!(format!("{:x}", Sha256::digest(fs::read(installed).expect("Installed SDK patch provenance"))),
                hash, "Installed SDK patch bytes differ: {marker}");
        }
        let sdk = Self { root, metadata };
        let mut libraries = sdk.strings("staticLibraries").peekable();
        assert!(libraries.peek().is_some(), "Mobile SDK must include static libraries");
        for library in libraries {
            assert!(!library.is_empty() && library.bytes().all(|b| b.is_ascii_alphanumeric() || b"_+.-".contains(&b)), "Invalid SDK library name");
            let relative = format!("lib/lib{library}.a");
            let archive = sdk.root.join(&relative);
            let hashes = &sdk.metadata[if target_os == "android" { "files" } else { "archives" }];
            let expected = hashes[&relative].as_str().unwrap_or_else(|| panic!("SDK lacks archive digest: {relative}"));
            assert_eq!(file_sha256(&archive), expected, "Installed SDK archive differs from provenance: {relative}");
            if target == "armv7-linux-androideabi" {
                let objects = &sdk.metadata["archiveObjects"][&relative];
                assert_eq!(objects["class"].as_str(), Some("ELF32"), "SDK archive must contain 32-bit objects: {relative}");
                assert_eq!(objects["machine"].as_str(), Some("ARM"), "SDK archive must contain ARM objects: {relative}");
                assert!(objects["members"].as_u64().is_some_and(|count| count > 0), "SDK archive object provenance is required: {relative}");
            }
            println!("cargo:rerun-if-changed={}", archive.display());
        }
        assert!(sdk.root.join("include/zim/archive.h").is_file(), "Missing target libzim headers");
        Some(sdk)
    }

    pub fn configure(&self, build: &mut cc::Build) {
        build.include(self.root.join("include")).cpp_link_stdlib(None);
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
            // Match the SDK's public off_t ABI on 32-bit Android as well as LP64.
            build.define("_FILE_OFFSET_BITS", "64").define("_LARGEFILE_SOURCE", "1");
            if env::var("TARGET").as_deref() == Ok("armv7-linux-androideabi") {
                build.flag("-march=armv7-a").flag("-mthumb").flag("-mfpu=neon").flag("-mfloat-abi=softfp");
            }
        }
    }

    pub fn emit_links(&self) {
        println!("cargo:rustc-link-search=native={}", self.root.join("lib").display());
        for library in self.strings("staticLibraries") {
            println!("cargo:rustc-link-lib=static={library}");
        }
        for library in self.strings("systemLibraries") {
            println!("cargo:rustc-link-lib={library}");
        }
        for framework in self.strings("frameworks") {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        for argument in self.strings("linkerFlags") {
            println!("cargo:rustc-link-arg={argument}");
        }
    }

    fn strings(&self, key: &'static str) -> impl Iterator<Item = &str> {
        self.metadata[key].as_array().unwrap_or_else(|| panic!("Missing SDK {key} array"))
            .iter().map(move |value| value.as_str().unwrap_or_else(|| panic!("Invalid SDK {key} entry")))
    }
}

fn file_sha256(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap_or_else(|error| panic!("Opening SDK archive {}: {error}", path.display()));
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap_or_else(|error| panic!("Reading SDK archive {}: {error}", path.display()));
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    format!("{:x}", hash.finalize())
}
