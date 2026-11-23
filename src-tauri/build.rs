// SPDX-License-Identifier: GPL-3.0-or-later
use std::env;
fn main() {
    emit_archive_versions();
    emit_build_identity();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        // Oboe selects the static C++ runtime. The NDK's API-level libc++.a
        // linker script supplies both libc++_static and its exception/RTTI ABI.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-l:libc++.a");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,defs");
    }
    // Platforms with an app-owned location provider (see permissions.rs). An
    // archive asks Kiwix, never the webview's own geolocation.
    println!("cargo::rustc-check-cfg=cfg(native_location)");
    if matches!(
        env::var("CARGO_CFG_TARGET_OS").as_deref(),
        // Linux (GeoClue) and Android providers are written but not yet
        // compiled; add "linux" | "android" here once they build and pass review.
        Ok("macos" | "ios" | "windows")
    ) {
        println!("cargo:rustc-cfg=native_location");
    }
    // The authenticated headless preview embeds this directory independently of
    // Tauri's platform-specific frontendDist. Rebuild it when those assets change.
    println!("cargo:rerun-if-changed=../dist");
    println!("cargo:rerun-if-env-changed=KIWIX_TYPECHECK_ONLY");

    // Cross-target checks skip platform shims. Force an error if a check-only
    // build is ever linked as an executable or shared library.
    if env::var("KIWIX_TYPECHECK_ONLY").as_deref() == Ok("1") {
        println!("cargo:rustc-link-lib=dylib=kiwix_typecheck_only_cannot_link");
        tauri_build_with_windows_manifest();
        return;
    }

    if matches!(env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("macos" | "ios")) {
        println!("cargo:rerun-if-changed=native/location.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/location.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_location");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreLocation");
        println!("cargo:rerun-if-changed=native/notifications.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/notifications.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_notifications");
        println!("cargo:rustc-link-lib=framework=UserNotifications");
        let application_framework = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") { "UIKit" } else { "AppKit" };
        println!("cargo:rustc-link-lib=framework={application_framework}");
        println!("cargo:rerun-if-changed=native/localswarm.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/localswarm.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_localswarm");
        println!("cargo:rustc-link-lib=framework=Network");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rerun-if-changed=native/pdf.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/pdf.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_pdf");
        println!("cargo:rustc-link-lib=framework=WebKit");
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            // The real OS print panel is macOS-only. iOS shares the renderer
            // but has no print button upstream (the share sheet is the only
            // system handoff), so this file is not compiled for iOS.
            println!("cargo:rerun-if-changed=native/print.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/print.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_print");
            println!("cargo:rustc-link-lib=framework=PDFKit");
        }
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rustc-link-lib=framework=ApplicationServices");
            println!("cargo:rerun-if-changed=native/share.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/share.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_share");
            println!("cargo:rerun-if-changed=native/tabbing.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/tabbing.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_tabbing");
        }
    }

    tauri_build_with_windows_manifest();
}

/// Expose the source revision in Settings so a test build is identifiable.
fn emit_build_identity() {
    let from_env = env::var("KIWIX_BUILD_COMMIT").ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR");
    let commit = from_env.or_else(|| {
        let manifest_dir = manifest_dir.as_ref()?;
        let root = std::path::Path::new(manifest_dir).parent()?.to_path_buf();
        // Do not bind a source archive nested inside an unrelated checkout to
        // that enclosing repository's HEAD. A real checkout always has .git
        // (a directory, or a file for a linked worktree).
        if !root.join(".git").exists() {
            return None;
        }
        let output = std::process::Command::new("git")
            .args(["-C", root.to_str()?, "rev-parse", "--short=12", "HEAD"])
            .output().ok()?;
        if !output.status.success() { return None; }
        let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=KIWIX_BUILD_COMMIT={commit}");
    println!("cargo:rerun-if-env-changed=KIWIX_BUILD_COMMIT");
    if let Some(manifest_dir) = manifest_dir {
        if let Some(root) = std::path::Path::new(&manifest_dir)
            .parent()
            .filter(|root| root.join(".git").exists())
        {
            let git_path = |name: &str| {
                std::process::Command::new("git")
                    .args([
                        "-C",
                        root.to_string_lossy().as_ref(),
                        "rev-parse",
                        "--path-format=absolute",
                        "--git-path",
                        name,
                    ])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                    .filter(|path| !path.is_empty())
            };
            if let (Some(head), Some(refs)) = (git_path("HEAD"), git_path("refs")) {
                println!("cargo:rerun-if-changed={head}");
                println!("cargo:rerun-if-changed={refs}");
            }
        }
    }
}

/// Report the two reader dependencies selected by the application's lockfile.
fn emit_archive_versions() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock: toml::Table = include_str!("Cargo.lock").parse().expect("invalid Cargo.lock");
    let packages = lock["package"].as_array().expect("missing Cargo packages");
    for (name, variable) in [("zimru", "KIWIX_ZIMRU"), ("zimfile-xapian", "KIWIX_GLASS")] {
        let package = packages.iter().find(|pkg| pkg["name"].as_str() == Some(name))
            .expect("missing locked archive dependency");
        let version = package["version"].as_str().expect("missing archive dependency version");
        let revision = package.get("source").and_then(|v| v.as_str())
            .and_then(|source| source.rsplit_once('#')).map(|(_, rev)| rev)
            .filter(|rev| rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()));
        match revision {
            Some(rev) => {
                println!("cargo:rustc-env={variable}={version} ({})", &rev[..12]);
                println!("cargo:rustc-env={variable}_REVISION={rev}");
            }
            None => println!("cargo:rustc-env={variable}={version}"),
        }
    }
}

/// tauri-build embeds its Common-Controls v6 manifest only into the app
/// binary. `cargo test` binaries import comctl32 v6 APIs too (the dialog
/// plugin's `TaskDialogIndirect`) and fail to start with
/// STATUS_ENTRYPOINT_NOT_FOUND without it. On MSVC, link the identical
/// manifest (a verbatim copy of tauri-build's default) into every binary,
/// tests included, and stop tauri-build from embedding a second copy.
/// link.exe merges in its default UAC fragment (`requestedExecutionLevel
/// asInvoker`), so binaries now also declare that explicitly; this only
/// disables installer-detection heuristics and changes no privilege.
fn tauri_build_with_windows_manifest() {
    let msvc_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc_windows {
        tauri_build::build();
        return;
    }
    let manifest = std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
        .join("windows-app-manifest.xml");
    assert!(manifest.is_file(), "Missing {}", manifest.display());
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("tauri-build failed");
}
